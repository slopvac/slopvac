"""Behaviour of the generated docs site builder."""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "docs"))

import build_site


def _write_rules(directory: Path) -> None:
    directory.mkdir()
    (directory / "demo.yml").write_text(
        "id: demo\n"
        "title: Demo category\n"
        "description: A demo.\n"
        "rules:\n"
        "  - id: no-foo\n"
        "    name: Avoid foo\n"
        "    kind: pattern\n"
        "    severity: error\n"
        "    message: 'foo found: {match}'\n"
        "    tiers: {strict: enforced, normal: enforced, relaxed: excluded}\n"
        "    fix: Delete foo.\n"
        "    examples:\n"
        "      - bad: Use foo here.\n"
        "        good: Use bar here.\n"
        "    provenance: {source: Hand written, url: 'https://example.test/foo'}\n"
    )


def test_rule_page_lists_required_fields(tmp_path: Path) -> None:
    _write_rules(tmp_path / "rules")
    pages = build_site.generate_rules(tmp_path / "rules")
    page = pages["rules/demo/no-foo.md"]
    assert page.startswith("# Avoid foo\n")
    assert "| Severity | error |" in page
    assert "| Profile `relaxed` | excluded |" in page
    assert "foo found: <match>" in page
    assert "Flagged: `Use foo here.`" in page
    assert "Better: `Use bar here.`" in page
    assert "<https://example.test/foo>" in page


def test_category_index_links_each_rule(tmp_path: Path) -> None:
    _write_rules(tmp_path / "rules")
    pages = build_site.generate_rules(tmp_path / "rules")
    assert "[Avoid foo](no-foo.md): `demo.no-foo`" in pages["rules/demo/index.md"]
    assert "[Demo category](demo/index.md): 1 rules" in pages["rules/index.md"]


def test_rule_url_is_versioned_and_stable() -> None:
    assert (
        build_site.rule_url("demo", "no-foo")
        == "https://srobroek.github.io/slopvac/v3/rules/demo/no-foo.html"
    )


def test_inline_code_survives_backticks() -> None:
    assert build_site._code("use `x`") == "`` use `x` ``"


def test_broken_link_is_reported() -> None:
    pages = {
        "a/index.md": "[x](missing.md)",
        "a/b.md": "[ok](index.md) [up](../c.md)",
        "c.md": "",
    }
    assert build_site.broken_links(pages) == ["a/index.md: broken link missing.md"]


def test_schema_with_wrong_id_is_rejected(tmp_path: Path) -> None:
    (tmp_path / "x.schema.json").write_text(
        json.dumps({"$id": "https://elsewhere.test/x.schema.json"})
    )
    with pytest.raises(SystemExit, match="expected"):
        build_site.generate_schemas(tmp_path)


def test_committed_schemas_publish_at_their_id() -> None:
    pages = build_site.generate_schemas()
    assert "schemas/common.schema.json" in pages
    assert "schemas/index.md" in pages


def test_real_build_has_no_broken_links_and_a_page_per_rule() -> None:
    pages = build_site.build()
    assert build_site.broken_links(pages) == []
    expected = sum(len(c.get("rules", [])) for c in build_site.load_categories())
    rule_pages = [
        p for p in pages if p.startswith("rules/") and not p.endswith("index.md")
    ]
    assert len(rule_pages) == expected
    assert "- [Rules](rules/index.md)" in pages["SUMMARY.md"]
