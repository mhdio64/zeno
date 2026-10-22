"""Drive the zeno editor in a real terminal and show what ends up on screen.

Spawns the editor under a ConPTY, sends keystrokes, and renders the screen
through a terminal emulator so the result can be read as plain text.

    python tools/tui_probe.py <file> [key ...]

Keys are given as Python escape sequences, e.g. 'X', '\\r', '\\x13' for Ctrl+S.
"""

import os
import sys
import threading
import time

import pyte
from winpty import PtyProcess

ROWS = 24
COLS = 90
DELAY = float(os.environ.get("TUI_PROBE_DELAY", "0.15"))


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    target = sys.argv[1]
    keys = [k.encode("latin-1", errors="backslashreplace").decode("unicode_escape") for k in sys.argv[2:]]

    screen = pyte.Screen(COLS, ROWS)
    stream = pyte.ByteStream(screen)

    proc = PtyProcess.spawn(
        ["target/release/zeno.exe", target],
        dimensions=(ROWS, COLS),
    )

    def pump():
        while True:
            try:
                data = proc.read(4096)
            except Exception:
                return
            if not data:
                return
            if isinstance(data, str):
                data = data.encode("utf-8", errors="replace")
            try:
                stream.feed(data)
            except Exception:
                return

    threading.Thread(target=pump, daemon=True).start()
    time.sleep(0.7)

    for key in keys:
        proc.write(key)
        time.sleep(DELAY)
    time.sleep(0.5)

    print("+" + "-" * COLS + "+")
    for line in screen.display:
        print("|" + line + "|")
    print("+" + "-" * COLS + "+")

    try:
        proc.terminate(force=True)
    except Exception:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
