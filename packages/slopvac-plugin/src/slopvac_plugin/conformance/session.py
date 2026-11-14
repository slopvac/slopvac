"""One plugin process driven over stdio, with a strict reader for what it prints."""

from __future__ import annotations

import json
import queue
import subprocess
import threading
from collections import deque
from collections.abc import Sequence
from typing import Any

from .. import framing
from .schemas import Schemas

READ_LIMIT = 64 * 1024 * 1024


class CaseFailure(Exception):
    """The plugin broke the contract."""


class CaseSkip(Exception):
    """The case does not apply. The message is the reason."""


class SuiteError(Exception):
    """The suite cannot run, for example because the command does not start."""


class Session:
    def __init__(
        self, command: Sequence[str], schemas: Schemas, timeout: float
    ) -> None:
        self.schemas = schemas
        self.timeout = timeout
        self._ids = 0
        self._events: queue.Queue[tuple[str, Any]] = queue.Queue()
        self._stderr: deque[bytes] = deque(maxlen=64)
        try:
            self.proc = subprocess.Popen(
                list(command),
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
        except OSError as exc:
            raise SuiteError(f"cannot start the plugin: {exc}") from exc
        self._threads = [
            threading.Thread(target=target, daemon=True)
            for target in (self._read_stdout, self._drain_stderr)
        ]
        for thread in self._threads:
            thread.start()

    # Streams ---------------------------------------------------------------

    def _read_stdout(self) -> None:
        stream = self.proc.stdout
        assert stream is not None
        try:
            while True:
                body = framing.read_frame(stream, lambda: READ_LIMIT)
                if body is None:
                    self._events.put(("eof", None))
                    return
                self._events.put(("frame", framing.loads_strict(body)))
        except (framing.FramingError, framing.ParseError) as exc:
            self._events.put(("violation", str(exc)))
        except (OSError, ValueError):
            self._events.put(("eof", None))

    def _drain_stderr(self) -> None:
        stream = self.proc.stderr
        assert stream is not None
        try:
            for line in stream:
                self._stderr.append(line[:512])
        except (OSError, ValueError):
            pass

    def stderr_tail(self) -> str:
        return b"".join(self._stderr).decode("utf-8", "replace")[-400:]

    # Sending -----------------------------------------------------------------

    def send_bytes(self, data: bytes, *, tolerate_closed: bool = False) -> None:
        stdin = self.proc.stdin
        assert stdin is not None
        try:
            stdin.write(data)
            stdin.flush()
        except OSError as exc:
            if not tolerate_closed:
                raise CaseFailure(f"the plugin closed its input early: {exc}") from exc

    def send(self, message: dict[str, Any]) -> None:
        self.send_bytes(framing.encode_frame(json.dumps(message).encode("utf-8")))

    def notify(self, method: str, params: dict[str, Any] | None = None) -> None:
        message: dict[str, Any] = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            message["params"] = params
        self.send(message)

    def start_request(self, method: str, params: dict[str, Any] | None = None) -> int:
        self._ids += 1
        message: dict[str, Any] = {"jsonrpc": "2.0", "id": self._ids, "method": method}
        if params is not None:
            message["params"] = params
        self.send(message)
        return self._ids

    def close_stdin(self) -> None:
        try:
            assert self.proc.stdin is not None
            self.proc.stdin.close()
        except OSError:
            pass

    # Receiving -----------------------------------------------------------------

    def next_event(self, timeout: float | None = None) -> tuple[str, Any]:
        try:
            return self._events.get(
                timeout=self.timeout if timeout is None else timeout
            )
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
        problems = self.schemas.errors(None, value)
        if problems:
            raise CaseFailure(f"message breaks the schema: {problems}")
        return value

    def response_to(
        self, request_id: int, timeout: float | None = None
    ) -> dict[str, Any]:
        message = self.next_frame(timeout)
        if message.get("id") != request_id or "method" in message:
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
                kind, value = self._events.get(timeout=timeout)
            except queue.Empty:
                return events
            events.append((kind, value))
            if kind in ("eof", "violation"):
                return events

    # Ending --------------------------------------------------------------------

    def wait(self, timeout: float) -> int:
        try:
            return self.proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            raise CaseFailure(f"the plugin did not exit within {timeout} s") from None

    def close(self) -> None:
        self.close_stdin()
        try:
            self.proc.wait(timeout=0.5)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()
        for thread in self._threads:
            thread.join(timeout=2)  # a blocked reader would hold the stream lock
        for stream in (self.proc.stdout, self.proc.stderr):
            if stream is not None:
                stream.close()
