"""Behaviour of the generated docs site builder, through its public interface."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "docs"))

import build_site

GENRES_YAML = ROOT / "crates/slopvac-context/src/builtin/genres.yaml"
OLD_PREFIX = "https://old.example/pkg/schemas/v3/"

RULES_YAML = """\
id: demo
title: Demo <category>
description: A demo.
rules:
  - id: no-foo
    name: Avoid foo
    kind: pattern
    severity: error
    message: 'foo found: {match} -- use {replacement}'
    tiers: {strict: enforced, normal: enforced, relaxed: excluded}
    fix: Delete foo.
    examples:
      - bad: Use foo here.
        good: Use bar here.
      - bad: Choose SHA-256, not MD5.
        note: 'preserve: instruction naming a real alternative'
      - bad: Remove this.
        good: ''
        note: The ending is dropped.
    provenance: {source: "Hand written \u2014 draft", url: 'https://example.test/foo'}
"""


def _schema(name: str, **extra: object) -> str:
    body = {"$id": OLD_PREFIX + name, "title": f"{name} title", **extra}
    return json.dumps(body, indent=2) + "\n"


@pytest.fixture
def sources(tmp_path: Path) -> dict[str, Path]:
    rules = tmp_path / "rules"
    rules.mkdir()
    (rules / "demo.yml").write_text(RULES_YAML)
    schemas = tmp_path / "schemas"
    schemas.mkdir()
    (schemas / "a.schema.json").write_text(
        _schema(
            "a.schema.json", properties={"b": {"$ref": OLD_PREFIX + "b.schema.json"}}
        )
    )
    (schemas / "b.schema.json").write_text(_schema("b.schema.json"))
    (schemas / "plugin-protocol.schema.json").write_text(
        _schema(
            "plugin-protocol.schema.json",
            description="Messages <of> v1.",
            **{
                "$defs": {
                    "Zed": {"type": "object", "description": "A | <b>"},
                    "Alpha": {"oneOf": []},
                }
            },
        )
    )
    config = tmp_path / "site.toml"
    config.write_text(
        'site_url = "https://docs.example.org/tools/"\napi_version = "v3"\n'
    )
    return {
        "rules": rules,
        "schemas": schemas,
        "config": config,
        "out": tmp_path / "out",
    }


def _run(sources: dict[str, Path], *extra: str) -> int:
    return build_site.main(
        [
            "--out", str(sources["out"]),
            "--config", str(sources["config"]),
            "--rules-dir", str(sources["rules"]),
            "--schemas-dir", str(sources["schemas"]),
            *extra,
        ]
    )  # fmt: skip


def _read(sources: dict[str, Path], path: str) -> str:
    return (sources["out"] / path).read_text()


def test_build_writes_rule_page_with_required_fields(sources: dict[str, Path]) -> None:
    assert _run(sources) == 0
    page = _read(sources, "rules/demo/no-foo.md")
    assert page.startswith("# Avoid foo\n")
    assert "| Severity | error |" in page
    assert "| Profile `relaxed` | `excluded` |" in page
    assert "Delete foo." in page
    assert "<https://example.test/foo>" in page


def test_unicode_dashes_in_catalogue_prose_become_ascii(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    page = _read(sources, "rules/demo/no-foo.md")
    assert "Hand written -- draft" in page
    assert "\u2014" not in page


def test_message_slots_render_as_visible_text_not_html_tags(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    page = _read(sources, "rules/demo/no-foo.md")
    assert "foo found: &lt;match&gt; -- use &lt;replacement&gt;" in page
    assert "<match>" not in page
    assert "Demo &lt;category&gt;" in _read(sources, "rules/demo/index.md")


def test_preserved_example_is_kept_as_written_with_its_reason(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    page = _read(sources, "rules/demo/no-foo.md")
    assert "### Example 1: flagged" in page
    assert "### Example 2: kept as written" in page
    assert "Reason: instruction naming a real alternative" in page
    kept = page.split("### Example 2")[1].split("### Example 3")[0]
    assert "Rewrite" not in kept


def test_empty_rewrite_and_note_are_rendered(sources: dict[str, Path]) -> None:
    _run(sources)
    page = _read(sources, "rules/demo/no-foo.md")
    assert "Rewrite: delete the text." in page
    assert "Note: The ending is dropped." in page


def test_category_and_root_indexes_link_each_page(sources: dict[str, Path]) -> None:
    _run(sources)
    assert "[Avoid foo](no-foo.md): `demo.no-foo`" in _read(
        sources, "rules/demo/index.md"
    )
    assert "(demo/index.md): 1 rules" in _read(sources, "rules/index.md")


def test_protocol_reference_lists_sorted_definitions_with_kinds(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    page = _read(sources, "plugins/protocol.md")
    assert "Messages &lt;of&gt; v1." in page
    assert page.index("`Alpha` (union)") < page.index("`Zed` (object): A | &lt;b&gt;")


def test_schemas_are_written_at_their_published_id_path(
    sources: dict[str, Path],
) -> None:
    assert _run(sources) == 0
    for name in ("a", "b", "plugin-protocol"):
        path = f"schemas/v3/{name}.schema.json"
        published = json.loads(_read(sources, path))
        assert published["$id"] == f"https://docs.example.org/tools/{path}"


def test_changing_site_url_rewrites_ids_and_cross_references(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    a = json.loads(_read(sources, "schemas/v3/a.schema.json"))
    assert (
        a["properties"]["b"]["$ref"]
        == "https://docs.example.org/tools/schemas/v3/b.schema.json"
    )
    assert OLD_PREFIX not in _read(sources, "schemas/v3/a.schema.json")


def test_schema_index_links_resolve(sources: dict[str, Path]) -> None:
    _run(sources)
    assert "[a.schema.json](v3/a.schema.json): a.schema.json title" in _read(
        sources, "schemas/index.md"
    )
    assert (sources["out"] / "schemas/v3/a.schema.json").is_file()


def test_schemas_with_mixed_id_prefixes_fail(sources: dict[str, Path]) -> None:
    (sources["schemas"] / "b.schema.json").write_text(
        json.dumps({"$id": "https://other.example/b.schema.json"})
    )
    with pytest.raises(SystemExit, match="different"):
        _run(sources)


def test_site_url_without_trailing_slash_fails(sources: dict[str, Path]) -> None:
    sources["config"].write_text(
        'site_url = "https://example.test/docs"\napi_version = "v3"\n'
    )
    with pytest.raises(SystemExit, match="slash"):
        _run(sources)


def test_rule_url_uses_the_configured_site(sources: dict[str, Path]) -> None:
    site = build_site.load_site(sources["config"])
    assert (
        build_site.rule_url("demo", "no-foo", site)
        == "https://docs.example.org/tools/v3/rules/demo/no-foo/"
    )


def test_check_passes_on_fresh_output_and_changes_nothing(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    before = {p: p.read_bytes() for p in sources["out"].rglob("*") if p.is_file()}
    assert _run(sources, "--check") == 0
    assert {
        p: p.read_bytes() for p in sources["out"].rglob("*") if p.is_file()
    } == before


def test_check_fails_on_edited_missing_and_obsolete_files(
    sources: dict[str, Path], capsys: pytest.CaptureFixture[str]
) -> None:
    _run(sources)
    (sources["out"] / "rules/demo/no-foo.md").write_text("edited\n")
    (sources["out"] / "styles/index.md").unlink()
    (sources["out"] / "old/page.md").parent.mkdir()
    (sources["out"] / "old/page.md").write_text("gone\n")
    assert _run(sources, "--check") == 1
    err = capsys.readouterr().err
    assert "rules/demo/no-foo.md: out of date" in err
    assert "styles/index.md: missing" in err
    assert "old/page.md: obsolete file" in err
    assert (sources["out"] / "old/page.md").exists()


def test_check_fails_when_nothing_was_generated(sources: dict[str, Path]) -> None:
    assert _run(sources, "--check") == 1
    assert not sources["out"].exists()


def test_broken_markdown_and_json_links_are_reported(sources: dict[str, Path]) -> None:
    pages = build_site.build(
        build_site.load_site(sources["config"]), sources["rules"], sources["schemas"]
    )
    pages["a/index.md"] = (
        "[x](missing.md) [y](../schemas/v3/none.json) [ok](../rules/index.md)"
    )
    assert build_site.broken_links(pages) == [
        "a/index.md: broken link missing.md",
        "a/index.md: broken link ../schemas/v3/none.json",
    ]


def test_real_data_builds_clean_with_a_page_per_rule_and_matching_ids() -> None:
    site = build_site.load_site()
    pages = build_site.build(site)
    assert build_site.broken_links(pages) == []
    assert build_site.schema_id_problems(pages, site) == []
    expected = sum(len(c.get("rules", [])) for c in build_site.load_categories())
    rule_pages = [
        p for p in pages if p.startswith("rules/") and not p.endswith("index.md")
    ]
    assert len(rule_pages) == expected


@pytest.mark.parametrize("preset", ["ste", "google"])
def test_every_bundled_style_preset_gets_a_generated_page(
    sources: dict[str, Path], preset: str
) -> None:
    assert _run(sources) == 0
    assert _read(sources, f"styles/{preset}.md").startswith(f"# Style: {preset}\n")


def test_styles_index_links_the_preset_pages_and_is_not_a_placeholder(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    index = _read(sources, "styles/index.md")
    assert "not generated yet" not in index
    assert "(ste.md)" in index
    assert "(google.md)" in index


def test_built_in_terms_page_is_generated_and_linked_from_the_root_index(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    assert _read(sources, "terms/index.md").startswith("# Built-in terms\n")
    assert "(terms/index.md)" in _read(sources, "index.md")


def test_a_failing_reference_example_stops_the_build_and_reports_its_stderr(
    sources: dict[str, Path], tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    fake_cargo = tmp_path / "fake-cargo"
    fake_cargo.write_text(
        "#!/bin/sh\necho 'boom: no such example xyzzy' >&2\nexit 101\n"
    )
    fake_cargo.chmod(0o755)
    build_site.rust_reference.cache_clear()
    monkeypatch.setattr(build_site, "CARGO", str(fake_cargo))
    try:
        with pytest.raises(SystemExit) as failure:
            _run(sources)
    finally:
        build_site.rust_reference.cache_clear()
    message = str(failure.value)
    assert "slopvac-styles" in message
    assert "boom: no such example xyzzy" in message


def test_genre_pages_come_from_the_registry_for_every_builtin_genre(
    sources: dict[str, Path],
) -> None:
    _run(sources)
    ids = [g["id"] for g in yaml.safe_load(GENRES_YAML.read_text())]
    written = {
        p.relative_to(sources["out"]).as_posix()
        for p in (sources["out"] / "genres").glob("*.md")
    }
    assert written == {"genres/index.md", *(f"genres/{i}.md" for i in ids)}
    index = _read(sources, "genres/index.md")
    assert "not generated yet" not in index
    assert all(f"({i}.md)" in index for i in ids)
    assert "Issue comments, discussions, blogs, drafts." in _read(
        sources, "genres/informal.md"
    )
