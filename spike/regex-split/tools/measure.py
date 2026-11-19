"""SPIKE harness: run one command under a wall-clock timeout and an address-space cap,
then append one JSON line with wall seconds, peak RSS of the child, exit code and the
reason it was killed, if any.

Usage: python3 measure.py <out.jsonl> <label> <timeout_s> <mem_mib> -- <cmd...>
Env for the child passes through unchanged. stdout of the child goes to /dev/null.
"""
import json
import os
import resource
import subprocess
import sys
import time

out, label, timeout_s, mem_mib = sys.argv[1:5]
assert sys.argv[5] == "--"
cmd = sys.argv[6:]
timeout_s = int(timeout_s)
mem = int(mem_mib) << 20


def cap():
    resource.setrlimit(resource.RLIMIT_AS, (mem, mem))


t0 = time.monotonic()
killed = None
p = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, preexec_fn=cap)
try:
    _, err = p.communicate(timeout=timeout_s)
except subprocess.TimeoutExpired:
    p.kill()
    _, err = p.communicate()
    killed = f"> {timeout_s} s, killed"
wall = time.monotonic() - t0
ru = resource.getrusage(resource.RUSAGE_CHILDREN)
rc = p.returncode
err = err.decode("utf-8", "replace")
if killed is None and rc != 0 and ("memory allocation" in err or "out of memory" in err.lower() or rc in (-6, 134)):
    killed = f"killed at {mem_mib} MiB address-space cap (rc={rc})"
rec = {
    "label": label,
    "wall_s": round(wall, 3),
    "peak_rss_mib": round(ru.ru_maxrss / 1024, 1),
    "rc": rc,
    "killed": killed,
    "stderr_tail": err[-600:],
    "load1": os.getloadavg()[0],
}
with open(out, "a") as f:
    f.write(json.dumps(rec) + "\n")
print(json.dumps(rec))
