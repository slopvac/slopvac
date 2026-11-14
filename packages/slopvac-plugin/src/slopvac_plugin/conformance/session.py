"""One plugin process driven over stdio, with a strict reader for what it prints.

The plugin under test is untrusted (threat model TM-2, TM-3, TM-4, TM-8). The session
gives it a minimal environment, its own process group, bounded memory for what it
prints, and a deadline on every write.
"""

from __future__ import annotations

import json
import os
import queue
import signal
import subprocess
import threading
import time
from collections.abc import Callable, Iterable, Mapping, Sequence
from contextlib import suppress
from pathlib import Path
from typing import Any

from .. import framing
from .schemas import Schemas
from .winjob import JobObject

# Limits on what a plugin may print, from the host's (section 3 and TM-2).
MAX_OUTPUT_BYTES = 512 * 1024 * 1024  # stdout and stderr together
MAX_QUEUED_MESSAGES = 1000  # frames the suite has not read yet
MAX_QUEUED_BYTES = 64 * 1024 * 1024
STDERR_TAIL_BYTES = 4096
STDERR_CHUNK = 64 * 1024

# Environment names every plugin gets, as the host passes them (section 11.1).
BASE_ENVIRONMENT = ("PATH", "LANG")
# A Windows process does not start without these.
WINDOWS_ENVIRONMENT = ("SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT", "TEMP", "TMP")


class CaseFailure(Exception):
    """The plugin broke the contract."""


class CaseSkip(Exception):
    """The case does not apply. The message is the reason."""


class SuiteError(Exception):
    """The suite cannot run, for example because the command does not start."""


def plugin_environment(
    extra: Iterable[str] = (), base: Mapping[str, str] | None = None
) -> dict[str, str]:
    """The allowlisted environment: PATH, LANG, the Windows basics, and ``extra`` names.

    A name that is not set in ``base`` is left out. CI tokens and credentials never
    reach the plugin unless the caller names them.
    """
    source = os.environ if base is None else base
    names = [*BASE_ENVIRONMENT, *extra]
    if os.name == "nt":
        names += WINDOWS_ENVIRONMENT
    for name in extra:
        if not name or "=" in name:
            raise ValueError(f"not an environment variable name: {name!r}")
    return {name: source[name] for name in dict.fromkeys(names) if name in source}


def taskkill_command(pid: int, system_root: str | None = None) -> list[str]:
    """The Windows command that ends a process and its descendants."""
    root = system_root or os.environ.get("SystemRoot") or r"C:\Windows"
    return [
        str(Path(root) / "System32" / "taskkill.exe"),
        "/PID",
        str(pid),
        "/T",
        "/F",
    ]


def kill_tree(proc: subprocess.Popen[bytes], job: JobObject | None = None) -> None:
    """End the plugin and everything it started. Safe to call twice.

    On Windows the job ends descendants that outlive the plugin. ``taskkill`` stays
    as the fallback for a plugin that could not be put in a job.
    """
    if os.name == "nt":
        if job is not None:
            with suppress(OSError):
                job.terminate()
        with suppress(OSError, subprocess.SubprocessError):
            subprocess.run(
                taskkill_command(proc.pid),
                check=False,
                capture_output=True,
                timeout=10,
            )
    else:
        # The plugin leads its own process group (start_new_session).
        with suppress(ProcessLookupError, PermissionError):
            os.killpg(proc.pid, signal.SIGKILL)
    with suppress(OSError):
        proc.kill()


def _bounded(fn: Callable[[], Any], seconds: float) -> bool:
    """Run ``fn`` on a helper thread. False when it is still running after ``seconds``."""
    thread = threading.Thread(target=fn, daemon=True)
    thread.start()
    thread.join(seconds)
    return not thread.is_alive()


class Session:
    def __init__(
        self,
        command: Sequence[str],
        schemas: Schemas,
        timeout: float,
        *,
        env_names: Iterable[str] = (),
        max_output_bytes: int = MAX_OUTPUT_BYTES,
        max_queued_messages: int = MAX_QUEUED_MESSAGES,
        max_queued_bytes: int = MAX_QUEUED_BYTES,
    ) -> None:
        self.schemas = schemas
        self.timeout = timeout
        self._max_output = max_output_bytes
        self._max_messages = max_queued_messages
        self._max_queued_bytes = max_queued_bytes
        self._ids = 0
        # Set by the handshake: the batch size and body size a request may use.
        self.max_documents = 1
        self.max_body_bytes = framing.PRE_INITIALIZE_LIMIT
        self._events: queue.Queue[tuple[str, Any, int]] = queue.Queue()
        self._lock = threading.Lock()
        self._output = 0
        self._queued = 0
        self._queued_bytes = 0
        self._violation: str | None = None
        self._swept = False
        self._stderr = bytearray()
        # Reply limit: 1 MiB until the initialize reply, then the negotiated limit.
        self._limit = framing.PRE_INITIALIZE_LIMIT
        self._initialize_id: int | None = None
        self._proposed = framing.PRE_INITIALIZE_LIMIT
        options: dict[str, Any]
        if os.name == "nt":
            options = {"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP}
        else:
            options = {"start_new_session": True}
        try:
            self.proc = subprocess.Popen(
                list(command),
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                env=plugin_environment(env_names),
                **options,
            )
        except OSError as exc:
            raise SuiteError(f"cannot start the plugin: {exc}") from exc
        self._job = JobObject.around(self.proc)
        self._readers = [
            threading.Thread(target=target, daemon=True)
            for target in (self._read_stdout, self._drain_stderr)
        ]
        for thread in self._readers:
            thread.start()

    # Streams ---------------------------------------------------------------

    def _violate(self, message: str) -> None:
        """Record the first violation, tell the reader of events, end the plugin."""
        with self._lock:
            if self._violation is not None:
                return
            self._violation = message
        self._events.put(("violation", message, 0))
        kill_tree(self.proc, self._job)

    def _count_output(self, size: int) -> bool:
        with self._lock:
            self._output += size
            over = self._output > self._max_output
        if over:
            self._violate(f"the plugin printed more than {self._max_output} bytes")
        return not over

    def _enqueue(self, kind: str, value: Any, size: int) -> bool:
        with self._lock:
            self._queued += 1
            self._queued_bytes += size
            over = (
                self._queued > self._max_messages
                or self._queued_bytes > self._max_queued_bytes
            )
        if over:
            self._violate(
                "too many unread messages from the plugin "
                f"(limits: {self._max_messages} messages, {self._max_queued_bytes} bytes)"
            )
            return False
        self._events.put((kind, value, size))
        return True

    def _put_eof(self) -> None:
        if self._violation is None:
            self._events.put(("eof", None, 0))

    def _note_reply(self, message: Any) -> None:
        """Take the negotiated limit from the initialize reply, as the plugin must."""
        if (
            self._initialize_id is None
            or not isinstance(message, dict)
            or type(message.get("id")) is not int
            or message["id"] != self._initialize_id
            or "result" not in message
        ):
            return
        self._initialize_id = None
        result = message["result"]
        limits = result.get("limits") if isinstance(result, dict) else None
        offered = limits.get("maxMessageBytes") if isinstance(limits, dict) else None
        if type(offered) is int and offered >= framing.MIN_MESSAGE_BYTES:
            self._limit = min(self._proposed, offered)
        else:
            self._limit = self._proposed

    def _read_stdout(self) -> None:
        stream = self.proc.stdout
        assert stream is not None
        try:
            while True:
                body = framing.read_frame(stream, lambda: self._limit)
                if body is None:
                    self._put_eof()
                    return
                # Count the header too, so a flood of empty frames is not free.
                if not self._count_output(len(body) + 32):
                    return
                message = framing.loads_strict(body)
                self._note_reply(message)
                if not self._enqueue("frame", message, len(body)):
                    return
        except (framing.FramingError, framing.ParseError) as exc:
            self._violate(str(exc))
        except (OSError, ValueError):
            self._put_eof()

    def _drain_stderr(self) -> None:
        stream = self.proc.stderr
        assert stream is not None
        try:
            while True:
                chunk = stream.read1(STDERR_CHUNK)  # type: ignore[attr-defined]
                if not chunk:
                    return
                if not self._count_output(len(chunk)):
                    return
                self._stderr += chunk
                del self._stderr[:-STDERR_TAIL_BYTES]
        except (OSError, ValueError):
            pass

    def stderr_tail(self) -> str:
        return bytes(self._stderr).decode("utf-8", "replace")[-400:]

    # Sending -----------------------------------------------------------------

    def send_bytes(
        self,
        data: bytes,
        *,
        tolerate_closed: bool = False,
        timeout: float | None = None,
    ) -> None:
        """Write to the plugin within ``timeout`` seconds, else end it and fail.

        ``timeout`` defaults to the session timeout.
        """
        limit = self.timeout if timeout is None else timeout
        stdin = self.proc.stdin
        assert stdin is not None
        problem: list[Exception] = []

        def write() -> None:
            try:
                stdin.write(data)
                stdin.flush()
            except (OSError, ValueError) as exc:
                problem.append(exc)

        writer = threading.Thread(target=write, daemon=True)
        writer.start()
        writer.join(limit)
        if writer.is_alive():
            kill_tree(self.proc, self._job)  # the broken pipe frees the writer
            writer.join(5)
            raise CaseFailure(f"the plugin did not read its input within {limit} s")
        if problem and not tolerate_closed:
            raise CaseFailure(f"the plugin closed its input early: {problem[0]}")

    def send(self, message: dict[str, Any], *, timeout: float | None = None) -> None:
        self.send_bytes(
            framing.encode_frame(json.dumps(message).encode("utf-8")), timeout=timeout
        )

    def notify(
        self,
        method: str,
        params: dict[str, Any] | None = None,
        *,
        timeout: float | None = None,
    ) -> None:
        message: dict[str, Any] = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            message["params"] = params
        self.send(message, timeout=timeout)

    def cancel_and_wait(self, request_id: int, grace: float) -> dict[str, Any]:
        """Cancel a request and take its response, all within ``grace`` seconds.

        Section 6.3 gives the plugin one grace period from the cancel. A write that
        blocks uses up the same period, so a late reply cannot pass.
        """
        deadline = time.monotonic() + grace
        self.notify("$/cancelRequest", {"id": request_id}, timeout=grace)
        return self.response_to(request_id, max(deadline - time.monotonic(), 0))

    def start_request(
        self,
        method: str,
        params: dict[str, Any] | None = None,
        *,
        timeout: float | None = None,
    ) -> int:
        self._ids += 1
        message: dict[str, Any] = {"jsonrpc": "2.0", "id": self._ids, "method": method}
        if params is not None:
            message["params"] = params
        if method == "initialize":
            self._initialize_id = self._ids
            proposed = ((params or {}).get("limits") or {}).get("maxMessageBytes")
            self._proposed = (
                proposed if type(proposed) is int else framing.PRE_INITIALIZE_LIMIT
            )
        self.send(message, timeout=timeout)
        return self._ids

    def close_stdin(self) -> None:
        def close() -> None:
            with suppress(OSError, ValueError):
                assert self.proc.stdin is not None
                self.proc.stdin.close()

        _bounded(close, 1)

    # Receiving -----------------------------------------------------------------

    def _get(self, timeout: float) -> tuple[str, Any]:
        kind, value, size = self._events.get(timeout=timeout)
        if kind == "frame":
            with self._lock:
                self._queued -= 1
                self._queued_bytes -= size
        return kind, value

    def next_event(self, timeout: float | None = None) -> tuple[str, Any]:
        try:
            return self._get(self.timeout if timeout is None else timeout)
        except queue.Empty:
            raise CaseFailure("no reply in time") from None

    def next_frame(self, timeout: float | None = None) -> dict[str, Any]:
        kind, value = self.next_event(timeout)
        if kind == "violation":
            raise CaseFailure(f"standard output broke the framing rules: {value}")
        if kind == "eof":
            raise CaseFailure(
                f"the plugin ended its output. stderr: {self.stderr_tail()!r}"
            )
        # The v1 protocol has no plugin-to-host call (section 3), so refuse one before
        # its params reach the schema: they are plugin-controlled and may be large.
        if isinstance(value, dict) and "method" in value:
            raise CaseFailure(
                f"the plugin sent a call ({str(value['method'])[:60]!r}): "
                "protocol v1 has no plugin-to-host calls"
            )
        problems = self.schemas.errors(None, value)
        if problems:
            raise CaseFailure(f"message breaks the schema: {problems}")
        return value

    def response_to(
        self, request_id: int, timeout: float | None = None
    ) -> dict[str, Any]:
        message = self.next_frame(timeout)
        # type() is int: JSON 1.0 equals 1 in Python, and the contract forbids it.
        if (
            type(message.get("id")) is not int
            or message["id"] != request_id
            or "method" in message
        ):
            raise CaseFailure(
                f"expected the response to request {request_id}, got {message}"
            )
        return message

    def request(
        self, method: str, params: dict[str, Any] | None = None
    ) -> dict[str, Any]:
        return self.response_to(self.start_request(method, params))

    def drain_events(self, timeout: float = 0.5) -> list[tuple[str, Any]]:
        """Events up to the end of output (or the timeout), for 'nothing else was sent'."""
        events: list[tuple[str, Any]] = []
        while True:
            try:
                kind, value = self._get(timeout)
            except queue.Empty:
                return events
            events.append((kind, value))
            if kind in ("eof", "violation"):
                return events

    # Ending --------------------------------------------------------------------

    def _sweep(self) -> None:
        """End descendants that outlive the plugin. They would hold the pipes open."""
        if not self._swept:
            self._swept = True
            kill_tree(self.proc, self._job)

    def wait(self, timeout: float) -> int:
        try:
            status = self.proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            raise CaseFailure(f"the plugin did not exit within {timeout} s") from None
        self._sweep()
        return status

    def close(self) -> None:
        self.close_stdin()
        with suppress(subprocess.TimeoutExpired):
            self.proc.wait(timeout=0.5)
        self._sweep()
        with suppress(subprocess.TimeoutExpired):
            self.proc.wait(timeout=5)
        for thread in self._readers:
            thread.join(timeout=2)
        # A reader that is still blocked holds the stream lock, and close() would wait
        # for it. That happens only when a descendant left the process group.
        for stream, thread in zip(
            (self.proc.stdout, self.proc.stderr), self._readers, strict=True
        ):
            if stream is not None and not thread.is_alive():
                with suppress(OSError, ValueError):
                    stream.close()
