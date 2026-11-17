"""The comment context tag and the `code-comments` input mode are two labels.

`ApplyMode` names an input surface, chosen by `--mode`. A match in source comments
carries the context tag `code-comment` in `Finding.contexts`. The two spellings differ on
purpose, and docs/contract/lint-report.md states the relation. No engine code emits the
tag yet: comment extraction is pending (slopvac-8l4.3.26). These tests pin the spellings
and the relation until it does.
"""

from __future__ import annotations

import json
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[2]
SCHEMA_DIR = ROOT / "schemas" / "v3"
COMMENT_TAG = "code-comment"
COMMENT_MODE = "code-comments"


def _schema(name: str) -> dict:
    return json.loads((SCHEMA_DIR / name).read_text(encoding="utf-8"))


def test_apply_mode_is_the_plural_input_surface():
    apply_mode = _schema("rule.schema.json")["$defs"]["ApplyMode"]
    assert apply_mode["enum"] == ["prose", COMMENT_MODE]


def test_comment_tag_is_a_valid_context_tag():
    common = _schema("common.schema.json")
    context_tag = {"$defs": common["$defs"], "$ref": "#/$defs/ContextTag"}
    validator = Draft202012Validator(context_tag)
    assert validator.is_valid(COMMENT_TAG)


def test_lint_report_doc_states_the_relation():
    doc = (ROOT / "docs" / "contract" / "lint-report.md").read_text(encoding="utf-8")
    assert f"the tag `{COMMENT_TAG}`" in doc
    assert f"`{COMMENT_MODE}`" in doc
