"""Drive the zeno editor in a real terminal, and read what it paints.

The editor's whole job is what ends up on the screen, so the only test that
can be believed is one that runs it under a pseudo-terminal and reads the
screen back through a terminal emulator. This module is the smallest thing
that does that on all three platforms:

* Windows spawns under ConPTY through `winpty`.
* Everything else opens a pty and hands the child its slave end.

The screen is emulated by `pyte`, so a test asks what text is visible and
where, not which escape codes were written.
"""

import os
import platform
import struct
import subprocess
import threading
import time

import pyte

ROWS = 24
COLS = 90

#: How long to wait after a keystroke for the editor to paint it. Generous
#: on purpose: a slow CI machine failing a test over its own latency would
#: teach us nothing.
DELAY = float(os.environ.get("ZENO_E2E_DELAY", "0.18"))

WINDOWS = platform.system() == "Windows"


def own_the_tty() -> None:
    """Makes the child's controlling terminal the one the test opened.

    Without this the child inherits the test process's controlling terminal —
    a CI runner's console, or none at all — and a program that sizes itself
    from `/dev/tty` (crossterm does) would draw for a screen nobody is
    looking at. New session, then claim the pty as the console.
    """
    import fcntl
    import termios

    os.setsid()
    fcntl.ioctl(0, termios.TIOCSCTTY, 0)


def binary() -> str:
    """The editor under test: `ZENO_BIN`, or the release build in `target/`."""
    given = os.environ.get("ZENO_BIN")
    if given:
        return given
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    name = "zeno.exe" if WINDOWS else "zeno"
    return os.path.join(root, "target", "release", name)


class Terminal:
    """A running editor, a screen, and the keys sent to it."""

    def __init__(self, *args: str, rows: int = ROWS, cols: int = COLS):
        self.rows = rows
        self.cols = cols
        self.screen = pyte.Screen(cols, rows)
        self.stream = pyte.ByteStream(self.screen)
        self._closed = False
        self._pump_thread = None

        argv = [binary(), *args]
        env = dict(os.environ)
        # A terminal that says what it is: the editor asks for a colour-
        # capable terminal, and an unset TERM makes some parsers guess.
        env.setdefault("TERM", "xterm-256color")

        if WINDOWS:
            from winpty import PtyProcess

            self.proc = PtyProcess.spawn(argv, dimensions=(rows, cols), env=env)
            self._write = self.proc.write
        else:
            import fcntl
            import pty
            import termios

            master, slave = pty.openpty()
            fcntl.ioctl(
                slave,
                termios.TIOCSWINSZ,
                struct.pack("HHHH", rows, cols, 0, 0),
            )
            self.proc = subprocess.Popen(
                argv,
                stdin=slave,
                stdout=slave,
                stderr=slave,
                env=env,
                close_fds=True,
                preexec_fn=own_the_tty,
            )
            os.close(slave)
            self._master = master
            self._write = lambda text: os.write(
                master,
                text.encode("latin-1", errors="replace")
                if isinstance(text, str)
                else text,
            )

        self._pump_thread = threading.Thread(target=self._pump, daemon=True)
        self._pump_thread.start()

    # ----- reading ------------------------------------------------------

    def _pump(self) -> None:
        while not self._closed:
            try:
                if WINDOWS:
                    data = self.proc.read(65536)
                else:
                    data = os.read(self._master, 65536)
            except Exception:
                return
            if not data:
                return
            if isinstance(data, str):
                data = data.encode("utf-8", errors="replace")
            try:
                self.stream.feed(data)
            except Exception:
                return

    def text(self) -> str:
        """The screen as it is now, one row per line, right-trimmed."""
        return "\n".join(row.rstrip() for row in self.screen.display)

    def row(self, index: int) -> str:
        return self.screen.display[index].rstrip()

    def contains(self, wanted: str) -> bool:
        return wanted in self.text()

    # ----- driving ------------------------------------------------------

    def send(self, keys: str) -> None:
        """Sends raw keys and lets the editor paint before the next step."""
        self._write(keys)
        time.sleep(DELAY)

    def type(self, text: str) -> None:
        for char in text:
            self._write(char)
        time.sleep(DELAY)

    def wait_for(self, wanted: str, timeout: float = 5.0) -> bool:
        """Waits until `wanted` is on screen. `False` when it never is."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            if wanted in self.text():
                return True
            time.sleep(0.02)
        return False

    def wait_until_gone(self, timeout: float = 5.0) -> bool:
        """Waits for the process to exit."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            if self.exit_code() is not None:
                return True
            time.sleep(0.02)
        return False

    def exit_code(self):
        """The exit status if the editor has exited, else `None`."""
        if WINDOWS:
            if self.proc.isalive():
                return None
            return getattr(self.proc, "exitstatus", None)
        return self.proc.poll()

    # ----- finishing ----------------------------------------------------

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        try:
            if WINDOWS:
                self.proc.terminate(force=True)
            else:
                self.proc.terminate()
                try:
                    self.proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.proc.kill()
                os.close(self._master)
        except Exception:
            pass

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


leader = "\x00"
"""`Ctrl+Space`, which a legacy terminal sends as NUL."""
