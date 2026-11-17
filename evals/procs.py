"""Run commands so that nothing they start outlives the call.

`subprocess.run(timeout=...)` kills only the direct child. A wrapper that spawns
another process (the model adapter spawns `opencode`) would leave that process
running. Two modes cover this:

- `own_group=True` (the harness): the command starts a new session and becomes
  the leader of its own process group. The harness waits for the leader to exit
  WITHOUT reaping it, signals the group, and only then reaps. An unreaped leader
  keeps its pid and group id reserved, so a recycled pid cannot receive the
  signal.
- `own_group=False` (the adapter): the command stays in the caller's group, so
  the harness's group kill reaches it too. The adapter kills the command's
  descendants itself on a timeout.
"""

from __future__ import annotations

import json
import os
import re
import signal
import subprocess
import threading
import time
from collections.abc import Mapping, Sequence
from pathlib import Path

# The adapter must give up before the harness does, so the adapter can report why.
# The credential wait (330s), one credential export (60s) and the model call (500s)
# must stay below the harness limit.
ADAPTER_TIMEOUT = 500
OUTER_TIMEOUT = 900
POLL_SECONDS = 0.02


def _wait_exit_noreap(pid: int, timeout: float) -> None:
    """Block until child `pid` has exited, leaving it unreaped. Raises TimeoutExpired."""
    deadline = time.monotonic() + timeout
    while True:
        if os.waitid(os.P_PID, pid, os.WEXITED | os.WNOHANG | os.WNOWAIT) is not None:
            return
        if time.monotonic() >= deadline:
            raise subprocess.TimeoutExpired(str(pid), timeout)
        time.sleep(POLL_SECONDS)


def _kill_group(pgid: int) -> None:
    try:
        os.killpg(pgid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        pass


def descendants(pid: int) -> list[int]:
    """Pids below `pid`, deepest first, from the process table."""
    table = subprocess.run(
        ["ps", "-axo", "pid=,ppid="], capture_output=True, text=True, check=False
    ).stdout
    children: dict[int, list[int]] = {}
    for line in table.splitlines():
        parts = line.split()
        if len(parts) == 2:
            children.setdefault(int(parts[1]), []).append(int(parts[0]))
    found: list[int] = []
    stack = [pid]
    while stack:
        for child in children.get(stack.pop(), []):
            found.append(child)
            stack.append(child)
    return list(reversed(found))


def kill_descendants(pid: int) -> None:
    for child in descendants(pid):
        try:
            os.kill(child, signal.SIGKILL)
        except (ProcessLookupError, PermissionError):
            pass


class LiveGroups:
    """The process groups a run has started, so an interrupt can kill them all.

    `cancel` sets a flag and kills every registered group. A group registered
    after that dies at once. A group leaves the registry BEFORE its leader is
    reaped, so a recycled pid is never signalled.
    """

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._pgids: set[int] = set()
        self.cancelled = threading.Event()

    def add(self, pgid: int) -> None:
        with self._lock:
            self._pgids.add(pgid)
            if self.cancelled.is_set():
                _kill_group(pgid)

    def discard(self, pgid: int) -> None:
        with self._lock:
            self._pgids.discard(pgid)

    def cancel(self) -> None:
        with self._lock:
            self.cancelled.set()
            for pgid in self._pgids:
                _kill_group(pgid)


def run_group(
    argv: Sequence[str],
    *,
    input: str | None = None,
    timeout: float,
    cwd: str | Path | None = None,
    env: Mapping[str, str] | None = None,
    own_group: bool = True,
    live: LiveGroups | None = None,
) -> subprocess.CompletedProcess:
    """Like `subprocess.run(capture_output=True, text=True)`, with cleanup.

    The command's own exit ends the wait, not the closing of its pipes, so a
    leftover child that holds a pipe cannot stall the call. Raises
    `subprocess.TimeoutExpired` once everything is dead. An interrupt also cleans
    up before it propagates.
    """
    proc = subprocess.Popen(
        list(argv),
        stdin=subprocess.PIPE if input is not None else subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        cwd=cwd,
        env=None if env is None else dict(env),
        start_new_session=own_group,
    )
    if live is not None and own_group:
        live.add(proc.pid)
    out: list[str] = []
    err: list[str] = []
    threads = [
        threading.Thread(target=lambda: out.append(proc.stdout.read()), daemon=True),
        threading.Thread(target=lambda: err.append(proc.stderr.read()), daemon=True),
    ]
    if input is not None:
        threads.append(threading.Thread(target=_feed, args=(proc, input), daemon=True))
    for thread in threads:
        thread.start()

    def cleanup() -> None:
        if own_group:
            _kill_group(proc.pid)  # the leader is still unreaped here
        else:
            kill_descendants(proc.pid)
            proc.kill()
        if live is not None:
            live.discard(proc.pid)
        proc.wait()
        for thread in threads:
            thread.join(timeout=5)

    try:
        _wait_exit_noreap(proc.pid, timeout)
    except BaseException:
        cleanup()
        raise
    if own_group:
        # Anything the command left behind dies with it, which also closes the
        # pipes the reader threads wait on.
        _kill_group(proc.pid)
    if live is not None:
        live.discard(proc.pid)
    proc.wait()
    for thread in threads:
        thread.join(timeout=5)
    return subprocess.CompletedProcess(
        list(argv), proc.returncode, "".join(out), "".join(err)
    )


def _feed(proc: subprocess.Popen, text: str) -> None:
    try:
        proc.stdin.write(text)
        proc.stdin.close()
    except (BrokenPipeError, ValueError, OSError):
        pass


def redact(text: str) -> str:
    """Remove paths and token-like strings before text is printed or stored."""
    text = re.sub(r"/(?:Users|home)/[^\s'\"]+", "<path>", text)
    text = re.sub(r"/(?:private/)?var/folders/[^\s'\"]+", "<path>", text)
    text = re.sub(r"\b(?:AKIA|ASIA)[A-Z0-9]{12,}\b", "<key>", text)
    text = re.sub(
        r"(?i)\b(bearer|token|secret|password|api[_-]?key)\b\s*[=:]?\s*\S+",
        r"\1 <redacted>",
        text,
    )
    return re.sub(r"\b[A-Za-z0-9+/_-]{32,}={0,2}", "<redacted>", text)


# --- token usage side channel --------------------------------------------------------
#
# The adapter keeps stdout for the answer alone. It reports token usage as the
# LAST stderr line: USAGE_PREFIX, then a JSON object of non-negative integers.
# Only that last non-empty line counts. The adapter folds every diagnostic onto one
# line (`one_line`), so error text cannot start a line of its own and pose as usage.

USAGE_PREFIX = "slopvac-usage "
USAGE_KEYS = ("input", "output", "reasoning", "cache_read", "cache_write")


def format_usage_line(usage: Mapping[str, int]) -> str:
    return USAGE_PREFIX + json.dumps(dict(usage), sort_keys=True)


def one_line(text: str) -> str:
    """Fold every line break and control character to a space."""
    return re.sub(r"[\x00-\x1f\x7f\u2028\u2029\x85]+", " ", text)


def parse_usage_line(stderr: str) -> dict[str, int] | None:
    """The usage from the last non-empty line of `stderr`, if it is a usage line."""
    lines = [line for line in stderr.splitlines() if line.strip()]
    if not lines or not lines[-1].startswith(USAGE_PREFIX):
        return None
    try:
        data = json.loads(lines[-1].removeprefix(USAGE_PREFIX))
    except json.JSONDecodeError:
        return None
    if not isinstance(data, dict):
        return None
    usage = {
        key: value
        for key, value in data.items()
        if key in USAGE_KEYS
        and isinstance(value, int)
        and not isinstance(value, bool)
        and value >= 0
    }
    return usage or None


def strip_usage_lines(stderr: str) -> str:
    return "\n".join(
        line for line in stderr.splitlines() if not line.startswith(USAGE_PREFIX)
    )


def sum_usage(parts: Sequence[Mapping[str, int] | None]) -> dict[str, int] | None:
    """Add usage over several invocations. One missing report makes the total unknown."""
    if not parts or any(part is None for part in parts):
        return None
    total: dict[str, int] = {}
    for part in parts:
        for key, value in part.items():
            total[key] = total.get(key, 0) + value
    return total
