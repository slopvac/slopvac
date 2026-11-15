"""The plugin runtime: framing, lifecycle, dispatch, and hook registration."""

from __future__ import annotations

import logging
import os
import queue
import re
import sys
import threading
import traceback
from collections.abc import Callable
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from typing import Any, BinaryIO, NoReturn

from . import framing
from .errors import (
    INTERNAL_ERROR,
    INVALID_PARAMS,
    INVALID_REQUEST,
    MESSAGE_TOO_LARGE,
    METHOD_NOT_FOUND,
    NOT_INITIALIZED,
    PARSE_ERROR,
    PROTOCOL_VERSION_UNSUPPORTED,
    REQUIRED_FEATURE_MISSING,
    OutputInvalid,
    RpcError,
)
from .hooks import BY_METHOD, KINDS, HookKind
from .types import Context
from .validate import KEBAB, is_int, is_request_id

log = logging.getLogger("slopvac_plugin")

PROTOCOL_MIN = 1
PROTOCOL_MAX = 1
DEFAULT_MAX_BATCH_DOCUMENTS = 64
DEFAULT_MAX_IN_FLIGHT = 1
FEATURE_NAME = re.compile(r"^[a-z][a-z0-9-]*$|^x-[a-z0-9-]+$")


@dataclass(frozen=True)
class _Fatal:
    reason: str


@dataclass(frozen=True)
class _Eof:
    pass


@dataclass(frozen=True)
class _Incoming:
    """A parsed message and what the reader reserved for it.

    ``reserved`` means the reader holds an admission slot (a cancel event) for the
    message id; whoever rejects the message must release it. ``duplicate`` means the
    id is already pending, so the message gets an error and holds nothing.
    """

    message: Any
    reserved: bool = False
    duplicate: bool = False


@dataclass(frozen=True)
class _Unreadable:
    """A body that is not JSON. The answer carries a null id."""

    error: RpcError


class Plugin:
    """Register hooks with the decorators, then call ``run()``.

    ``network`` must equal the ``network`` value of the manifest declaration. Hooks
    offered must be a subset of the declared hooks.
    """

    def __init__(
        self,
        name: str,
        version: str,
        *,
        network: bool = False,
        required_features: tuple[str, ...] = (),
        optional_features: tuple[str, ...] = (),
        max_in_flight: int = DEFAULT_MAX_IN_FLIGHT,
        max_message_bytes: int = framing.DEFAULT_MAX_MESSAGE_BYTES,
        max_batch_documents: int = DEFAULT_MAX_BATCH_DOCUMENTS,
    ) -> None:
        for feature in (*required_features, *optional_features):
            if not FEATURE_NAME.match(feature):
                raise ValueError(f"invalid feature name: {feature!r}")
        if max_in_flight < 1 or max_batch_documents < 1:
            raise ValueError("limits must be at least 1")
        if max_message_bytes < framing.MIN_MESSAGE_BYTES:
            raise ValueError(
                f"max_message_bytes must be >= {framing.MIN_MESSAGE_BYTES}"
            )
        self.name, self.version, self.network = name, version, network
        self.required_features = tuple(required_features)
        self.optional_features = tuple(optional_features)
        self.max_in_flight = max_in_flight
        self.max_message_bytes = max_message_bytes
        self.max_batch_documents = max_batch_documents
        self._hooks: dict[tuple[str, str], Callable[..., Any]] = {}
        # Negotiated state. The reader thread reads _limit, so it is a plain int.
        self._limit = framing.PRE_INITIALIZE_LIMIT
        self._batch = max_batch_documents
        self._offline = False
        self._granted: frozenset[str] = frozenset()
        self._events: dict[int, threading.Event] = {}
        # Guards _events. The reader waits on it for a free slot (section 6.1).
        self._events_lock = threading.Condition()
        self._write_lock = threading.Lock()
        self._out: BinaryIO | None = None

    # Registration ---------------------------------------------------------

    def hook(
        self, kind: str, hook_id: str
    ) -> Callable[[Callable[..., Any]], Callable[..., Any]]:
        """Register a handler for one hook kind. The kind-named decorators call this."""
        if kind not in KINDS:
            raise ValueError(f"unknown hook kind: {kind!r}")
        if not KEBAB.match(hook_id):
            raise ValueError(f"invalid hook id: {hook_id!r}")

        def register(fn: Callable[..., Any]) -> Callable[..., Any]:
            if (kind, hook_id) in self._hooks:
                raise ValueError(f"hook registered twice: {kind}/{hook_id}")
            self._hooks[(kind, hook_id)] = fn
            return fn

        return register

    def tagger(self, hook_id: str):
        """``fn(doc: Document, ctx: Context) -> Iterable[Tag]``."""
        return self.hook("tagger", hook_id)

    def extractor(self, hook_id: str):
        """``fn(doc: Document, ctx: Context) -> Iterable[Segment]``."""
        return self.hook("extractor", hook_id)

    def reporter(self, hook_id: str):
        """``fn(report: dict, options: dict, ctx: Context) -> Rendered``."""
        return self.hook("reporter", hook_id)

    def fixer(self, hook_id: str):
        """``fn(doc: Document, findings: list[Finding], ctx: Context) -> Iterable[Edit]``."""
        return self.hook("fixer", hook_id)

    def scorer(self, hook_id: str):
        """``fn(doc: Document, findings: list[Finding], ctx: Context) -> float``."""
        return self.hook("scorer", hook_id)

    def provider(self, hook_id: str):
        """``fn(item: Item, task: str, model: str, ctx: Context) -> Completion``."""
        return self.hook("provider", hook_id)

    def recipe(self, hook_id: str):
        """``fn(harness: str, options: dict, ctx: Context) -> Iterable[RecipeFile]``."""
        return self.hook("recipe", hook_id)

    # Entry points ---------------------------------------------------------

    def run(self) -> NoReturn:
        """Serve on the process streams and exit with the protocol's exit status.

        Standard output carries frames only: the real descriptor is kept for frames,
        and descriptor 1 and ``sys.stdout`` point at standard error, so ``print``,
        logging to stdout, C extensions and child processes cannot corrupt a frame.
        """
        sys.stdout.flush()
        frames = os.fdopen(os.dup(1), "wb")
        os.dup2(2, 1)
        sys.stdout = sys.stderr
        logging.basicConfig(stream=sys.stderr, level=logging.INFO)
        code = self.serve(sys.stdin.buffer, frames)
        try:
            frames.flush()
            sys.stderr.flush()
        finally:
            # The reader thread may sit in a blocking read of standard input.
            os._exit(code)

    def serve(self, stdin: BinaryIO, stdout: BinaryIO) -> int:
        """Run the lifecycle on the given streams and return the exit status."""
        self._out = stdout
        # Bounded to twice maxInFlight frames: the reader blocks when it is full
        # (threat model section 5.5), so a flooding peer cannot grow memory.
        inbox: queue.Queue[Any] = queue.Queue(maxsize=2 * self.max_in_flight)
        threading.Thread(
            target=self._read_loop, args=(stdin, inbox), name="reader", daemon=True
        ).start()
        # Hooks never run on this thread, even for maxInFlight 1: end of input and
        # framing errors must not wait behind a slow handler (section 3, P12).
        pool = ThreadPoolExecutor(self.max_in_flight, thread_name_prefix="hook")
        state = "new"
        try:
            while True:
                item = inbox.get()
                if isinstance(item, _Fatal):
                    log.error("framing error: %s", item.reason)
                    return 1
                if isinstance(item, _Eof):
                    return 0 if state == "shutdown" else 1
                if isinstance(item, _Unreadable):
                    self._respond_error(None, item.error)
                    continue
                if item.duplicate:
                    self._respond_error(
                        item.message["id"],
                        RpcError(INVALID_REQUEST, "a request with this id is pending"),
                    )
                    continue
                outcome = self._handle(item.message, item.reserved, state, pool)
                if isinstance(outcome, int):
                    return outcome
                state = outcome
        finally:
            with self._events_lock:
                for event in self._events.values():
                    event.set()
            pool.shutdown(wait=False, cancel_futures=True)

    # Reader thread ----------------------------------------------------------

    def _read_loop(self, stdin: BinaryIO, inbox: queue.Queue[Any]) -> None:
        try:
            while True:
                body = framing.read_frame(stdin, lambda: self._limit)
                if body is None:
                    inbox.put(_Eof())
                    return
                try:
                    message = framing.loads_strict(body)
                except framing.ParseError as exc:
                    inbox.put(_Unreadable(RpcError(PARSE_ERROR, str(exc))))
                    continue
                if self._take_cancel(message):
                    continue
                inbox.put(self._admit(message))
        except framing.FramingError as exc:
            inbox.put(_Fatal(str(exc)))
        except (OSError, ValueError):
            inbox.put(_Eof())

    def _take_cancel(self, message: Any) -> bool:
        """Handle ``$/cancelRequest`` on the reader thread so a busy handler sees it."""
        if not (
            isinstance(message, dict)
            and message.get("method") == "$/cancelRequest"
            and "id" not in message
        ):
            return False
        params = message.get("params")
        target = params.get("id") if isinstance(params, dict) else None
        if is_request_id(target):
            with self._events_lock:
                event = self._events.get(target)
            if event is not None:
                event.set()
        return True

    def _admit(self, message: Any) -> _Incoming:
        """Reserve capacity for a well-formed hook request; pass anything else on.

        Only a request the dispatcher can run reserves a slot, so every rejection
        of one releases it (``_handle``) and a malformed message holds nothing. A
        pending id is never admitted twice, which also bounds queued plus running
        jobs by ``maxInFlight``.
        """
        if not (
            isinstance(message, dict)
            and message.get("jsonrpc") == "2.0"
            and isinstance(message.get("method"), str)
            and message["method"] in BY_METHOD
            and is_request_id(message.get("id"))
        ):
            return _Incoming(message)
        msg_id = message["id"]
        with self._events_lock:
            if msg_id in self._events:
                return _Incoming(message, duplicate=True)
            # The host keeps at most maxInFlight requests pending. A peer that sends
            # more waits here, with the rest of its input unread, until a slot
            # frees. The contract has no busy error to answer with.
            self._events_lock.wait_for(lambda: len(self._events) < self.max_in_flight)
            self._events[msg_id] = threading.Event()
        return _Incoming(message, reserved=True)

    # Main thread --------------------------------------------------------------

    def _handle(
        self, message: Any, reserved: bool, state: str, pool: ThreadPoolExecutor
    ) -> str | int:
        """Process one message. Returns the next state, or an exit status."""
        if not isinstance(message, dict) or message.get("jsonrpc") != "2.0":
            self._respond_error(
                _safe_id(message),
                RpcError(INVALID_REQUEST, "not a JSON-RPC 2.0 message"),
            )
            return state
        method = message.get("method")
        if not isinstance(method, str) or not method:
            if "result" not in message and "error" not in message:
                self._respond_error(
                    _safe_id(message), RpcError(INVALID_REQUEST, "no method")
                )
            return state  # the host sent a response; v1 plugins send no requests
        is_request = "id" in message
        msg_id = message.get("id")
        if is_request and not is_request_id(msg_id):
            self._respond_error(None, RpcError(INVALID_REQUEST, "invalid request id"))
            return state
        if not is_request:
            return self._notification(method, state)
        try:
            return self._request(msg_id, method, message.get("params"), state, pool)
        except RpcError as exc:
            self._respond_error(msg_id, exc)
            if reserved:
                self._drop(msg_id)
            return state

    def _notification(self, method: str, state: str) -> str | int:
        if method == "exit":
            return 0 if state == "shutdown" else 1
        return state

    def _request(
        self,
        msg_id: int,
        method: str,
        params: Any,
        state: str,
        pool: ThreadPoolExecutor,
    ) -> str:
        if method == "initialize":
            if state != "new":
                raise RpcError(INVALID_REQUEST, "already initialized")
            self._respond(msg_id, self._initialize(params))
            return "ready"
        if state == "new":
            raise RpcError(NOT_INITIALIZED, "send initialize first")
        if state == "shutdown":
            raise RpcError(INVALID_REQUEST, "the plugin is shut down")
        if method == "shutdown":
            self._respond(msg_id, None)
            return "shutdown"
        kind = BY_METHOD.get(method)
        if kind is None or not any(k == kind.kind for k, _ in self._hooks):
            raise RpcError(METHOD_NOT_FOUND, f"unknown method: {method}")
        pool.submit(self._execute, msg_id, kind, params)
        return state

    def _initialize(self, params: Any) -> dict[str, Any]:
        if not isinstance(params, dict):
            raise RpcError(INVALID_PARAMS, "params must be an object")
        highest = params.get("protocolVersion")
        lowest = params.get("minProtocolVersion", 1)
        caps, limits = params.get("capabilities"), params.get("limits")
        if not (is_int(highest) and is_int(lowest) and highest >= 1 and lowest >= 1):
            raise RpcError(INVALID_PARAMS, "protocol versions must be integers from 1")
        if not (isinstance(caps, dict) and isinstance(caps.get("features"), list)):
            raise RpcError(INVALID_PARAMS, "capabilities.features must be a list")
        if not (
            isinstance(limits, dict)
            and is_int(limits.get("maxMessageBytes"))
            and limits["maxMessageBytes"] >= framing.MIN_MESSAGE_BYTES
            and is_int(limits.get("maxBatchDocuments"))
            and limits["maxBatchDocuments"] >= 1
        ):
            raise RpcError(INVALID_PARAMS, "limits are missing or below the floor")
        if not isinstance(params.get("offline"), bool):
            raise RpcError(INVALID_PARAMS, "offline must be a boolean")
        if highest < PROTOCOL_MIN or lowest > PROTOCOL_MAX:
            raise RpcError(
                PROTOCOL_VERSION_UNSUPPORTED,
                "no common protocol version",
                {"supported": {"min": PROTOCOL_MIN, "max": PROTOCOL_MAX}},
            )
        offered = {f for f in caps["features"] if isinstance(f, str)}
        for feature in self.required_features:
            if feature not in offered:
                raise RpcError(
                    REQUIRED_FEATURE_MISSING,
                    f"the host lacks the required feature {feature}",
                    {"feature": feature},
                )
        self._offline = params["offline"]
        self._granted = frozenset(
            f
            for f in (*self.required_features, *self.optional_features)
            if f in offered
        )
        self._limit = min(limits["maxMessageBytes"], self.max_message_bytes)
        self._batch = min(limits["maxBatchDocuments"], self.max_batch_documents)
        return {
            "protocolVersion": min(highest, PROTOCOL_MAX),
            "pluginInfo": {"name": self.name, "version": self.version},
            "capabilities": {
                "hooks": [{"kind": k, "id": i} for k, i in self._hooks],
                "features": {
                    "required": list(self.required_features),
                    "optional": list(self.optional_features),
                },
                "requiresNetwork": self.network,
            },
            "limits": {
                "maxMessageBytes": self._limit,
                "maxBatchDocuments": self._batch,
                "maxInFlight": self.max_in_flight,
            },
        }

    def _execute(self, msg_id: int, kind: HookKind, params: Any) -> None:
        with self._events_lock:
            event = self._events.setdefault(msg_id, threading.Event())
        ctx = Context(msg_id, self._offline, self._granted, event)
        try:
            if not isinstance(params, dict):
                raise RpcError(INVALID_PARAMS, "params must be an object")
            hook_id = params.get(kind.kind)
            fn = (
                self._hooks.get((kind.kind, hook_id))
                if isinstance(hook_id, str)
                else None
            )
            if fn is None:
                raise RpcError(INVALID_PARAMS, f"unknown {kind.kind} id")
            self._respond(msg_id, kind.run(fn, params, ctx, self._batch))
        except RpcError as exc:
            self._respond_error(msg_id, exc)
        except OutputInvalid as exc:
            log.error("refusing to send invalid output: %s", exc)
            self._respond_error(
                msg_id, RpcError(INTERNAL_ERROR, "the hook produced invalid output")
            )
        except Exception as exc:  # noqa: BLE001 - a hook bug must not kill the plugin
            # Log the location only: an exception message may quote document text.
            frame = traceback.extract_tb(exc.__traceback__)[-1]
            log.error(
                "hook failed: %s at %s:%d",
                type(exc).__name__,
                frame.filename,
                frame.lineno,
            )
            self._respond_error(msg_id, RpcError(INTERNAL_ERROR, "the hook failed"))
        finally:
            self._drop(msg_id)

    def _drop(self, msg_id: int) -> None:
        with self._events_lock:
            self._events.pop(msg_id, None)
            self._events_lock.notify_all()

    # Writing --------------------------------------------------------------------

    def _respond(self, msg_id: int, result: Any) -> None:
        self._send({"jsonrpc": "2.0", "id": msg_id, "result": result}, msg_id)

    def _respond_error(self, msg_id: int | None, error: RpcError) -> None:
        self._send({"jsonrpc": "2.0", "id": msg_id, "error": error.to_wire()}, msg_id)

    def _send(self, message: dict[str, Any], msg_id: int | None) -> None:
        body = framing.dumps(message)
        if len(body) > self._limit:
            too_large = RpcError(
                MESSAGE_TOO_LARGE, f"the reply exceeds {self._limit} bytes"
            )
            body = framing.dumps(
                {"jsonrpc": "2.0", "id": msg_id, "error": too_large.to_wire()}
            )
        assert self._out is not None
        with self._write_lock:
            try:
                self._out.write(framing.encode_frame(body))
                self._out.flush()
            except (OSError, ValueError):
                log.error("the host closed the pipe")


def _safe_id(message: Any) -> int | None:
    if isinstance(message, dict) and is_request_id(message.get("id")):
        return message["id"]
    return None
