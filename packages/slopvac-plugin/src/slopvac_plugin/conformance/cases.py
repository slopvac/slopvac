"""Plugin conformance cases P1 to P19 (docs/contract/plugin-protocol.md, section 12.2).

The checks here do not import the SDK's validators. The suite is the oracle, so it
carries its own short copies of the span and edit rules.
"""

from __future__ import annotations

import contextlib
import itertools
from collections.abc import Callable, Iterator, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from .samples import DOCUMENT_KINDS, params_for
from .schemas import MAX_UNIQUE_ITEMS, Schemas
from .session import CaseFailure, CaseSkip, Session, serialise

ALL_KINDS = ("tagger", "extractor", "reporter", "fixer", "scorer", "provider", "recipe")
RESULT_DEFS = {
    "tagger": "TagResult",
    "extractor": "ExtractResult",
    "reporter": "RenderResult",
    "fixer": "FixResult",
    "scorer": "ScoreResult",
    "provider": "CompleteResult",
    "recipe": "RecipeResult",
}
METHODS = {
    "tagger": "tagger/tag",
    "extractor": "extractor/extract",
    "reporter": "reporter/render",
    "fixer": "fixer/fix",
    "scorer": "scorer/score",
    "provider": "provider/complete",
    "recipe": "recipe/plan",
}
MAX_MESSAGE_BYTES = 16 * 1024 * 1024
MAX_IN_FLIGHT = 16  # section 6.1: the host ceiling
FLOOR = 65536
PRE_INITIALIZE_LIMIT = 1024 * 1024


@dataclass
class Env:
    command: Sequence[str]
    schemas: Schemas
    timeout: float = 10.0
    exit_timeout: float = 2.0
    cancel_grace: float = 2.0  # section 6.3: an answer this long after the cancel
    pass_env: Sequence[str] = ()  # extra environment names the plugin may see
    inputs: dict[str, Any] = field(default_factory=dict)
    declaration: dict[str, Any] | None = None
    hooks: list[tuple[str, str]] = field(default_factory=list)
    # The wall-clock limit (time.monotonic) of the running case; the runner sets it.
    deadline: float | None = None

    def spawn(self) -> Session:
        return Session(
            self.command,
            self.schemas,
            self.timeout,
            env_names=self.pass_env,
            deadline=self.deadline,
        )

    @contextlib.contextmanager
    def session(self) -> Iterator[Session]:
        s = self.spawn()
        try:
            yield s
        finally:
            s.close()

    @contextlib.contextmanager
    def ready(self, **options: Any) -> Iterator[tuple[Session, dict[str, Any]]]:
        """A session after the handshake: initialize, reply checked, initialized sent."""
        with self.session() as s:
            yield s, handshake(s, **options)

    def callable_hooks(self) -> list[tuple[str, str]]:
        """Hooks the suite may call. A provider may reach a model, so it needs inputs."""
        return [
            (k, i)
            for k, i in self.hooks
            if k != "provider" or "provider" in self.inputs
        ]

    def params(self, kind: str, hook_id: str) -> dict[str, Any]:
        return params_for(kind, hook_id, self.inputs)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise CaseFailure(message)


def host_params(
    *,
    offline: bool = False,
    highest: int = 2,
    lowest: int = 1,
    max_message_bytes: int = MAX_MESSAGE_BYTES,
    extras: bool = False,
) -> dict[str, Any]:
    params: dict[str, Any] = {
        "protocolVersion": highest,
        "minProtocolVersion": lowest,
        "hostInfo": {"name": "slopvac-plugin-conformance", "version": "1"},
        "capabilities": {"hooks": list(ALL_KINDS), "features": ["document-path"]},
        "offline": offline,
        "limits": {"maxMessageBytes": max_message_bytes, "maxBatchDocuments": 64},
    }
    if extras:
        params["capabilities"]["hooks"] += ["hologram", "x-private-kind"]
        params["capabilities"]["features"] += ["x-private", "future-feature"]
        params["futureField"] = {"nested": [1, 2, {"x-deep": True}]}
        params["x-extra"] = "ignored"
    return params


def error_of(response: dict[str, Any], code: int, slug: str) -> dict[str, Any]:
    error = response.get("error")
    require(error is not None, f"expected error {code} {slug}, got {response}")
    require(
        error["code"] == code and error["data"]["slopvac_code"] == slug,
        f"expected {code} {slug}, got {error['code']} {error['data']['slopvac_code']}",
    )
    return error


def result_of(response: dict[str, Any]) -> Any:
    require("error" not in response, f"unexpected error: {response.get('error')}")
    return response["result"]


def _declared_hooks(result: Any) -> list[Any]:
    """The hooks list of an initialize result, before the schema has checked its shape."""
    caps = result.get("capabilities") if isinstance(result, dict) else None
    hooks = caps.get("hooks") if isinstance(caps, dict) else None
    return hooks if isinstance(hooks, list) else []


def handshake(s: Session, **options: Any) -> dict[str, Any]:
    reply = s.request("initialize", host_params(**options))
    result = result_of(reply)
    hooks = _declared_hooks(result)
    require(
        len(hooks) <= MAX_UNIQUE_ITEMS,
        f"the plugin declares {len(hooks)} hooks, more than the limit of "
        f"{MAX_UNIQUE_ITEMS}",
    )
    problems = s.schemas.errors("InitializeResult", result)
    require(not problems, f"initialize result breaks InitializeResult: {problems}")
    limits = result["limits"]
    s.max_documents = limits["maxBatchDocuments"]
    s.max_body_bytes = min(
        limits["maxMessageBytes"], options.get("max_message_bytes", MAX_MESSAGE_BYTES)
    )
    s.notify("initialized", {})
    return result


def span_problem(data: bytes, span: dict[str, Any]) -> str | None:
    start, end = span["start"], span["end"]
    if not 0 <= start <= end <= len(data):
        return f"[{start}, {end}) lies outside {len(data)} bytes"
    for offset in (start, end):
        if offset < len(data) and (data[offset] & 0xC0) == 0x80:
            return f"offset {offset} is inside a multi-byte character"
        if 0 < offset < len(data) and data[offset - 1 : offset + 1] == b"\r\n":
            return f"offset {offset} splits a CRLF"
    return None


def check_spans(data: bytes, spans: Sequence[dict[str, Any]], what: str) -> None:
    for span in spans:
        problem = span_problem(data, span)
        require(problem is None, f"{what}: {problem}")


# Per-document limits on a fixer reply (threat model section 5.5, TM-14).
MAX_EDITS = 1000
MAX_INSERTED_BYTES = 1024 * 1024


def check_edits(doc: dict[str, Any], edits: Sequence[dict[str, Any]]) -> None:
    require(
        len(edits) <= MAX_EDITS,
        f"{len(edits)} edits is more than the limit of {MAX_EDITS}",
    )
    findings = doc.get("findings", [])
    inserted = 0
    for edit in edits:
        require(
            edit["finding"] < len(findings),
            "edit names a finding that was not selected",
        )
        outer, span = findings[edit["finding"]]["span"], edit["span"]
        require(
            outer["start"] <= span["start"] and span["end"] <= outer["end"],
            "edit lies outside the span of its finding",
        )
        inserted += len(edit["text"].encode("utf-8", "surrogatepass"))
    spanned = sum(f["span"]["end"] - f["span"]["start"] for f in findings)
    require(
        inserted <= min(MAX_INSERTED_BYTES, 4 * spanned + 4096),
        "edits insert more bytes than the limit allows",
    )
    # Sort and sweep (positions.md, Edit). Zero-width edits at one offset are legal.
    reach = 0
    for start, end in sorted((e["span"]["start"], e["span"]["end"]) for e in edits):
        require(start >= reach, "two edits overlap")
        reach = max(reach, end)


def check_result(env: Env, kind: str, params: dict[str, Any], result: Any) -> None:
    """P15: the result matches its schema def, and spans and ids are sound."""
    problems = env.schemas.errors(RESULT_DEFS[kind], result)
    require(not problems, f"{kind} result breaks {RESULT_DEFS[kind]}: {problems}")
    if kind == "provider":
        got = sorted(r["item"] for r in result["results"])
        require(got == sorted(i["id"] for i in params["items"]), "one entry per item")
        for entry in result["results"]:
            require(
                "error" in entry or entry.get("model"), "entry needs text and model"
            )
        return
    if kind not in DOCUMENT_KINDS:
        return
    docs = {d["id"]: d for d in params["documents"]}
    got_ids = [e["document"] for e in result["results"]]
    require(sorted(got_ids) == sorted(docs), "exactly one entry per document")
    for entry in result["results"]:
        if "error" in entry:
            continue
        doc = docs[entry["document"]]
        data = doc["text"].encode("utf-8")
        if kind == "tagger":
            check_spans(data, [t["span"] for t in entry.get("tags", [])], "tag span")
        elif kind == "extractor":
            segments = entry.get("segments", [])
            check_spans(data, [s["span"] for s in segments], "segment span")
            ends = [s["span"] for s in segments]
            for a, b in itertools.pairwise(ends):
                require(a["end"] <= b["start"], "segments overlap or are out of order")
        elif kind == "fixer":
            edits = entry.get("edits", [])
            check_spans(data, [e["span"] for e in edits], "edit span")
            check_edits(doc, edits)


def request_size(s: Session, method: str, params: dict[str, Any]) -> int:
    """Bytes of the body the session would send, measured with its own serialiser."""
    message = {"jsonrpc": "2.0", "id": s.next_id, "method": method, "params": params}
    return len(serialise(message))


def fits(s: Session, method: str, params: dict[str, Any]) -> bool:
    """Whether one request stays inside the limits the plugin negotiated."""
    documents = params.get("documents", [])
    return (
        len(documents) <= s.max_documents
        and request_size(s, method, params) <= s.max_body_bytes
    )


def batches(s: Session, kind: str, params: dict[str, Any]) -> list[dict[str, Any]]:
    """Split a request into requests that fit the negotiated limits.

    The host respects maxBatchDocuments and maxMessageBytes (sections 2.2 and 5.3), so
    the suite does too. When a document cannot fit alone, the sample input is unsuitable
    for this plugin: the case skips and no oversized frame is written.
    """
    method = METHODS[kind]
    documents = params.get("documents")
    if not documents:
        if not fits(s, method, params):
            raise CaseSkip(
                f"the {kind} request is larger than maxMessageBytes {s.max_body_bytes}"
            )
        return [params]
    out: list[dict[str, Any]] = []
    current: list[dict[str, Any]] = []
    for document in documents:
        trial = {**params, "documents": [*current, document]}
        if current and not fits(s, method, trial):
            out.append({**params, "documents": current})
            current = []
        current.append(document)
        if not fits(s, method, {**params, "documents": current}):
            raise CaseSkip(
                f"document {document.get('id')!r} alone exceeds the negotiated limits "
                f"(maxMessageBytes {s.max_body_bytes}, maxBatchDocuments "
                f"{s.max_documents}): unsuitable suite input"
            )
    out.append({**params, "documents": current})
    return out


def send_batches(
    s: Session, kind: str, params: dict[str, Any]
) -> list[tuple[dict[str, Any], Any]]:
    """Send a request in as many batches as the limits need: (batch, result) pairs.

    Every batch is planned before the first is sent, so a request that cannot be
    split to fit skips before it writes anything.
    """
    return [
        (part, result_of(s.request(METHODS[kind], part)))
        for part in batches(s, kind, params)
    ]


def send_hook(env: Env, s: Session, kind: str, params: dict[str, Any]) -> Any:
    """Send a hook request; check each reply against its own batch, then join them."""
    pairs = send_batches(s, kind, params)
    for part, result in pairs:
        check_result(env, kind, part, result)
    joined = pairs[0][1]
    for _, result in pairs[1:]:
        joined = {**joined, "results": [*joined["results"], *result["results"]]}
    return joined


def call_hook(
    env: Env, s: Session, kind: str, hook_id: str, **extra: Any
) -> tuple[dict, Any]:
    params = {**env.params(kind, hook_id), **extra}
    return params, send_hook(env, s, kind, params)


def sentinel(s: Session) -> None:
    """Nothing else was sent: the next frame answers a request sent after the probe."""
    error_of(s.request("x-conformance/sentinel"), -32601, "method-not-found")


def need_hooks(
    env: Env, kinds: Sequence[str] = ALL_KINDS, *, callable_only: bool = True
) -> list[tuple[str, str]]:
    pool = env.callable_hooks() if callable_only else env.hooks
    chosen = [(k, i) for k, i in pool if k in kinds]
    if not chosen:
        raise CaseSkip("the plugin declares no matching hook")
    return chosen


Case = Callable[[Env], None]
CASES: dict[str, tuple[str, Case]] = {}


def case(case_id: str, title: str) -> Callable[[Case], Case]:
    def register(fn: Case) -> Case:
        CASES[case_id] = (title, fn)
        return fn

    return register


@case("P1", "initialize reply lists hooks, features and limits")
def p1(env: Env) -> None:
    with env.ready(highest=2, lowest=1) as (_, result):
        require(result["protocolVersion"] == 1, "negotiated version must be 1")
        limits = result["limits"]
        require(
            limits["maxMessageBytes"] <= MAX_MESSAGE_BYTES,
            "limit raised above the host's",
        )
        require(
            limits["maxBatchDocuments"] <= 64, "batch limit raised above the host's"
        )
        require(
            limits["maxInFlight"] <= MAX_IN_FLIGHT,
            f"maxInFlight is above the host ceiling of {MAX_IN_FLIGHT}",
        )


@case("P2", "a version range without 1 gets protocol-version-unsupported")
def p2(env: Env) -> None:
    with env.session() as s:
        reply = s.request("initialize", host_params(highest=3, lowest=2))
        error = error_of(reply, -32001, "protocol-version-unsupported")
        require("supported" in error["data"], "data.supported is missing")


@case("P3", "a request before initialize gets not-initialized")
def p3(env: Env) -> None:
    with env.session() as s:
        reply = s.request(
            "tagger/tag", {"tagger": "x", "documents": [{"id": "a", "text": "t"}]}
        )
        error_of(reply, -32003, "not-initialized")


@case("P4", "an unknown method: error for a request, silence for a notification")
def p4(env: Env) -> None:
    with env.ready() as (s, _):
        error_of(s.request("x-conformance/unknown"), -32601, "method-not-found")
        error_of(s.request("$/unknownRequest"), -32601, "method-not-found")
        s.notify("x-conformance/unknown", {})
        s.notify("$/unknownNotification", {})
        sentinel(s)


@case("P5", "unknown fields, x- keys and unknown hook kinds are ignored")
def p5(env: Env) -> None:
    with env.ready(extras=True) as (s, result):
        declared = {(h["kind"], h["id"]) for h in result["capabilities"]["hooks"]}
        for kind, hook_id in [h for h in env.callable_hooks() if h in declared]:
            params = env.params(kind, hook_id)
            params["x-extra"] = {"a": 1}
            params["futureField"] = [1, "two"]
            for doc in params.get("documents", []):
                doc["x-doc"] = True
                doc["futureDocField"] = None
            send_hook(env, s, kind, params)


@case(
    "P6",
    "an undeclared hook id gets invalid-params, an undeclared kind method-not-found",
)
def p6(env: Env) -> None:
    if not env.hooks:
        raise CaseSkip("the plugin declares no hook")
    declared = {k for k, _ in env.hooks}
    with env.ready() as (s, _):
        for kind, _id in env.hooks:
            params = env.params(kind, "no-such-hook")
            error_of(s.request(METHODS[kind], params), -32602, "invalid-params")
        for kind in ALL_KINDS:
            if kind not in declared:
                error_of(
                    s.request(METHODS[kind], {kind: "x"}), -32601, "method-not-found"
                )


@case("P7", "an invalid document fails alone")
def p7(env: Env) -> None:
    kinds = [k for k, _ in env.callable_hooks() if k in DOCUMENT_KINDS]
    todo = [
        (k, i)
        for k, i in env.callable_hooks()
        if k in DOCUMENT_KINDS and "invalid_document" in env.inputs.get(k, {})
    ]
    if not todo:
        detail = "give inputs.<kind>.invalid_document" if kinds else "no document hook"
        raise CaseSkip(f"needs an invalid sample document: {detail}")
    with env.ready() as (s, _):
        mixed: list[tuple[str, dict[str, Any]]] = []
        for kind, hook_id in todo:
            params = env.params(kind, hook_id)
            bad = env.inputs[kind]["invalid_document"]
            params["documents"] = [params["documents"][0], bad]
            if fits(s, METHODS[kind], params):
                mixed.append((kind, params))
        if not mixed:
            raise CaseSkip(
                "the negotiated limits (maxBatchDocuments "
                f"{s.max_documents}, maxMessageBytes {s.max_body_bytes}) "
                "cannot hold a valid and an invalid document in one request"
            )
        for kind, params in mixed:
            bad = env.inputs[kind]["invalid_document"]
            result = result_of(s.request(METHODS[kind], params))
            check_result(env, kind, params, result)
            by_id = {e["document"]: e for e in result["results"]}
            require(
                "error" in by_id[bad["id"]], "the invalid document has no error entry"
            )
            require(
                "error" not in by_id[params["documents"][0]["id"]],
                "the valid document failed",
            )


@case("P8", "the same request twice gives equal results")
def p8(env: Env) -> None:
    hooks = [h for h in need_hooks(env) if h[0] != "provider"]
    if not hooks:
        raise CaseSkip(
            "only a provider is declared, and a provider may differ between calls"
        )
    with env.ready() as (s, _):
        for kind, hook_id in hooks:
            _, first = call_hook(env, s, kind, hook_id)
            _, second = call_hook(env, s, kind, hook_id)
            require(first == second, f"{kind}/{hook_id} gave different results")


@case("P9", "cancel gets an answer, a cancel for an unknown id gets silence")
def p9(env: Env) -> None:
    hooks = env.callable_hooks()
    with env.ready() as (s, _):
        if hooks:
            kind, hook_id = hooks[0]
            first = batches(s, kind, env.params(kind, hook_id))[0]
            request_id = s.start_request(METHODS[kind], first)
            # One deadline from before the cancel write covers write and response.
            response = s.cancel_and_wait(request_id, env.cancel_grace)
            if "error" in response:
                error_of(response, -32005, "request-cancelled")
        s.notify("$/cancelRequest", {"id": 987654})
        sentinel(s)


@case("P10", "shutdown, then a request, then exit: null, invalid-request, status 0")
def p10(env: Env) -> None:
    with env.ready() as (s, _):
        reply = s.request("shutdown")
        require(
            reply.get("result", 1) is None and "result" in reply,
            "shutdown must answer null",
        )
        error_of(s.request("x-conformance/late"), -32600, "invalid-request")
        s.notify("exit")
        require(s.wait(env.exit_timeout) == 0, "exit after shutdown must give status 0")


@case("P11", "exit without shutdown gives status 1")
def p11(env: Env) -> None:
    with env.ready() as (s, _):
        s.notify("exit")
        status = s.wait(env.exit_timeout)
        require(status == 1, f"exit without shutdown gave status {status}, not 1")


@case("P12", "end of input ends the process at once")
def p12(env: Env) -> None:
    with env.ready() as (s, _):
        s.close_stdin()
        s.wait(env.exit_timeout)


def expect_rejected_at_the_header(env: Env, s: Session, length: int) -> None:
    """Send a header that announces ``length`` bytes and no body.

    A plugin that rejects on the header exits 1 and prints nothing. One that waits for
    the body keeps running, so the exit check fails it. A valid body would not help
    it: the contract says the body is not read.
    """
    s.send_bytes(b"Content-Length: %d\r\n\r\n" % length, tolerate_closed=True)
    status = s.wait(env.exit_timeout)
    require(status == 1, f"an oversized body gave status {status}, not 1")
    require(
        all(kind == "eof" for kind, _ in s.drain_events()),
        "the plugin printed output for an oversized body",
    )


@case("P13", "a body over the limit is not read and the plugin exits with status 1")
def p13(env: Env) -> None:
    with env.ready(max_message_bytes=FLOOR) as (s, result):
        limit = result["limits"]["maxMessageBytes"]
        require(limit <= FLOOR, f"the reply raised maxMessageBytes to {limit}")
        expect_rejected_at_the_header(env, s, limit + 1)
    with env.session() as s:
        expect_rejected_at_the_header(env, s, PRE_INITIALIZE_LIMIT + 1)


@case("P14", "standard output holds frames only")
def p14(env: Env) -> None:
    with env.ready() as (s, _):
        for kind, hook_id in env.callable_hooks():
            call_hook(env, s, kind, hook_id)
        require("result" in s.request("shutdown"), "shutdown failed")
        s.notify("exit")
        s.wait(env.exit_timeout)
        events = s.drain_events()
        require(
            all(kind == "eof" for kind, _ in events),
            f"standard output holds more than frames: {events[:1]}",
        )


@case("P15", "every result matches its schema, with spans in range")
def p15(env: Env) -> None:
    hooks = need_hooks(env)
    with env.ready() as (s, _):
        for kind, hook_id in hooks:
            call_hook(env, s, kind, hook_id)  # checks each batch against its request


@case("P16", "a provider refuses with offline when offline mode is on")
def p16(env: Env) -> None:
    providers = [(k, i) for k, i in env.hooks if k == "provider"]
    if not providers:
        raise CaseSkip("the plugin declares no provider")
    with env.ready(offline=True) as (s, _):
        for kind, hook_id in providers:
            reply = s.request(METHODS[kind], env.params(kind, hook_id))
            error_of(reply, -32007, "offline")
    # The suite cannot observe sockets: "opens no connection" is the plugin's promise.


def _snapshot(root: Path) -> set[tuple[str, int, int]]:
    skip = {"__pycache__", ".git", ".venv", "node_modules"}
    seen: set[tuple[str, int, int]] = set()
    for path in root.rglob("*"):
        if len(seen) > 5000:
            break
        if skip & set(path.relative_to(root).parts) or path.suffix == ".pyc":
            continue
        with contextlib.suppress(OSError):
            stat = path.stat()
            seen.add((str(path.relative_to(root)), stat.st_size, stat.st_mtime_ns))
    return seen


@case("P17", "a recipe plan holds relative paths and the plugin writes nothing")
def p17(env: Env) -> None:
    hooks = need_hooks(env, ("recipe",))
    with env.ready() as (s, _):
        before = _snapshot(Path.cwd())
        for kind, hook_id in hooks:
            call_hook(env, s, kind, hook_id)  # checks each batch against its request
        require(
            _snapshot(Path.cwd()) == before,
            "the working directory changed during recipe/plan",
        )


@case("P18", "every fixer edit names a finding and lies inside its span")
def p18(env: Env) -> None:
    hooks = need_hooks(env, ("fixer",))
    with env.ready() as (s, _):
        for kind, hook_id in hooks:
            call_hook(env, s, kind, hook_id)  # checks each batch, edit rules included


@case("P19", "hooks, network and features are a subset of the manifest declaration")
def p19(env: Env) -> None:
    declaration = env.declaration
    if declaration is None:
        raise CaseSkip("give --manifest to compare the reply with the declaration")
    problems = env.schemas.errors("PluginDeclaration", declaration)
    require(not problems, f"the declaration breaks PluginDeclaration: {problems}")
    with env.ready() as (_, result):
        caps = result["capabilities"]
        require(
            caps["requiresNetwork"] == declaration["network"],
            "requiresNetwork differs from the declared network",
        )
        declared = {(h["kind"], h["id"]) for h in declaration["hooks"]}
        extra = {(h["kind"], h["id"]) for h in caps["hooks"]} - declared
        require(not extra, f"hooks missing from the declaration: {sorted(extra)}")
        for level in ("required", "optional"):
            extra_features = set(caps["features"][level]) - set(
                declaration["features"][level]
            )
            require(
                not extra_features,
                f"{level} features missing from the declaration: {sorted(extra_features)}",
            )
