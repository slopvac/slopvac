"""The schema equivalence normal form treats layout as layout and meaning as meaning."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

import pytest
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[2]
_spec = importlib.util.spec_from_file_location(
    "schema_equivalence", ROOT / "scripts" / "contract" / "schema_equivalence.py"
)
equivalence = importlib.util.module_from_spec(_spec)
sys.modules["schema_equivalence"] = equivalence
_spec.loader.exec_module(equivalence)

BASE = "https://example.test/v3/"


def side(**docs: dict) -> object:
    return equivalence.Side(
        {
            name: {"$id": f"{BASE}{name}.schema.json", **doc}
            for name, doc in docs.items()
        }
    )


COMMON = {"$defs": {"Id": {"type": "string", "pattern": "^[a-z]+$"}}}
OLD_REPORT = {
    "type": "object",
    "properties": {"id": {"$ref": "common.schema.json#/$defs/Id"}},
    "required": ["id", "n"],
    "patternProperties": {"^x-": {}},
    "additionalProperties": False,
    "description": "Old words.",
}


@pytest.mark.parametrize(
    "new_report",
    [
        pytest.param(
            {
                "$defs": {"Id": {"type": "string", "pattern": "^[a-z]+$"}},
                "additionalProperties": False,
                "patternProperties": {"^x-": True},
                "properties": {"id": {"$ref": "#/$defs/Id", "description": "Id."}},
                "required": ["n", "id"],
                "type": "object",
            },
            id="local-def-sorted-required-true-for-empty",
        ),
        pytest.param(
            {
                "allOf": [{"type": "object"}, {"required": ["n", "id"]}],
                "properties": {"id": {"type": ["string"], "pattern": "^[a-z]+$"}},
                "patternProperties": {"^x-": True},
                "additionalProperties": False,
            },
            id="inlined-and-split-conjunction",
        ),
    ],
)
def test_a_layout_change_is_equivalent(new_report: dict) -> None:
    old = side(common=COMMON, report=OLD_REPORT)
    new = side(common=COMMON, report=new_report)
    assert equivalence.compare(old, new, "report") == []


def test_a_tightened_pattern_is_a_difference_unless_expected() -> None:
    tightened = {"$defs": {"Id": {"type": "string", "pattern": "^[a-z]+(?![\\s\\S])"}}}
    old = side(common=COMMON, report=OLD_REPORT)
    new = side(common=tightened, report=OLD_REPORT)
    differences = equivalence.compare(old, new, "report")
    assert [d[0] for d in differences] == ["/properties/id/pattern"]
    expected = {"^[a-z]+$": "^[a-z]+(?![\\s\\S])"}
    assert equivalence.compare(old, new, "report", expected) == []


def test_a_removed_def_is_a_difference() -> None:
    old = side(common=COMMON)
    new = side(common={"$defs": {}})
    assert equivalence.compare(old, new, "common") == [
        ("/$defs/Id", "<def>", "<absent>")
    ]


def test_a_dropped_closing_keyword_is_a_difference() -> None:
    loose = {k: v for k, v in OLD_REPORT.items() if k != "additionalProperties"}
    old = side(common=COMMON, report=OLD_REPORT)
    new = side(common=COMMON, report=loose)
    assert [d[0] for d in equivalence.compare(old, new, "report")] == [
        "/additionalProperties"
    ]


# Each pair validates `instance` differently, but a merge that ignores keyword scope gives
# both the same normal form.
@pytest.mark.parametrize(
    ("old_schema", "new_schema", "instance"),
    [
        pytest.param(
            {"allOf": [{"if": {"type": "integer"}}, {"then": {"maximum": 0}}]},
            {"if": {"type": "integer"}, "then": {"maximum": 0}},
            5,
            id="if-then-split",
        ),
        pytest.param(
            {"allOf": [{"if": {"type": "integer"}}, {"else": {"type": "integer"}}]},
            {"if": {"type": "integer"}, "else": {"type": "integer"}},
            "a",
            id="if-else-split",
        ),
        pytest.param(
            {"allOf": [{"prefixItems": [True]}, {"items": False}]},
            {"prefixItems": [True], "items": False},
            [1],
            id="prefix-items-split",
        ),
        pytest.param(
            {"allOf": [{"contains": {"type": "integer"}}, {"minContains": 2}]},
            {"contains": {"type": "integer"}, "minContains": 2},
            [1],
            id="min-contains-split",
        ),
        pytest.param(
            {"allOf": [{"contains": {"type": "integer"}}, {"maxContains": 1}]},
            {"contains": {"type": "integer"}, "maxContains": 1},
            [1, 2],
            id="max-contains-split",
        ),
        pytest.param(
            {"unevaluatedProperties": False, "allOf": [{"properties": {"a": True}}]},
            {"allOf": [{"unevaluatedProperties": False}, {"properties": {"a": True}}]},
            {"a": 1},
            id="unevaluated-properties-moved-into-a-member",
        ),
        pytest.param(
            {"unevaluatedItems": False, "allOf": [{"prefixItems": [True]}]},
            {"allOf": [{"unevaluatedItems": False}, {"prefixItems": [True]}]},
            [1],
            id="unevaluated-items-moved-into-a-member",
        ),
    ],
)
def test_a_split_that_changes_keyword_scope_is_a_difference(
    old_schema: dict, new_schema: dict, instance: object
) -> None:
    before = Draft202012Validator(old_schema).is_valid(instance)
    after = Draft202012Validator(new_schema).is_valid(instance)
    assert before is not after, "the pair must validate the instance differently"
    old = side(report=old_schema)
    new = side(report=new_schema)
    assert equivalence.compare(old, new, "report") != []
