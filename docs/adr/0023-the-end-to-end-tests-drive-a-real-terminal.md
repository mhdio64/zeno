# The end-to-end tests drive a real terminal

Spec §9 asks for three layers of test, and the third — "end-to-end tests
driving a real PTY" — is the only one that can answer the questions no unit
test owns: does the alternate screen come up, does a key arrive as the
terminal really encodes it, does what the editor paints survive the trip
through a pty and a screen. Wrapping, the panels and `zeno -` are all things
you can only see by looking.

The decision is a harness that spawns the built editor under a pseudo
terminal, writes keystrokes to it as bytes, and renders the bytes it writes
back through a terminal emulator — `pyte` — so a test asserts on *screen
text*: `self.assertIn("hello", screen.text())`. It lives in `tests/e2e/`
alongside the tests that use it, and CI runs the suite on Linux, macOS and
Windows, because the input path is different on each (spec §3).

Two things had to be learned before it could work:

- **The child must own the pty as its controlling terminal.** crossterm sizes
  itself from `/dev/tty`, which is the *controlling* terminal, not the file
  descriptor it was handed. Without `setsid` + `TIOCSCTTY` the editor inherits
  the runner's console — 30 rows of a CI shell — and paints a screen two rows
  taller than the emulator watching it, which shows up as every line shifted
  by one and a status bar wrapped over two. The harness opens the session, so
  the editor draws the screen the test is reading.
- **A pty's byte stream cannot carry every key.** On Windows, ConPTY's input
  parser drops the NUL byte a legacy terminal sends for `Ctrl+Space`, and it
  does not put Alt back on a letter that follows an escape, so `Leader …` and
  `Alt+Z` cannot be reached by writing bytes to the pty at all. The tests
  that need them say so and skip on Windows; everything a real user reaches —
  plain keys, `Ctrl` chords, `F1` — round-trips fine, and a user at a console
  is unaffected, because their keys arrive as key events with their modifier
  state already on them (the same reality ADR 0010 builds on). On POSIX both
  encodings work, and the whole suite runs.

The cost is that these tests are the slow ones — each waits for a frame after
every keystroke, so the suite is measured in seconds rather than
milliseconds — and that they are timing-sensitive by construction. They are
kept few and blunt on purpose: each one covers a path a user takes (the file
appears, typing edits, `Ctrl+S` persists, wrap unfolds a long line, help
opens, quitting exits zero), and the fine-grained behaviour of those paths
stays in the unit tests where it can be asserted exactly.
