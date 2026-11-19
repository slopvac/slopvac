"""The additive-only schema check flags every breaking kind and allows additions."""

from __future__ import annotations

import importlib.util
import json
import sys
from collections.abc import Callable
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
_spec = importlib.util.spec_from_file_location(
    "schema_compat", ROOT / "scripts" / "contract" / "schema_compat.py"
)
assert _spec is not None and _spec.loader is not None
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
        "/required/b",
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
    "ref-retargeted-to-narrower-def": (
        schema(
            properties={"r": {"$ref": "#/$defs/A"}},
            **{
                "$defs": {
                    "A": {"type": "string"},
                    "B": {"type": "string", "minLength": 3},
                }
            },
        ),
        schema(
            properties={"r": {"$ref": "#/$defs/B"}},
            **{
                "$defs": {
                    "A": {"type": "string"},
                    "B": {"type": "string", "minLength": 3},
                }
            },
        ),
        "/properties/r/$ref(thing.schema.json#/$defs/B)/minLength",
        "bound-tightened",
    ),
    "unresolvable-ref": (
        schema(properties={"r": {"$ref": "#/$defs/Missing"}}),
        schema(properties={"r": {"$ref": "#/$defs/Missing"}}),
        "/properties/r/$ref",
        "unverified",
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
        "/allOf/1/required/z",
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
    assert (
        findings[0].render() == f'{FILE}:/required/b: required-added: (absent) -> "b"'
    )


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


def test_header_only_allow_file_has_no_entries(tmp_path):
    path = tmp_path / "allow.toml"
    path.write_text("# reviewed exceptions\n", encoding="utf-8")
    assert compat.load_allow(path) == []


ALLOW_A = (
    '[[allow]]\nfile = "thing.schema.json"\npointer = "/properties/a"\n'
    'kind = "property-removed"\nreason = "reviewed"\napproved_by = "tester"\n'
)
NEVER_MATCHES = (
    '[[allow]]\nfile = "thing.schema.json"\npointer = "/properties/zzz"\n'
    'kind = "property-removed"\nreason = "typo"\napproved_by = "tester"\n'
)


def git_repo(tmp_path: Path) -> tuple[Path, Callable[..., None]]:
    import subprocess

    repo = tmp_path / "repo"
    (repo / "schemas" / "v3").mkdir(parents=True)

    def git(*args: str) -> None:
        subprocess.run(
            [
                "git",
                "-C",
                str(repo),
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                *args,
            ],
            check=True,
            capture_output=True,
        )

    git("init", "-q")
    return repo, git


def commit(repo: Path, git: Callable[..., None], body: dict, allow: str) -> None:
    write_dir(repo / "schemas" / "v3", {FILE: body})
    (repo / "schemas" / "v3" / compat.ALLOW_FILE).write_text(allow, encoding="utf-8")
    git("add", "-A")
    git("commit", "-q", "-m", "step")


def test_allow_entry_still_passes_once_its_change_is_on_the_base(tmp_path):
    repo, git = git_repo(tmp_path)
    commit(repo, git, OLD_WITH_A, "")
    commit(repo, git, schema(properties={}), ALLOW_A)
    assert compat.main(["--repo", str(repo), "--base", "HEAD~1"]) == 0
    # The change has landed: the base now holds the removal and its entry.
    assert compat.main(["--repo", str(repo), "--base", "HEAD"]) == 0


@pytest.mark.parametrize("base", ["HEAD~1", "HEAD"], ids=["before", "after-landing"])
def test_an_added_entry_that_matches_nothing_is_stale_on_either_base(tmp_path, base):
    repo, git = git_repo(tmp_path)
    commit(repo, git, OLD_WITH_A, "")
    commit(repo, git, schema(properties={}), ALLOW_A)
    (repo / "schemas" / "v3" / compat.ALLOW_FILE).write_text(
        ALLOW_A + NEVER_MATCHES, encoding="utf-8"
    )
    findings = compat.check(
        compat.snapshot_from_ref(repo, base),
        repo / "schemas" / "v3",
        compat.load_allow(repo / "schemas" / "v3" / compat.ALLOW_FILE),
    )
    assert kinds(findings) == [("/properties/zzz", "allow-entry-stale")]


def test_a_landed_entry_does_not_suppress_a_new_finding_at_its_pointer(tmp_path):
    # The base already holds this entry, so it approved an earlier, landed change.
    # Removing `a` now is a new break and needs a new review.
    base = write_dir(tmp_path / "old", {FILE: OLD_WITH_A})
    (base / compat.ALLOW_FILE).write_text(ALLOW_A, encoding="utf-8")
    new = write_dir(tmp_path / "new", {FILE: schema(properties={})})
    findings = compat.check(
        compat.snapshot_from_dir(base), new, compat.load_allow(base / compat.ALLOW_FILE)
    )
    assert kinds(findings) == [("/properties/a", "property-removed")]


UNRESOLVED = "https://elsewhere.test/x.schema.json"
RESOLVED = "#/$defs/D"
ALLOW_REF = (
    '[[allow]]\nfile = "thing.schema.json"\npointer = "/properties/p/$ref"\n'
    'kind = "unverified"\nreason = "external"\napproved_by = "tester"\n'
)


def with_ref(ref: str) -> dict:
    return schema(
        properties={"p": {"$ref": ref}}, **{"$defs": {"D": {"type": "string"}}}
    )


def test_an_approved_unresolved_ref_still_passes_once_landed(tmp_path):
    repo, git = git_repo(tmp_path)
    commit(repo, git, with_ref(RESOLVED), "")
    commit(repo, git, with_ref(UNRESOLVED), ALLOW_REF)
    assert compat.main(["--repo", str(repo), "--base", "HEAD~1"]) == 0
    # On the landed base the reference is unchanged but still unresolvable.
    assert compat.main(["--repo", str(repo), "--base", "HEAD"]) == 0


def test_a_landed_entry_does_not_cover_a_changed_unresolved_ref(tmp_path, capsys):
    repo, git = git_repo(tmp_path)
    commit(repo, git, with_ref(RESOLVED), "")
    commit(repo, git, with_ref(UNRESOLVED), ALLOW_REF)
    write_dir(repo / "schemas" / "v3", {FILE: with_ref(UNRESOLVED + "?v=2")})
    assert compat.main(["--repo", str(repo), "--base", "HEAD"]) == 1
    assert "/properties/p/$ref: unverified" in capsys.readouterr().out


def test_an_allow_file_override_ignores_the_default_one(tmp_path):
    old = write_dir(tmp_path / "old", {FILE: OLD_WITH_A})
    new = write_dir(tmp_path / "new", {FILE: schema(properties={})})
    (new / compat.ALLOW_FILE).write_text("not = [toml", encoding="utf-8")
    chosen = tmp_path / "chosen.toml"
    chosen.write_text(ALLOW_A, encoding="utf-8")
    args = ["--baseline-dir", str(old), "--schemas-dir", str(new)]
    assert compat.main([*args, "--allow-file", str(chosen)]) == 0


def test_missing_allow_file_has_no_entries(tmp_path):
    assert compat.load_allow(tmp_path / "absent.toml") == []


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


TIGHTENING = {
    "format-added": (
        {"type": "string"},
        {"type": "string", "format": "uri"},
        "/properties/x/format",
        "constraint-added",
    ),
    "format-changed": (
        {"type": "string", "format": "uri"},
        {"type": "string", "format": "email"},
        "/properties/x/format",
        "constraint-added",
    ),
    "multiple-of-added": (
        {"type": "integer"},
        {"type": "integer", "multipleOf": 5},
        "/properties/x/multipleOf",
        "constraint-added",
    ),
    "unique-items-added": (
        {"type": "array"},
        {"type": "array", "uniqueItems": True},
        "/properties/x/uniqueItems",
        "constraint-added",
    ),
    "not-added": (
        {"type": "string"},
        {"type": "string", "not": {"const": "forbidden"}},
        "/properties/x/not",
        "constraint-added",
    ),
    "dependent-required-extended": (
        {"dependentRequired": {"a": ["b"]}},
        {"dependentRequired": {"a": ["b", "c"]}},
        "/properties/x/dependentRequired/a/c",
        "required-added",
    ),
    "dependent-required-new-key": (
        {"type": "object"},
        {"type": "object", "dependentRequired": {"a": ["b"]}},
        "/properties/x/dependentRequired/a/b",
        "required-added",
    ),
    "min-contains-raised": (
        {"contains": {"type": "string"}, "minContains": 1},
        {"contains": {"type": "string"}, "minContains": 2},
        "/properties/x/minContains",
        "bound-tightened",
    ),
    "max-contains-lowered": (
        {"contains": {"type": "string"}, "maxContains": 5},
        {"contains": {"type": "string"}, "maxContains": 3},
        "/properties/x/maxContains",
        "bound-tightened",
    ),
}


@pytest.mark.parametrize(
    ("old", "new", "pointer", "kind"), TIGHTENING.values(), ids=TIGHTENING.keys()
)
def test_tightening_keyword_is_reported(tmp_path, old, new, pointer, kind):
    findings = run(
        tmp_path, schema(properties={"x": old}), schema(properties={"x": new})
    )
    assert kinds(findings) == [(pointer, kind)]


LOOSENING = {
    "format-removed": ({"format": "uri"}, {}),
    "multiple-of-removed": ({"multipleOf": 5}, {}),
    "unique-items-dropped": ({"uniqueItems": True}, {"uniqueItems": False}),
    "not-removed": ({"not": {"const": "x"}}, {}),
    "dependent-required-shrunk": (
        {"dependentRequired": {"a": ["b", "c"]}},
        {"dependentRequired": {"a": ["b"]}},
    ),
    "max-contains-raised": (
        {"contains": {}, "maxContains": 3},
        {"contains": {}, "maxContains": 5},
    ),
}


@pytest.mark.parametrize(("old", "new"), LOOSENING.values(), ids=LOOSENING.keys())
def test_loosened_keyword_is_allowed(tmp_path, old, new):
    findings = run(
        tmp_path, schema(properties={"x": old}), schema(properties={"x": new})
    )
    assert findings == []


UNMODELLED = {
    "added": ({}, {"contentMediaType": "application/json"}, "contentMediaType"),
    "changed": (
        {"contentEncoding": "base64"},
        {"contentEncoding": "base32"},
        "contentEncoding",
    ),
    "removed": ({"$anchor": "a"}, {}, "$anchor"),
    "default-changed": ({"default": 1}, {"default": 2}, "default"),
    "default-added": ({}, {"default": 1}, "default"),
}


@pytest.mark.parametrize(
    ("old", "new", "keyword"), UNMODELLED.values(), ids=UNMODELLED.keys()
)
def test_unmodelled_keyword_change_fails_closed(tmp_path, old, new, keyword):
    findings = run(
        tmp_path, schema(properties={"x": old}), schema(properties={"x": new})
    )
    assert kinds(findings) == [(f"/properties/x/{keyword}", "unverified")]


def test_unmodelled_keyword_is_suppressed_by_an_allow_entry(tmp_path):
    allow = entry(pointer="/properties/x/default", kind="unverified")
    findings = run(
        tmp_path,
        schema(properties={"x": {"default": 1}}),
        schema(properties={"x": {"default": 2}}),
        allow=[allow],
    )
    assert findings == []


ANNOTATIONS = [
    ("description", "a", "b"),
    ("title", "a", "b"),
    ("examples", [1], [2]),
    ("$comment", "a", "b"),
    ("deprecated", False, True),
    ("readOnly", False, True),
    ("writeOnly", False, True),
]


@pytest.mark.parametrize(
    ("keyword", "old", "new"), ANNOTATIONS, ids=[a[0] for a in ANNOTATIONS]
)
def test_annotation_change_is_allowed(tmp_path, keyword, old, new):
    findings = run(
        tmp_path,
        schema(properties={"x": {keyword: old}}),
        schema(properties={"x": {keyword: new}}),
    )
    assert findings == []


A, B, C = {"title": "A"}, {"title": "B"}, {"title": "C"}
REORDERED = {
    "anyOf-swapped": ("anyOf", [A, B], [B, A], "/anyOf"),
    "oneOf-swapped": ("oneOf", [A, B], [B, A], "/oneOf"),
    "allOf-swapped": ("allOf", [A, B], [B, A], "/allOf"),
    "prefixItems-swapped": ("prefixItems", [A, B], [B, A], "/prefixItems"),
    "anyOf-inserted-first": ("anyOf", [A, B], [C, A, B], "/anyOf"),
}


@pytest.mark.parametrize(
    ("keyword", "old", "new", "pointer"), REORDERED.values(), ids=REORDERED.keys()
)
def test_reordered_list_fails_closed(tmp_path, keyword, old, new, pointer):
    findings = run(tmp_path, schema(**{keyword: old}), schema(**{keyword: new}))
    assert kinds(findings) == [(pointer, "list-reordered")]


def test_reordered_list_is_suppressed_by_an_allow_entry(tmp_path):
    allow = entry(pointer="/anyOf", kind="list-reordered")
    findings = run(tmp_path, schema(anyOf=[A, B]), schema(anyOf=[B, A]), allow=[allow])
    assert findings == []


def test_list_member_appended_is_not_a_reorder(tmp_path):
    assert run(tmp_path, schema(anyOf=[A, B]), schema(anyOf=[A, B, C])) == []


COMMON_ID = "https://example.test/schemas/v3/common.schema.json"


def common(**defs: object) -> dict:
    return {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": COMMON_ID,
        "$defs": defs,
    }


def referrer(ref: str) -> dict:
    return schema(properties={"p": {"$ref": ref}})


def run_files(tmp_path: Path, old: dict, new: dict) -> list:
    base = compat.snapshot_from_dir(write_dir(tmp_path / "old", old))
    return compat.check(base, write_dir(tmp_path / "new", new), [])


REF_FORMS = ["common.schema.json#/$defs/D", f"{COMMON_ID}#/$defs/D"]


@pytest.mark.parametrize("ref", REF_FORMS, ids=["relative", "absolute"])
def test_narrowed_shared_def_is_reported_against_the_referrer(tmp_path, ref):
    old = {
        "common.schema.json": common(D={"type": "string", "minLength": 1}),
        FILE: referrer(ref),
    }
    new = {
        "common.schema.json": common(D={"type": "string", "minLength": 2}),
        FILE: referrer(ref),
    }
    findings = run_files(tmp_path, old, new)
    pointer = "/properties/p/$ref(common.schema.json#/$defs/D)/minLength"
    assert (FILE, pointer, "bound-tightened") in [
        (f.file, f.pointer, f.kind) for f in findings
    ]
    assert ("common.schema.json", "/$defs/D/minLength", "bound-tightened") in [
        (f.file, f.pointer, f.kind) for f in findings
    ]


def test_removed_property_of_shared_def_is_reported_against_the_referrer(tmp_path):
    ref = "common.schema.json#/$defs/D"
    old = {
        "common.schema.json": common(D={"properties": {"k": {}}}),
        FILE: referrer(ref),
    }
    new = {"common.schema.json": common(D={"properties": {}}), FILE: referrer(ref)}
    found = [(f.file, f.pointer, f.kind) for f in run_files(tmp_path, old, new)]
    assert (
        FILE,
        "/properties/p/$ref(common.schema.json#/$defs/D)/properties/k",
        "property-removed",
    ) in found


def test_unchanged_ref_with_widened_target_passes(tmp_path):
    ref = "common.schema.json#/$defs/D"
    old = {
        "common.schema.json": common(D={"type": "string", "minLength": 2}),
        FILE: referrer(ref),
    }
    new = {
        "common.schema.json": common(D={"type": "string", "minLength": 1}),
        FILE: referrer(ref),
    }
    assert run_files(tmp_path, old, new) == []


def test_ref_added_to_a_narrower_def_is_reported(tmp_path):
    ref = "common.schema.json#/$defs/D"
    defs = common(D={"type": "string", "minLength": 2})
    old = {"common.schema.json": defs, FILE: schema(properties={"p": {}})}
    new = {"common.schema.json": defs, FILE: referrer(ref)}
    found = [(f.pointer, f.kind) for f in run_files(tmp_path, old, new)]
    assert (
        "/properties/p/$ref(common.schema.json#/$defs/D)/minLength",
        "bound-tightened",
    ) in found


def tree(min_length: int) -> dict:
    return common(
        Tree={
            "type": "object",
            "properties": {
                "name": {"type": "string", "minLength": min_length},
                "child": {"$ref": "#/$defs/Tree"},
            },
        }
    )


def test_self_referencing_def_terminates_and_reports_once_per_path(tmp_path):
    ref = "common.schema.json#/$defs/Tree"
    old = {"common.schema.json": tree(1), FILE: referrer(ref)}
    new = {"common.schema.json": tree(2), FILE: referrer(ref)}
    found = [(f.file, f.pointer, f.kind) for f in run_files(tmp_path, old, new)]
    assert (
        FILE,
        "/properties/p/$ref(common.schema.json#/$defs/Tree)/properties/name/minLength",
        "bound-tightened",
    ) in found


def test_unchanged_self_referencing_def_passes(tmp_path):
    ref = "common.schema.json#/$defs/Tree"
    files = {"common.schema.json": tree(1), FILE: referrer(ref)}
    assert run_files(tmp_path, files, files) == []


def test_mutual_cycle_across_files_terminates(tmp_path):
    a = schema(
        **{
            "$defs": {
                "A": {"properties": {"b": {"$ref": "other.schema.json#/$defs/B"}}}
            }
        }
    )
    b = {
        **schema(
            **{
                "$defs": {
                    "B": {"properties": {"a": {"$ref": "thing.schema.json#/$defs/A"}}}
                }
            }
        ),
        "$id": "https://example.test/schemas/v3/other.schema.json",
    }
    files = {FILE: a, "other.schema.json": b}
    assert run_files(tmp_path, files, files) == []


def test_unreadable_schema_is_a_chained_read_error(tmp_path):
    directory = write_dir(tmp_path / "bad", {})
    (directory / FILE).write_bytes(b"\xff\xfe not utf-8")
    with pytest.raises(compat.CompatError) as caught:
        compat.snapshot_from_dir(directory)
    assert isinstance(caught.value.__cause__, UnicodeDecodeError)


def test_schema_path_that_is_a_directory_is_a_chained_read_error(tmp_path):
    directory = write_dir(tmp_path / "bad", {})
    (directory / FILE).mkdir()
    with pytest.raises(compat.CompatError) as caught:
        compat.snapshot_from_dir(directory)
    assert isinstance(caught.value.__cause__, OSError)


def test_unreadable_corpus_file_is_a_chained_read_error(tmp_path):
    directory = write_dir(tmp_path / "bad", {FILE: schema()})
    target = directory / "conformance" / "thing" / "valid" / "x.json"
    target.parent.mkdir(parents=True)
    target.write_bytes(b"\xff\xfe")
    with pytest.raises(compat.CompatError) as caught:
        compat.snapshot_from_dir(directory)
    assert isinstance(caught.value.__cause__, UnicodeDecodeError)


def test_unreadable_allow_file_is_a_chained_read_error(tmp_path):
    path = tmp_path / "allow.toml"
    path.write_bytes(b"\xff\xfe")
    with pytest.raises(compat.CompatError) as caught:
        compat.load_allow(path)
    assert isinstance(caught.value.__cause__, UnicodeDecodeError)


@pytest.mark.parametrize("unreadable", ["new-schema", "baseline-schema", "allow-file"])
def test_cli_exits_2_for_an_unreadable_input(tmp_path, unreadable):
    good = write_dir(tmp_path / "good", {FILE: schema()})
    bad = write_dir(tmp_path / "bad", {FILE: schema()})
    (bad / FILE).write_bytes(b"\xff\xfe")
    allow = tmp_path / "allow.toml"
    allow.write_bytes(b"\xff\xfe")
    args = {
        "new-schema": ["--baseline-dir", str(good), "--schemas-dir", str(bad)],
        "baseline-schema": ["--baseline-dir", str(bad), "--schemas-dir", str(good)],
        "allow-file": [
            "--baseline-dir",
            str(good),
            "--schemas-dir",
            str(good),
            "--allow-file",
            str(allow),
        ],
    }[unreadable]
    assert compat.main([*args, "--repo", str(tmp_path)]) == 2


SCALAR_AS_BOOL = {
    "const-1-to-true": ({"const": 1}, {"const": True}, "const"),
    "const-0-to-false": ({"const": 0}, {"const": False}, "const"),
    "const-true-to-1": ({"const": True}, {"const": 1}, "const"),
    "enum-member-1-to-true": ({"enum": [1, 2]}, {"enum": [True, 2]}, "enum"),
    "enum-member-0-to-false": ({"enum": ["a", 0]}, {"enum": ["a", False]}, "enum"),
    "nested-enum-member": ({"enum": [[1]]}, {"enum": [[True]]}, "enum"),
}


@pytest.mark.parametrize(
    ("old", "new", "keyword"), SCALAR_AS_BOOL.values(), ids=SCALAR_AS_BOOL.keys()
)
def test_bool_is_not_equal_to_the_matching_number(tmp_path, old, new, keyword):
    findings = run(
        tmp_path, schema(properties={"x": old}), schema(properties={"x": new})
    )
    assert kinds(findings) == [(f"/properties/x/{keyword}", f"{keyword}-narrowed")]


JSON_EQUAL = {
    "int-to-float": ({"const": 1}, {"const": 1.0}),
    "enum-reordered-members": ({"enum": [1, 2]}, {"enum": [2.0, 1]}),
}


@pytest.mark.parametrize(("old", "new"), JSON_EQUAL.values(), ids=JSON_EQUAL.keys())
def test_numerically_equal_values_are_not_a_change(tmp_path, old, new):
    findings = run(
        tmp_path, schema(properties={"x": old}), schema(properties={"x": new})
    )
    assert findings == []


MEMBER = {"properties": {"gone": {"type": "string"}}}
DELETED_SUBTREE = {
    "items": (
        schema(items=MEMBER),
        schema(),
        "/items/properties/gone",
        "property-removed",
    ),
    "then": (
        schema(**{"if": {"required": ["k"]}, "then": MEMBER}),
        schema(**{"if": {"required": ["k"]}}),
        "/then/properties/gone",
        "property-removed",
    ),
    "else": (
        schema(**{"if": {"required": ["k"]}, "else": MEMBER}),
        schema(**{"if": {"required": ["k"]}}),
        "/else/properties/gone",
        "property-removed",
    ),
    "allOf-member": (
        schema(allOf=[{}, MEMBER]),
        schema(allOf=[{}]),
        "/allOf/1/properties/gone",
        "property-removed",
    ),
    "deleted-def-inside-items": (
        schema(items={"$defs": {"D": {}}}),
        schema(),
        "/items/$defs/D",
        "def-removed",
    ),
    "deleted-pattern-property": (
        schema(items={"patternProperties": {"^x-": {}}}),
        schema(),
        "/items/patternProperties/^x-",
        "pattern-property-removed",
    ),
}


@pytest.mark.parametrize(
    ("old", "new", "pointer", "kind"),
    DELETED_SUBTREE.values(),
    ids=DELETED_SUBTREE.keys(),
)
def test_removed_enclosing_schema_reports_what_it_described(
    tmp_path, old, new, pointer, kind
):
    assert kinds(run(tmp_path, old, new)) == [(pointer, kind)]


ADDED_ENTRIES = {
    "pattern-property-with-type": (
        {"patternProperties": {"^n-": {"type": "integer"}}},
        "/patternProperties/^n-/type",
        "type-narrowed",
    ),
    "pattern-property-with-bound": (
        {"patternProperties": {"^n-": {"minimum": 1}}},
        "/patternProperties/^n-/minimum",
        "bound-tightened",
    ),
    "pattern-property-with-required": (
        {"patternProperties": {"^n-": {"required": ["k"]}}},
        "/patternProperties/^n-/required/k",
        "required-added",
    ),
    "dependent-schema-with-required": (
        {"dependentSchemas": {"a": {"required": ["b"]}}},
        "/dependentSchemas/a/required/b",
        "required-added",
    ),
    "dependent-schema-with-enum": (
        {"dependentSchemas": {"a": {"enum": [1]}}},
        "/dependentSchemas/a/enum",
        "enum-narrowed",
    ),
}


@pytest.mark.parametrize(
    ("addition", "pointer", "kind"), ADDED_ENTRIES.values(), ids=ADDED_ENTRIES.keys()
)
def test_added_restrictive_entry_is_reported(tmp_path, addition, pointer, kind):
    assert kinds(run(tmp_path, schema(), schema(**addition))) == [(pointer, kind)]


EXEMPT_ADDITIONS = {
    "unrestricted-pattern-property": {"patternProperties": {"^x-": {}}},
    "unrestricted-dependent-schema": {"dependentSchemas": {"a": {}}},
    "new-def": {"$defs": {"D": {"type": "string", "minLength": 9}}},
    "new-optional-property": {"properties": {"p": {"type": "string", "minLength": 9}}},
}


@pytest.mark.parametrize(
    "addition", EXEMPT_ADDITIONS.values(), ids=EXEMPT_ADDITIONS.keys()
)
def test_allowed_addition_is_not_reported(tmp_path, addition):
    assert run(tmp_path, schema(), schema(**addition)) == []


MAYBE_NARROWING = {
    "anyOf-introduced": (schema(), schema(anyOf=[{"type": "object"}]), "/anyOf"),
    "oneOf-introduced": (schema(), schema(oneOf=[{"type": "object"}]), "/oneOf"),
    "oneOf-branch-appended": (
        schema(oneOf=[{"required": ["a"]}]),
        schema(oneOf=[{"required": ["a"]}, {"required": ["b"]}]),
        "/oneOf/1",
    ),
    "prefixItems-appended": (
        schema(prefixItems=[{"type": "string"}]),
        schema(prefixItems=[{"type": "string"}, {"type": "integer"}]),
        "/prefixItems/1",
    ),
}


@pytest.mark.parametrize(
    ("old", "new", "pointer"), MAYBE_NARROWING.values(), ids=MAYBE_NARROWING.keys()
)
def test_addition_that_may_narrow_fails_closed(tmp_path, old, new, pointer):
    assert kinds(run(tmp_path, old, new)) == [(pointer, "narrowing-possible")]


def test_narrowing_possible_is_suppressed_by_an_allow_entry(tmp_path):
    allow = entry(pointer="/oneOf/1", kind="narrowing-possible")
    findings = run(
        tmp_path,
        schema(oneOf=[{"required": ["a"]}]),
        schema(oneOf=[{"required": ["a"]}, {"required": ["b"]}]),
        allow=[allow],
    )
    assert findings == []


def test_each_added_required_field_has_its_own_finding(tmp_path):
    findings = run(tmp_path, schema(), schema(required=["a", "b"]))
    assert kinds(findings) == [
        ("/required/a", "required-added"),
        ("/required/b", "required-added"),
    ]


def test_allow_entry_for_one_required_field_leaves_the_others(tmp_path):
    allow = entry(pointer="/required/a", kind="required-added")
    findings = run(tmp_path, schema(), schema(required=["a", "b"]), allow=[allow])
    assert kinds(findings) == [("/required/b", "required-added")]


def test_deleted_definition_used_by_the_corpus_is_a_read_error(tmp_path):
    old = schema(**{"$defs": {"D": {"type": "string"}}})
    new = schema(**{"$defs": {}})
    corpus = {"thing/valid/D--ok.json": '"text"'}
    base = compat.snapshot_from_dir(write_dir(tmp_path / "old", {FILE: old}, corpus))
    new_dir = write_dir(tmp_path / "new", {FILE: new})
    with pytest.raises(compat.CompatError):
        compat.check(base, new_dir, [])


def test_cli_exits_2_when_the_corpus_uses_a_deleted_definition(tmp_path):
    old = write_dir(
        tmp_path / "old",
        {FILE: schema(**{"$defs": {"D": {"type": "string"}}})},
        {"thing/valid/D--ok.json": '"text"'},
    )
    new = write_dir(tmp_path / "new", {FILE: schema(**{"$defs": {}})})
    args = ["--baseline-dir", str(old), "--schemas-dir", str(new)]
    assert compat.main(args) == 2


# --- references resolve per side and by URI ----------------------------------------


def named(name: str, **defs: object) -> dict:
    return {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": f"https://example.test/schemas/v3/{name}",
        "$defs": defs,
    }


def test_ref_retargeted_between_unchanged_files_compares_each_sides_own_leaf(tmp_path):
    local = {"D": {"$ref": "#/$defs/Leaf"}}
    a = named("a.schema.json", **local, Leaf={"type": "string"})
    b = named("b.schema.json", **local, Leaf={"type": "string", "minLength": 5})
    files = {"a.schema.json": a, "b.schema.json": b}
    old = {**files, FILE: referrer("a.schema.json#/$defs/D")}
    new = {**files, FILE: referrer("b.schema.json#/$defs/D")}
    found = [(f.file, f.pointer, f.kind) for f in run_files(tmp_path, old, new)]
    assert found == [
        (
            FILE,
            (
                "/properties/p/$ref(b.schema.json#/$defs/D)"
                "/$ref(b.schema.json#/$defs/Leaf)/minLength"
            ),
            "bound-tightened",
        )
    ]


@pytest.mark.parametrize(
    "ref",
    [
        "https://example.test/schemas/other/common.schema.json#/$defs/D",
        "../other/common.schema.json#/$defs/D",
        "https://elsewhere.test/unknown.schema.json#/$defs/D",
    ],
    ids=["same-basename-outside-directory", "relative-outside-directory", "unknown"],
)
def test_ref_to_an_unregistered_uri_is_unverified(tmp_path, ref):
    files = {
        "common.schema.json": common(D={"type": "string"}),
        FILE: referrer(ref),
    }
    found = [(f.pointer, f.kind) for f in run_files(tmp_path, files, files)]
    assert found == [("/properties/p/$ref", "unverified")]


REMOVED_REF_TARGET = {
    "common.schema.json": common(D={"type": "string", "minLength": 1})
}


@pytest.mark.parametrize(
    ("inline", "expected"),
    [
        (
            {"type": "string", "minLength": 2},
            [
                (
                    "/properties/p/$ref(common.schema.json#/$defs/D)/minLength",
                    "bound-tightened",
                )
            ],
        ),
        ({"type": "string"}, []),
    ],
    ids=["inline-narrower-than-target", "inline-wider-than-target"],
)
def test_removed_ref_compares_the_old_target_with_the_inline_replacement(
    tmp_path, inline, expected
):
    old = {**REMOVED_REF_TARGET, FILE: referrer("common.schema.json#/$defs/D")}
    new = {**REMOVED_REF_TARGET, FILE: schema(properties={"p": inline})}
    assert kinds(run_files(tmp_path, old, new)) == expected


# --- baseline loaders --------------------------------------------------------------


def commit_repo(repo: Path, files: dict[str, bytes]) -> Path:
    import subprocess

    for rel, data in files.items():
        target = repo / "schemas" / "v3" / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
    git = ["git", "-C", str(repo), "-c", "user.name=t", "-c", "user.email=t@e.test"]
    subprocess.run([*git, "init", "-q"], check=True)
    subprocess.run([*git, "add", "."], check=True)
    subprocess.run([*git, "commit", "-q", "-m", "base"], check=True)
    return repo


NOT_VALID_CORPUS = {
    "conformance/thing/invalid/bad.json": b"\xff\xfe not utf-8",
    "conformance/thing/valid/nested/deep.json": b"\xff\xfe not utf-8",
    "conformance/stray.json": b"\xff\xfe not utf-8",
}


@pytest.mark.parametrize("loader", ["directory", "git"])
def test_only_valid_corpus_files_are_loaded(tmp_path, loader):
    files = {
        FILE: json.dumps(schema()).encode(),
        "conformance/thing/valid/ok.json": b"{}",
        **NOT_VALID_CORPUS,
    }
    if loader == "directory":
        root = tmp_path / "dir"
        for rel, data in files.items():
            (root / rel).parent.mkdir(parents=True, exist_ok=True)
            (root / rel).write_bytes(data)
        snapshot = compat.snapshot_from_dir(root)
    else:
        snapshot = compat.snapshot_from_ref(
            commit_repo(tmp_path / "repo", files), "HEAD"
        )
    assert snapshot.corpus == {"thing/valid/ok.json": "{}"}


BAD_BLOBS = {
    "schema": {FILE: b"\xff\xfe"},
    "valid-instance": {
        FILE: json.dumps(schema()).encode(),
        "conformance/thing/valid/x.json": b"\xff\xfe",
    },
}


@pytest.mark.parametrize("files", BAD_BLOBS.values(), ids=BAD_BLOBS.keys())
def test_git_blob_that_is_not_utf8_is_a_chained_read_error(tmp_path, files):
    repo = commit_repo(tmp_path / "repo", files)
    with pytest.raises(compat.CompatError) as caught:
        compat.snapshot_from_ref(repo, "HEAD")
    assert isinstance(caught.value.__cause__, UnicodeDecodeError)


@pytest.mark.parametrize("files", BAD_BLOBS.values(), ids=BAD_BLOBS.keys())
def test_cli_exits_2_for_a_git_blob_that_is_not_utf8(tmp_path, files):
    repo = commit_repo(tmp_path / "repo", files)
    assert compat.main(["--base", "HEAD", "--repo", str(repo)]) == 2


# --- recursion and required $id -----------------------------------------------------


def node_defs() -> dict:
    return {
        "Node": {
            "type": "object",
            "properties": {
                "label": {"type": "string", "minLength": 1},
                "child": {"$ref": "#/$defs/Node"},
            },
        }
    }


def test_recursive_ref_replaced_by_two_inline_levels_is_unverified(tmp_path):
    level2 = {
        "type": "object",
        "properties": {
            "label": {"type": "string", "minLength": 2},
            "child": {"$ref": "#/$defs/Node"},
        },
    }
    level1 = {
        "type": "object",
        "properties": {
            "label": {"type": "string", "minLength": 1},
            "child": level2,
        },
    }
    old = schema(properties={"p": {"$ref": "#/$defs/Node"}}, **{"$defs": node_defs()})
    new = schema(properties={"p": level1}, **{"$defs": node_defs()})
    assert kinds(run(tmp_path, old, new)) == [
        (
            "/properties/p/$ref(thing.schema.json#/$defs/Node)/properties/child/$ref",
            "unverified",
        )
    ]


def test_unverified_recursion_stop_is_suppressed_by_an_allow_entry(tmp_path):
    pointer = "/properties/p/$ref(thing.schema.json#/$defs/Node)/properties/child/$ref"
    inline = {
        "type": "object",
        "properties": {
            "label": {"type": "string", "minLength": 1},
            "child": {
                "type": "object",
                "properties": {"child": {"$ref": "#/$defs/Node"}},
            },
        },
    }
    old = schema(properties={"p": {"$ref": "#/$defs/Node"}}, **{"$defs": node_defs()})
    new = schema(properties={"p": inline}, **{"$defs": node_defs()})
    allow = entry(pointer=pointer, kind="unverified")
    assert run(tmp_path, old, new, allow=[allow]) == []


@pytest.mark.parametrize("keyword", ["enum", "const", "default"])
def test_portable_end_anchor_equivalence_is_limited_to_pattern(tmp_path, keyword):
    """A rule regex held in another keyword compiles with PCRE2, where `$` differs."""
    def body(regex: str) -> dict:
        value = [regex] if keyword == "enum" else regex
        return schema(properties={"s": {"type": "string", keyword: value}})

    findings = run(tmp_path, body("^a$"), body(f"^a{PORTABLE}"))
    assert findings != []


def test_unchanged_recursive_ref_still_passes(tmp_path):
    both = schema(properties={"p": {"$ref": "#/$defs/Node"}}, **{"$defs": node_defs()})
    assert run(tmp_path, both, both) == []


PORTABLE = r"(?![\s\S])"


@pytest.mark.parametrize(
    ("old", "new", "flagged"),
    [
        pytest.param("^[a-z]+$", f"^[a-z]+{PORTABLE}", False, id="trailing-dollar"),
        pytest.param(
            r"^(?!(.*/)?\.\.(/|$))",
            rf"^(?!(.*/)?\.\.(/|{PORTABLE}))",
            False,
            id="inner",
        ),
        pytest.param("^a$|^b$", f"^a{PORTABLE}|^b{PORTABLE}", False, id="alternation"),
        pytest.param(f"^[a-z]+{PORTABLE}", "^[a-z]+$", False, id="reverse"),
        pytest.param("^[a-z]+$", f"^[a-z0-9]+{PORTABLE}", True, id="anchor-and-class"),
        pytest.param("^[a-z]+$", "^[a-z]+", True, id="anchor-dropped"),
    ],
)
def test_portable_end_anchor_equals_dollar(tmp_path, old, new, flagged):
    findings = run(
        tmp_path,
        schema(properties={"s": {"pattern": old}}),
        schema(properties={"s": {"pattern": new}}),
    )
    assert kinds(findings) == (
        [("/properties/s/pattern", "pattern-changed")] if flagged else []
    )


@pytest.mark.parametrize("side", ["new", "baseline"])
def test_schema_without_id_is_a_read_error(tmp_path, side):
    with_id = write_dir(tmp_path / "with", {FILE: schema()})
    without = write_dir(tmp_path / "without", {})
    (without / FILE).write_text(json.dumps({"type": "object"}), encoding="utf-8")
    baseline, new = (with_id, without) if side == "new" else (without, with_id)
    assert (
        compat.main(["--baseline-dir", str(baseline), "--schemas-dir", str(new)]) == 2
    )
