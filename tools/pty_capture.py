"""Run a command under a ConPTY, send bytes, print everything it wrote.

    python tools/pty_capture.py <exe> [key ...]

Keys are Python escape sequences, e.g. 'X', '\\r', '\\x13' for Ctrl+S.
"""

import os
import sys
import threading
import time

from winpty import PtyProcess

ROWS = 24
COLS = 90
DELAY = float(os.environ.get("PTY_DELAY", "0.3"))


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    exe = sys.argv[1]
    keys = [
        k.encode("latin-1", errors="backslashreplace").decode("unicode_escape")
        for k in sys.argv[2:]
    ]

    proc = PtyProcess.spawn([exe], dimensions=(ROWS, COLS))
    chunks = []

    def pump():
        while True:
            try:
                data = proc.read(4096)
            except Exception:
                return
            if not data:
                return
            chunks.append(data)

    reader = threading.Thread(target=pump, daemon=True)
    reader.start()
    time.sleep(DELAY)

    for key in keys:
        proc.write(key)
        time.sleep(DELAY)

    proc.wait()
    reader.join(timeout=2.0)
    out = "".join(chunks).replace("\r\n", "\n")
    sys.stdout.buffer.write(out.encode("utf-8", errors="replace"))
    sys.stdout.buffer.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
