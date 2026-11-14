"""Hook kinds: params parsing, handler calls, and result building (sections 5.1 to 5.4)."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

from . import validate
from .errors import (
    INVALID_PARAMS,
    OFFLINE,
    DocumentInvalid,
    OutputInvalid,
    RpcError,
)
from .types import (
    Completion,
    Context,
    Document,
    Edit,
    Finding,
    Item,
    RecipeFile,
    Rendered,
    Segment,
    Span,
    Tag,
)

TASKS = ("fact-check", "update", "rewrite")
Result = dict[str, Any]


def bad_params(message: str) -> RpcError:
    return RpcError(INVALID_PARAMS, message)


def _obj(value: Any, what: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise bad_params(f"{what} must be an object")
    return value


def _str(value: Any, what: str, *, nonempty: bool = False) -> str:
    if not isinstance(value, str) or (nonempty and not value):
        raise bad_params(f"{what} must be a {'non-empty ' if nonempty else ''}string")
    return value


def _span(raw: Any, what: str) -> Span:
    raw = _obj(raw, what)
    start, end = raw.get("start"), raw.get("end")
    if not (validate.is_int(start) and validate.is_int(end)):
        raise bad_params(f"{what} offsets must be non-negative integers")
    return Span(start, end)


def parse_document(raw: Any) -> Document:
    raw = _obj(raw, "document")
    text = _str(raw.get("text"), "document text")
    try:
        text.encode("utf-8")
    except UnicodeEncodeError as exc:
        raise bad_params("document text is not valid Unicode") from exc
    media_type, path = raw.get("media_type"), raw.get("path")
    for name, value in (("media_type", media_type), ("path", path)):
        if value is not None:
            _str(value, f"document {name}")
    return Document(
        _str(raw.get("id"), "document id", nonempty=True), text, media_type, path
    )


def parse_finding(raw: Any, document: Document) -> Finding:
    raw = _obj(raw, "finding")
    span = _span(raw.get("span"), "finding span")
    if span.start > span.end or span.end > len(document.data):
        raise bad_params("finding span lies outside the document")
    return Finding(
        _str(raw.get("rule"), "finding rule"),
        _str(raw.get("severity"), "finding severity"),
        span,
        _str(raw.get("message"), "finding message"),
    )


def _documents(
    params: dict[str, Any], max_batch: int, *, findings: bool
) -> list[tuple[Document, list[Finding]]]:
    raw = params.get("documents")
    if not isinstance(raw, list) or not 1 <= len(raw) <= max_batch:
        raise bad_params(f"documents must be a list of 1 to {max_batch} entries")
    out: list[tuple[Document, list[Finding]]] = []
    seen: set[str] = set()
    for entry in raw:
        document = parse_document(entry)
        if document.id in seen:
            raise bad_params("document ids must be unique")
        seen.add(document.id)
        selected: list[Finding] = []
        if findings:
            raw_findings = entry.get("findings")
            if not isinstance(raw_findings, list):
                raise bad_params("document findings must be a list")
            selected = [parse_finding(f, document) for f in raw_findings]
        out.append((document, selected))
    return out


def _per_document(
    params: dict[str, Any],
    ctx: Context,
    max_batch: int,
    *,
    findings: bool,
    produce: Callable[[Document, list[Finding]], Result],
) -> Result:
    entries: list[Result] = []
    for document, selected in _documents(params, max_batch, findings=findings):
        ctx.raise_if_cancelled()
        try:
            entries.append({"document": document.id, **produce(document, selected)})
        except DocumentInvalid as exc:
            entries.append({"document": document.id, "error": exc.to_wire()})
    return {"results": entries}


def _list(value: Any, what: str) -> list[Any]:
    if value is None:
        return []
    try:
        return list(value)
    except TypeError as exc:
        raise OutputInvalid(f"{what} must be an iterable") from exc


def _expect(items: list[Any], cls: type, what: str) -> None:
    for item in items:
        if not isinstance(item, cls):
            raise OutputInvalid(f"{what} must be {cls.__name__} values")


def _wire_span(span: Span) -> dict[str, int]:
    return {"start": span.start, "end": span.end}


Run = Callable[[Callable[..., Any], dict[str, Any], Context, int], Result]


def tagger(
    fn: Callable[..., Any], params: dict[str, Any], ctx: Context, batch: int
) -> Result:
    def produce(doc: Document, _findings: list[Finding]) -> Result:
        tags = _list(fn(doc, ctx), "tags")
        _expect(tags, Tag, "tags")
        for tag in tags:
            validate.check_tag(doc.data, tag)
        return {
            "tags": [
                {
                    "kind": t.kind,
                    "span": _wire_span(t.span),
                    "confidence": t.confidence,
                    "evidence": t.evidence,
                }
                for t in tags
            ]
        }

    return _per_document(params, ctx, batch, findings=False, produce=produce)


def extractor(
    fn: Callable[..., Any], params: dict[str, Any], ctx: Context, batch: int
) -> Result:
    def produce(doc: Document, _findings: list[Finding]) -> Result:
        segments = _list(fn(doc, ctx), "segments")
        _expect(segments, Segment, "segments")
        validate.check_segments(doc.data, segments)
        wire = []
        for s in segments:
            item: Result = {"span": _wire_span(s.span), "context": s.context}
            if s.text is not None:
                item["text"] = s.text
            wire.append(item)
        return {"segments": wire}

    return _per_document(params, ctx, batch, findings=False, produce=produce)


def fixer(
    fn: Callable[..., Any], params: dict[str, Any], ctx: Context, batch: int
) -> Result:
    def produce(doc: Document, selected: list[Finding]) -> Result:
        edits = _list(fn(doc, selected, ctx), "edits")
        _expect(edits, Edit, "edits")
        validate.check_edits(doc.data, selected, edits)
        return {
            "edits": [
                {"finding": e.finding, "span": _wire_span(e.span), "text": e.text}
                for e in edits
            ]
        }

    return _per_document(params, ctx, batch, findings=True, produce=produce)


def scorer(
    fn: Callable[..., Any], params: dict[str, Any], ctx: Context, batch: int
) -> Result:
    def produce(doc: Document, selected: list[Finding]) -> Result:
        return {"score": validate.check_score(fn(doc, selected, ctx))}

    return _per_document(params, ctx, batch, findings=True, produce=produce)


def reporter(
    fn: Callable[..., Any], params: dict[str, Any], ctx: Context, batch: int
) -> Result:
    report = _obj(params.get("report"), "report")
    version = report.get("schema_version")
    if type(version) is not int or version != 3:
        raise bad_params("report.schema_version must be 3")
    options = _obj(params.get("options", {}), "options")
    rendered = fn(report, options, ctx)
    if not isinstance(rendered, Rendered) or not isinstance(rendered.output, str):
        raise OutputInvalid("a reporter returns Rendered(output, media_type)")
    return {"output": rendered.output, "media_type": rendered.media_type}


def provider(
    fn: Callable[..., Any], params: dict[str, Any], ctx: Context, batch: int
) -> Result:
    task = params.get("task")
    if task not in TASKS:
        raise bad_params(f"task must be one of {TASKS}")
    model = _str(params.get("model"), "model", nonempty=True)
    raw_items = params.get("items")
    if not isinstance(raw_items, list) or not raw_items:
        raise bad_params("items must be a non-empty list")
    items: list[Item] = []
    for entry in raw_items:
        raw = _obj(entry, "item")
        instruction = raw.get("instruction")
        if instruction is not None:
            _str(instruction, "item instruction")
        items.append(
            Item(
                _str(raw.get("id"), "item id", nonempty=True),
                _str(raw.get("text"), "item text"),
                instruction,
            )
        )
    if len({i.id for i in items}) != len(items):
        raise bad_params("item ids must be unique")
    if ctx.offline:
        raise RpcError(OFFLINE, "offline mode is on")
    results: list[Result] = []
    for item in items:
        ctx.raise_if_cancelled()
        try:
            done = fn(item, task, model, ctx)
        except DocumentInvalid as exc:
            results.append({"item": item.id, "error": exc.to_wire()})
            continue
        if not isinstance(done, Completion) or not done.model:
            raise OutputInvalid("a provider returns Completion(text, model)")
        results.append({"item": item.id, "text": done.text, "model": done.model})
    return {"results": results}


def recipe(
    fn: Callable[..., Any], params: dict[str, Any], ctx: Context, batch: int
) -> Result:
    harness = _str(params.get("harness"), "harness", nonempty=True)
    options = _obj(params.get("options", {}), "options")
    files = _list(fn(harness, options, ctx), "files")
    _expect(files, RecipeFile, "files")
    validate.check_recipe(files)
    wire = []
    for f in files:
        item: Result = {"path": f.path, "action": f.action}
        if f.content is not None:
            item["content"] = f.content
        wire.append(item)
    return {"files": wire}


@dataclass(frozen=True)
class HookKind:
    kind: str
    method: str
    run: Run


KINDS: dict[str, HookKind] = {
    k.kind: k
    for k in (
        HookKind("tagger", "tagger/tag", tagger),
        HookKind("extractor", "extractor/extract", extractor),
        HookKind("reporter", "reporter/render", reporter),
        HookKind("fixer", "fixer/fix", fixer),
        HookKind("scorer", "scorer/score", scorer),
        HookKind("provider", "provider/complete", provider),
        HookKind("recipe", "recipe/plan", recipe),
    )
}
BY_METHOD = {k.method: k for k in KINDS.values()}
