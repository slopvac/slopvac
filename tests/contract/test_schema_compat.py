"""The additive-only schema check flags every breaking kind and allows additions."""

from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
_spec = importlib.util.spec_from_file_location(
    "schema_compat", ROOT / "scripts" / "contract" / "schema_compat.py"
)
compat = importlib.util.module_from_spec(_spec)
sys.modules[_spec.name] = compat
_spec.loader.exec_module(compat)

ID = "https://example.test/schemas/v3/thing.schema.json"
FILE = "thing.schema.json"


def schema(**body: object) -> dict:
    return {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": ID,
        "type": "object",
        **body,
    }


def write_dir(
    directory: Path, schemas: dict[str, dict], corpus: dict[str, str] | None = None
) -> Path:
    directory.mkdir(parents=True, exist_ok=True)
    for name, body in schemas.items():
        (directory / name).write_text(json.dumps(body), encoding="utf-8")
    for rel, text in (corpus or {}).items():
        target = directory / "conformance" / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")
    return directory


def run(tmp_path: Path, old: dict, new: dict, allow=(), corpus=None) -> list:
    base = compat.snapshot_from_dir(write_dir(tmp_path / "old", {FILE: old}, corpus))
    return compat.check(base, write_dir(tmp_path / "new", {FILE: new}), list(allow))


def kinds(findings: list) -> list[tuple[str, str]]:
    return [(f.pointer, f.kind) for f in findings]


PROP = {"properties": {"a": {"type": "string"}}}
OLD_WITH_A = schema(**PROP)

BREAKING = {
    "removed-property": (
        OLD_WITH_A,
        schema(properties={}),
        "/properties/a",
        "property-removed",
    ),
    "removed-def": (
        schema(**{"$defs": {"D": {"type": "string"}}}),
        schema(**{"$defs": {}}),
        "/$defs/D",
        "def-removed",
    ),
    "removed-nested-def-property": (
        schema(**{"$defs": {"D": {"properties": {"x": {"type": "string"}}}}}),
        schema(**{"$defs": {"D": {"properties": {}}}}),
        "/$defs/D/properties/x",
        "property-removed",
    ),
    "removed-property-in-items": (
        schema(items={"properties": {"x": {}}}),
        schema(items={"properties": {}}),
        "/items/properties/x",
        "property-removed",
    ),
    "removed-property-in-allof": (
        schema(allOf=[{"properties": {"x": {}}}]),
        schema(allOf=[{"properties": {}}]),
        "/allOf/0/properties/x",
        "property-removed",
    ),
    "removed-property-in-then": (
        schema(**{"if": {"required": ["k"]}, "then": {"properties": {"x": {}}}}),
        schema(**{"if": {"required": ["k"]}, "then": {"properties": {}}}),
        "/then/properties/x",
        "property-removed",
    ),
    "new-required": (
        schema(required=["a"]),
        schema(required=["a", "b"]),
        "/required",
        "required-added",
    ),
    "type-narrowed": (
        {**schema(), "type": ["string", "null"]},
        {**schema(), "type": "string"},
        "/type",
        "type-narrowed",
    ),
    "number-to-integer": (
        {**schema(), "type": "number"},
        {**schema(), "type": "integer"},
        "/type",
        "type-narrowed",
    ),
    "enum-narrowed": (
        schema(properties={"e": {"enum": ["a", "b"]}}),
        schema(properties={"e": {"enum": ["a"]}}),
        "/properties/e/enum",
        "enum-narrowed",
    ),
    "enum-added-where-open": (
        schema(properties={"e": {"type": "string"}}),
        schema(properties={"e": {"type": "string", "enum": ["a"]}}),
        "/properties/e/enum",
        "enum-narrowed",
    ),
    "const-narrowed": (
        schema(properties={"e": {"enum": ["a", "b"]}}),
        schema(properties={"e": {"const": "a"}}),
        "/properties/e/const",
        "const-narrowed",
    ),
    "const-changed": (
        schema(properties={"v": {"const": 3}}),
        schema(properties={"v": {"const": 4}}),
        "/properties/v/const",
        "const-narrowed",
    ),
    "pattern-changed": (
        schema(properties={"s": {"pattern": "^a"}}),
        schema(properties={"s": {"pattern": "^b"}}),
        "/properties/s/pattern",
        "pattern-changed",
    ),
    "pattern-added": (
        schema(properties={"s": {"type": "string"}}),
        schema(properties={"s": {"type": "string", "pattern": "^b"}}),
        "/properties/s/pattern",
        "pattern-changed",
    ),
    "additional-properties-false": (
        schema(),
        schema(additionalProperties=False),
        "/additionalProperties",
        "additional-properties-narrowed",
    ),
    "additional-properties-narrower-schema": (
        schema(additionalProperties=True),
        schema(additionalProperties={"type": "string"}),
        "/additionalProperties",
        "additional-properties-narrowed",
    ),
    "additional-properties-schema-tightened": (
        schema(additionalProperties={"type": "string"}),
        schema(additionalProperties={"type": "string", "minLength": 2}),
        "/additionalProperties/minLength",
        "bound-tightened",
    ),
    "pattern-property-removed": (
        schema(patternProperties={"^x-": {}}),
        schema(patternProperties={}),
        "/patternProperties/^x-",
        "pattern-property-removed",
    ),
    "alternative-removed": (
        schema(anyOf=[{"type": "string"}, {"type": "null"}]),
        schema(anyOf=[{"type": "string"}]),
        "/anyOf/1",
        "alternative-removed",
    ),
    "ref-changed": (
        schema(properties={"r": {"$ref": "#/$defs/A"}}),
        schema(properties={"r": {"$ref": "#/$defs/B"}}),
        "/properties/r/$ref",
        "ref-changed",
    ),
    "id-changed": (
        schema(),
        {**schema(), "$id": "https://example.test/schemas/v3/other.schema.json"},
        "/$id",
        "id-changed",
    ),
    "allof-member-added": (
        schema(allOf=[{}]),
        schema(allOf=[{}, {"required": ["z"]}]),
        "/allOf/1/required",
        "required-added",
    ),
    "if-changed": (
        schema(**{"if": {"required": ["a"]}, "then": {}}),
        schema(**{"if": {"required": ["b"]}, "then": {}}),
        "/if",
        "if-changed",
    ),
    "schema-false": (
        schema(properties={"p": {"type": "string"}}),
        schema(properties={"p": False}),
        "/properties/p",
        "schema-narrowed",
    ),
}


@pytest.mark.parametrize(
    ("old", "new", "pointer", "kind"), BREAKING.values(), ids=BREAKING.keys()
)
def test_breaking_change_is_reported(tmp_path, old, new, pointer, kind):
    assert kinds(run(tmp_path, old, new)) == [(pointer, kind)]


BOUNDS = [
    ("minimum", 1, 2),
    ("exclusiveMinimum", 1, 2),
    ("minLength", 1, 2),
    ("minItems", 0, 1),
    ("minProperties", 0, 1),
    ("maximum", 10, 5),
    ("exclusiveMaximum", 10, 5),
    ("maxLength", 10, 5),
    ("maxItems", 10, 5),
    ("maxProperties", 10, 5),
]


@pytest.mark.parametrize(("keyword", "old", "new"), BOUNDS, ids=[b[0] for b in BOUNDS])
def test_tightened_bound_is_reported(tmp_path, keyword, old, new):
    old_s = schema(properties={"n": {keyword: old}})
    new_s = schema(properties={"n": {keyword: new}})
    assert kinds(run(tmp_path, old_s, new_s)) == [
        (f"/properties/n/{keyword}", "bound-tightened")
    ]


@pytest.mark.parametrize("keyword", [b[0] for b in BOUNDS])
def test_bound_added_where_open_is_reported(tmp_path, keyword):
    new_s = schema(properties={"n": {keyword: 1}})
    findings = run(tmp_path, schema(properties={"n": {}}), new_s)
    assert kinds(findings) == [(f"/properties/n/{keyword}", "bound-tightened")]


ALLOWED = {
    "optional-property-added": (
        OLD_WITH_A,
        schema(properties={"a": {"type": "string"}, "b": {}}),
    ),
    "new-def": (schema(), schema(**{"$defs": {"N": {"type": "string"}}})),
    "enum-widened": (
        schema(properties={"e": {"enum": ["a"]}}),
        schema(properties={"e": {"enum": ["a", "b"]}}),
    ),
    "type-widened": ({**schema(), "type": "integer"}, {**schema(), "type": "number"}),
    "type-removed": (
        {**schema(), "type": "string"},
        {k: v for k, v in schema().items() if k != "type"},
    ),
    "required-dropped": (schema(required=["a"]), schema()),
    "bounds-loosened": (
        schema(properties={"n": {"minimum": 2, "maxLength": 5}}),
        schema(properties={"n": {"minimum": 1, "maxLength": 9}}),
    ),
    "additional-properties-opened": (
        schema(additionalProperties=False),
        schema(additionalProperties=True),
    ),
    "alternative-added": (
        schema(anyOf=[{"type": "string"}]),
        schema(anyOf=[{"type": "string"}, {"type": "null"}]),
    ),
    "description-edited": (
        schema(description="old"),
        schema(description="new", title="added"),
    ),
}


@pytest.mark.parametrize(("old", "new"), ALLOWED.values(), ids=ALLOWED.keys())
def test_additive_change_is_allowed(tmp_path, old, new):
    assert run(tmp_path, old, new) == []


def test_new_file_is_allowed(tmp_path):
    base = compat.snapshot_from_dir(write_dir(tmp_path / "old", {FILE: schema()}))
    other = {**schema(), "$id": "https://example.test/schemas/v3/extra.schema.json"}
    new_dir = write_dir(tmp_path / "new", {FILE: schema(), "extra.schema.json": other})
    assert compat.check(base, new_dir, []) == []


def test_removed_file_is_reported(tmp_path):
    base = compat.snapshot_from_dir(write_dir(tmp_path / "old", {FILE: schema()}))
    findings = compat.check(base, write_dir(tmp_path / "new", {}), [])
    assert [(f.file, f.kind) for f in findings] == [(FILE, "file-removed")]


def test_finding_renders_file_pointer_kind_old_new(tmp_path):
    findings = run(tmp_path, schema(required=[]), schema(required=["b"]))
    assert findings[0].render() == f'{FILE}:/required: required-added: (absent) -> "b"'


def entry(**over: str) -> object:
    fields = {
        "file": FILE,
        "pointer": "/properties/a",
        "kind": "property-removed",
        "reason": "reviewed",
        "approved_by": "tester",
    }
    return compat.AllowEntry(**{**fields, **over})


def test_allow_entry_suppresses_its_finding(tmp_path):
    assert run(tmp_path, OLD_WITH_A, schema(properties={}), allow=[entry()]) == []


def test_allow_entry_suppresses_only_the_finding_it_names(tmp_path):
    old = schema(properties={"a": {}, "b": {}})
    new = schema(properties={})
    assert kinds(run(tmp_path, old, new, allow=[entry()])) == [
        ("/properties/b", "property-removed")
    ]


def test_stale_allow_entry_is_reported(tmp_path):
    findings = run(tmp_path, schema(), schema(), allow=[entry()])
    assert [(f.file, f.pointer, f.kind) for f in findings] == [
        (FILE, "/properties/a", "allow-entry-stale")
    ]


def test_allow_file_is_parsed(tmp_path):
    path = tmp_path / "allow.toml"
    path.write_text(
        '[[allow]]\nfile = "a.schema.json"\npointer = "/x"\nkind = "property-removed"\n'
        'reason = "r"\napproved_by = "me"\n',
        encoding="utf-8",
    )
    assert compat.load_allow(path) == [
        compat.AllowEntry("a.schema.json", "/x", "property-removed", "r", "me")
    ]


@pytest.mark.parametrize(
    "body",
    [
        '[[allow]]\nfile = "a"\n',
        '[[allow]]\nfile="a"\npointer=""\nkind="k"\nreason=""\napproved_by="x"\n',
    ],
    ids=["missing-fields", "empty-reason"],
)
def test_malformed_allow_entry_is_an_error(tmp_path, body):
    path = tmp_path / "allow.toml"
    path.write_text(body, encoding="utf-8")
    with pytest.raises(compat.CompatError):
        compat.load_allow(path)


def test_committed_allow_file_has_no_entries():
    assert compat.load_allow(ROOT / "schemas" / "v3" / "compat-allow.toml") == []


CORPUS_OLD = schema(properties={"n": {"type": "integer"}})


def test_baseline_valid_instance_rejected_by_new_schema_is_reported(tmp_path):
    new = schema(properties={"n": {"type": "integer"}}, additionalProperties=False)
    corpus = {"thing/valid/ok.json": '{"n": 1, "extra": 2}'}
    findings = run(tmp_path, CORPUS_OLD, new, corpus=corpus)
    corpus_findings = [f for f in findings if f.kind == "corpus-rejected"]
    assert [(f.file, f.pointer) for f in corpus_findings] == [
        (FILE, "/conformance/valid/ok.json")
    ]


def test_baseline_valid_instance_still_accepted_passes(tmp_path):
    corpus = {"thing/valid/ok.json": '{"n": 1}', "thing/invalid/bad.json": '{"n": "x"}'}
    assert run(tmp_path, CORPUS_OLD, CORPUS_OLD, corpus=corpus) == []


def test_corpus_instance_in_toml_and_def_form_is_checked(tmp_path):
    old = schema(
        **{"$defs": {"D": {"type": "object", "properties": {"k": {"type": "string"}}}}}
    )
    new = schema(
        **{
            "$defs": {
                "D": {
                    "type": "object",
                    "properties": {"k": {"type": "string"}},
                    "required": ["k"],
                }
            }
        }
    )
    corpus = {"thing/valid/D--empty.toml": "other = 1\n"}
    findings = run(tmp_path, old, new, corpus=corpus)
    assert ("/conformance/valid/D--empty.toml", "corpus-rejected") in kinds(findings)


def test_cli_exit_codes(tmp_path):
    old = write_dir(tmp_path / "old", {FILE: schema(required=[])})
    ok = write_dir(tmp_path / "ok", {FILE: schema(required=[])})
    bad = write_dir(tmp_path / "bad", {FILE: schema(required=["b"])})
    base = ["--baseline-dir", str(old), "--repo", str(tmp_path)]
    assert compat.main([*base, "--schemas-dir", str(ok)]) == 0
    assert compat.main([*base, "--schemas-dir", str(bad)]) == 1
    assert (
        compat.main(
            ["--baseline-dir", str(tmp_path / "missing"), "--schemas-dir", str(ok)]
        )
        == 2
    )
    assert compat.main([]) == 2


def test_git_ref_baseline_is_read_through_git_show(tmp_path):
    import subprocess

    repo = tmp_path / "repo"
    write_dir(
        repo / "schemas" / "v3",
        {FILE: schema(required=[])},
        {"thing/valid/ok.json": "{}"},
    )
    git = [
        "git",
        "-C",
        str(repo),
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@example.test",
    ]
    subprocess.run([*git, "init", "-q"], check=True)
    subprocess.run([*git, "add", "."], check=True)
    subprocess.run([*git, "commit", "-q", "-m", "base"], check=True)
    snapshot = compat.snapshot_from_ref(repo, "HEAD")
    assert snapshot.schemas[FILE] == schema(required=[])
    assert snapshot.corpus == {"thing/valid/ok.json": "{}"}
    with pytest.raises(compat.CompatError):
        compat.snapshot_from_ref(repo, "no-such-ref")
