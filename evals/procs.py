"""Run a command in its own process group and kill the whole group on any exit path.

`subprocess.run(timeout=...)` kills only the direct child. A wrapper that spawns
another process (the model adapter spawns `opencode`) would leave that process
running. Here the command starts a new session, so `killpg` reaches every
descendant, and the group is reaped before the call returns or raises.
"""

from __future__ import annotations

import os
import re
import signal
import subprocess
import threading
from collections.abc import Mapping, Sequence
from pathlib import Path

# The adapter must give up before the harness does, so the adapter can report why.
ADAPTER_TIMEOUT = 600
OUTER_TIMEOUT = 900


def _kill_group(proc: subprocess.Popen) -> None:
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        pass


def run_group(
    argv: Sequence[str],
    *,
    input: str | None = None,
    timeout: float,
    cwd: str | Path | None = None,
    env: Mapping[str, str] | None = None,
) -> subprocess.CompletedProcess:
    """Like `subprocess.run(capture_output=True, text=True)`, with group cleanup.

    The command's own exit ends the wait, not the closing of its pipes, so a
    leftover child that holds a pipe cannot stall the call. When the command exits
    or the timeout passes, the whole group is killed and the pipes are drained.
    Raises `subprocess.TimeoutExpired` after the group is dead. An interrupt also
    kills the group before it propagates.
    """
    proc = subprocess.Popen(
        list(argv),
        stdin=subprocess.PIPE if input is not None else subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        cwd=cwd,
        env=None if env is None else dict(env),
        start_new_session=True,
    )
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
    try:
        proc.wait(timeout=timeout)
    except BaseException:
        _kill_group(proc)
        proc.wait()
        for thread in threads:
            thread.join(timeout=5)
        raise
    # Anything the command left behind in its group dies with it, which also
    # closes the pipes the reader threads wait on.
    _kill_group(proc)
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
