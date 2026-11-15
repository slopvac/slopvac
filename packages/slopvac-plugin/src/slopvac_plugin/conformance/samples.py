"""Request parameters the cases send. A plugin can override them per hook kind."""

from __future__ import annotations

from typing import Any

SAMPLE_TEXT = "Héllo wörld — 日本語.\r\nSecond line\n"
DOCUMENT_KINDS = ("tagger", "extractor", "fixer", "scorer")


def sample_documents(*, findings: bool) -> list[dict[str, Any]]:
    data = SAMPLE_TEXT.encode("utf-8")
    first = data.index("Héllo".encode())
    second = data.index("wörld".encode())
    docs: list[dict[str, Any]] = [
        {"id": "doc-1", "text": SAMPLE_TEXT},
        {"id": "doc-2", "text": "Plain ASCII text.\n"},
    ]
    if findings:
        docs[0]["findings"] = [
            _finding(first, first + len("Héllo".encode())),
            _finding(second, second + len("wörld".encode())),
        ]
        docs[1]["findings"] = [_finding(0, 5)]
    return docs


def sample_report() -> dict[str, Any]:
    """A complete lint report (schema_version 3) with one document and one finding."""
    counts = {"findings": 1, "errors": 0, "warnings": 1}
    strong = {**counts, "per_100_words": 10.0}
    weak = {"findings": 0, "errors": 0, "warnings": 0, "per_100_words": 0.0}
    category = {
        "category": "prose-inflation",
        **counts,
        "suggestions": 0,
        "per_100_words": 10.0,
        "score": 90.0,
    }
    finding = {
        "rule_id": "conformance:sample",
        "pack": "conformance",
        "category": "prose-inflation",
        "severity": "warning",
        "message": "sample finding",
        "span": {"start": 0, "end": 5},
        "line": 1,
        "column": 1,
        "end_line": 1,
        "end_column": 6,
        "contexts": ["prose"],
        "ai_signal": "strong",
        "matched_text": "Hello",
    }
    document = {
        "path": "sample.md",
        "profile": "normal",
        "genre": {"id": "internal", "source": "default", "reasons": []},
        "contexts_seen": ["prose"],
        "config_sources": [{"layer": "core"}],
        "words": 10,
        "sentences": 1,
        "paragraphs": 1,
        "findings": [finding],
        "categories": [
            {
                **category,
                "gating_per_100_words": 10.0,
                "budget": None,
                "over_budget": False,
            }
        ],
        "total_findings": 1,
        "errors": 0,
        "warnings": 1,
        "suggestions": 0,
        "per_100_words": 10.0,
        "score": 90.0,
        "gate": {"passed": True, "reasons": []},
        "unchecked": [],
        "ai_register": {"strong": strong, "weak": weak},
        "prose": {**counts, "per_100_words": 10.0},
    }
    return {
        "schema_version": 3,
        "slopvac_version": "3.0.0",
        "summary": {
            "documents": 1,
            "words": 10,
            "findings": 1,
            "errors": 0,
            "warnings": 1,
            "suggestions": 0,
            "per_100_words": 10.0,
            "score": 90.0,
            "passed": True,
            "categories": [category],
            "ai_register": {"strong": strong, "weak": weak},
            "prose": {**counts, "per_100_words": 10.0},
        },
        "documents": [document],
        "notices": [],
    }


def _finding(start: int, end: int) -> dict[str, Any]:
    return {
        "rule": "conformance:sample",
        "severity": "warning",
        "span": {"start": start, "end": end},
        "message": "sample finding",
    }


def params_for(kind: str, hook_id: str, inputs: dict[str, Any]) -> dict[str, Any]:
    """Default params for one hook, with the plugin's own values laid over them."""
    params: dict[str, Any]
    if kind in DOCUMENT_KINDS:
        params = {
            kind: hook_id,
            "documents": sample_documents(findings=kind in ("fixer", "scorer")),
        }
    elif kind == "reporter":
        params = {"reporter": hook_id, "report": sample_report(), "options": {}}
    elif kind == "provider":
        params = {
            "provider": hook_id,
            "task": "rewrite",
            "model": "conformance-model",
            "items": [{"id": "item-1", "text": "Some text."}],
        }
    elif kind == "recipe":
        params = {"recipe": hook_id, "harness": "conformance", "options": {}}
    else:
        raise ValueError(f"unknown hook kind: {kind}")
    override = {
        k: v for k, v in inputs.get(kind, {}).items() if k != "invalid_document"
    }
    return {**params, **override, kind: hook_id}
