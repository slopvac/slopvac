"""A plugin that attacks the conformance session. Standard library only.

Usage: hostile_plugin.py MODE [ARG]. Modes:
  env-probe PATH       write the environment variable names, one per line, to PATH
  stderr-flood         print 8 MiB to standard error without a newline
  frame-flood COUNT    print COUNT small valid frames
  big-frames COUNT     print COUNT valid frames of 100 KiB
  trickle              print a small valid frame every 0.2 s for 30 s
  deaf                 never read standard input
  spawner PATH         start a descendant that keeps the pipes open, write its pid to PATH
  spawner-exit PATH    the same, and then exit at once
"""

import os
import subprocess
import sys
import time

mode = sys.argv[1]
arg = sys.argv[2] if len(sys.argv) > 2 else ""
out = sys.stdout.buffer


def frame(pad: int) -> bytes:
    body = b'{"jsonrpc":"2.0","method":"x-note","params":{"x-pad":"%s"}}' % (b"a" * pad)
    return b"Content-Length: %d\r\n\r\n" % len(body) + body


if mode == "env-probe":
    with open(arg, "w", encoding="utf-8") as handle:
        handle.write("\n".join(sorted(os.environ)))
elif mode == "stderr-flood":
    for _ in range(128):
        sys.stderr.buffer.write(b"x" * 65536)
        sys.stderr.buffer.flush()
    time.sleep(60)
elif mode == "frame-flood":
    for _ in range(int(arg)):
        out.write(frame(0))
    out.flush()
    time.sleep(60)
elif mode == "big-frames":
    for _ in range(int(arg)):
        out.write(frame(100 * 1024))
        out.flush()
    time.sleep(60)
elif mode == "trickle":
    for _ in range(150):
        out.write(frame(0))
        out.flush()
        time.sleep(0.2)
elif mode in ("spawner", "spawner-exit"):
    child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(120)"])
    with open(arg, "w", encoding="utf-8") as handle:
        handle.write(str(child.pid))
    if mode == "spawner":
        time.sleep(60)
else:  # deaf
    time.sleep(60)
