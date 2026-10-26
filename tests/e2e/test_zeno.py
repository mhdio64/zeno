"""End-to-end tests: a real editor in a real terminal.

Every test here opens a file, drives the editor with keystrokes, and asserts
on what a user would see. They are the only tests that can catch a break in
the parts no unit test owns — raw mode, the alternate screen, the key
encoding a terminal really sends — which is what the features added around
wrapping, panels and `zeno -` depend on.

Run them with:

    cargo build --release
    python -m unittest discover -s tests/e2e -v
"""

import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from harness import WINDOWS, Terminal, binary, leader  # noqa: E402


class EditorTest(unittest.TestCase):
    """A test that owns one temporary directory of files."""

    def setUp(self) -> None:
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)

    def write(self, name: str, text: str) -> str:
        path = os.path.join(self.dir.name, name)
        with open(path, "w", encoding="utf-8", newline="") as handle:
            handle.write(text)
        return path

    def read(self, path: str) -> str:
        with open(path, encoding="utf-8", newline="") as handle:
            return handle.read()


class TheEditorStarts(EditorTest):
    def test_the_binary_under_test_exists(self) -> None:
        self.assertTrue(
            os.path.exists(binary()),
            f"no editor at {binary()}: build it, or set ZENO_BIN",
        )

    def test_the_file_is_on_screen_with_a_gutter(self) -> None:
        path = self.write("hello.txt", "hello world\nsecond line\n")
        with Terminal(path) as screen:
            self.assertTrue(screen.wait_for("hello world"), screen.text())
            self.assertTrue(screen.wait_for("second line"), screen.text())
            first = screen.row(0)
            self.assertIn("1", first, "the gutter numbers the first line")
            self.assertLess(
                first.index("1"),
                first.index("hello"),
                "the number comes before the text",
            )

    def test_a_shebang_file_is_named_in_the_status_line(self) -> None:
        path = self.write("script", "#!/bin/sh\necho hi\n")
        with Terminal(path) as screen:
            self.assertTrue(screen.wait_for("Bourne"), screen.text())


class EditingAndSaving(EditorTest):
    def test_typing_edits_the_buffer(self) -> None:
        path = self.write("type.txt", "abc\n")
        with Terminal(path) as screen:
            self.assertTrue(screen.wait_for("abc"), screen.text())
            screen.send("X")
            self.assertTrue(screen.wait_for("Xabc"), screen.text())

    def test_ctrl_q_quits(self) -> None:
        path = self.write("quit.txt", "abc\n")
        with Terminal(path) as screen:
            self.assertTrue(screen.wait_for("abc"), screen.text())
            screen.send("\x11")  # Ctrl+Q
            self.assertTrue(screen.wait_until_gone(), "the editor exited")
        self.assertEqual(screen.exit_code(), 0)

    def test_ctrl_s_writes_the_file(self) -> None:
        path = self.write("save.txt", "abc\n")
        with Terminal(path) as screen:
            self.assertTrue(screen.wait_for("abc"), screen.text())
            screen.send("X")
            self.assertTrue(screen.wait_for("Xabc"), screen.text())
            # Ctrl+S. Kept alive afterwards so the exit does not race the
            # write: the editor saves on the key, not on the way out.
            screen.send("\x13")
        self.assertEqual(self.read(path), "Xabc\n")

    def test_a_run_of_typing_undoes_as_one_step(self) -> None:
        path = self.write("undo.txt", "abc\n")
        with Terminal(path) as screen:
            self.assertTrue(screen.wait_for("abc"), screen.text())
            screen.type("XYZ")
            self.assertTrue(screen.wait_for("XYZabc"), screen.text())
            screen.send("\x1a")  # Ctrl+Z
            self.assertTrue(
                screen.wait_for("abc"),
                screen.text(),
            )
            self.assertFalse(screen.contains("XYZ"), screen.text())


class TheGutterAndWrap(EditorTest):
    # The leader is `Ctrl+Space`, which a legacy terminal sends as a NUL byte,
    # and `Alt+Z` arrives as Escape plus a letter. Windows' ConPTY input parser
    # drops the NUL byte and does not put Alt back on a key that follows an
    # escape, so a test that drives the editor by writing bytes to a pty cannot
    # reach either key there — a real user at a console can, because their keys
    # arrive as key events with their modifiers already on. The tests below run
    # where the bytes are enough.
    @unittest.skipIf(
        WINDOWS,
        "the pty's byte stream cannot carry NUL or Alt through Windows' ConPTY",
    )
    def test_leader_comma_hides_the_numbers(self) -> None:
        path = self.write("gutter.txt", "one\ntwo\n")
        with Terminal(path) as screen:
            self.assertTrue(screen.wait_for("one"), screen.text())
            numbered = screen.row(0)
            self.assertIn("1", numbered)
            screen.send(leader + ",")
            self.assertTrue(screen.wait_for("one"), screen.text())
            plain = screen.row(0)
            self.assertNotIn("1", plain, f"the gutter is gone: {plain!r}")
            screen.send(leader + ",")
            self.assertTrue(screen.wait_for(numbered.strip()), screen.text())

    @unittest.skipIf(
        WINDOWS,
        "the pty's byte stream cannot carry Alt through Windows' ConPTY",
    )
    def test_alt_z_wraps_a_long_line_onto_more_rows(self) -> None:
        words = " ".join(f"word{index}" for index in range(60))
        path = self.write("wrap.txt", words + "\n")
        with Terminal(path, cols=60) as screen:
            self.assertTrue(screen.wait_for("word0"), screen.text())
            self.assertNotIn("word59", screen.text(), "the line is cut off")
            screen.send("\x1bz")  # Alt+Z
            self.assertTrue(
                screen.wait_for("word59"),
                "every word is on screen once the line is wrapped",
            )
            # The wrapped row carries no number of its own: it is the same
            # line, and the gutter says where lines start.
            rows = [screen.row(index) for index in range(4)]
            numbered = [row for row in rows if row.strip() and row.split()[0].isdigit()]
            self.assertLessEqual(
                len(numbered), 2, f"one line, one number: {rows!r}"
            )


class ThePanels(EditorTest):
    @unittest.skipIf(
        WINDOWS,
        "the pty's byte stream cannot carry NUL through Windows' ConPTY",
    )
    def test_leader_question_mark_lists_the_bindings(self) -> None:
        path = self.write("help.txt", "abc\n")
        with Terminal(path) as screen:
            self.assertTrue(screen.wait_for("abc"), screen.text())
            screen.send(leader + "?")
            self.assertTrue(screen.wait_for("Save"), screen.text())
            self.assertTrue(screen.wait_for("Quit without saving"), screen.text())
            screen.send("\x1b")  # Esc
            self.assertTrue(screen.wait_for("abc"), screen.text())

    def test_f1_opens_the_palette_and_esc_closes_it(self) -> None:
        path = self.write("palette.txt", "abc\n")
        with Terminal(path) as screen:
            self.assertTrue(screen.wait_for("abc"), screen.text())
            screen.send("\x1bOP")  # F1
            self.assertTrue(screen.wait_for("ctrl+s"), screen.text())
            screen.send("\x1b")  # Esc
            self.assertTrue(screen.wait_for("abc"), screen.text())


class ThePrompt(EditorTest):
    def test_find_reaches_a_line_further_down(self) -> None:
        lines = [f"line {index}" for index in range(1, 200)]
        path = self.write("find.txt", "\n".join(lines) + "\n")
        with Terminal(path) as screen:
            self.assertTrue(screen.wait_for("line 1"), screen.text())
            screen.send("\x06")  # Ctrl+F
            screen.type("line 150")
            screen.send("\r")  # Enter jumps to match and keeps prompt open
            self.assertTrue(
                screen.wait_for("150 line 150"),
                "the viewport jumped to line 150 with its gutter number",
            )
            # The prompt line is still open, replacing the status line
            self.assertFalse(screen.contains("Ln 150"), "prompt owns status row")
            screen.send("\x1b")  # First Esc closes prompt, restoring status line with search active
            self.assertTrue(screen.wait_for("Ln 150"), "status line restored")
            self.assertTrue(screen.contains("Find: line 150"), "highlights remain active")
            screen.send("\x1b")  # Second Esc clears active search
            self.assertTrue(screen.wait_for("find.txt"), screen.text())
            self.assertFalse(screen.contains("Find: line 150"), "search cleared")


@unittest.skipIf(
    WINDOWS,
    "a pipe on Windows is exercised by tools/tui_probe.py; the console's "
    "end-of-file key cannot be sent through ConPTY",
)
class ThePipe(EditorTest):
    def test_a_pipe_is_edited_and_printed_back(self) -> None:
        # The document arrives on the terminal itself, because a terminal is
        # what the editor needs for keys; Ctrl+D ends the input (ADR 0020).
        with Terminal("-") as screen:
            screen.send("abc\n")
            screen.send("\x04")
            self.assertTrue(screen.wait_for("abc"), screen.text())
            screen.send("X")
            self.assertTrue(screen.wait_for("Xabc"), screen.text())
            # `Leader Q`, because the buffer is edited and quitting with
            # changes in it asks first.
            screen.send(leader + "Q")
            self.assertTrue(screen.wait_until_gone(), "the editor exited")
            self.assertEqual(screen.exit_code(), 0, "the buffer is printed")
            self.assertTrue(screen.contains("Xabc"), screen.text())


if __name__ == "__main__":
    unittest.main()
