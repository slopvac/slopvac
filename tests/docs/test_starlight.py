"""Behaviour of the Starlight adaptation, through the build CLI."""

from __future__ import annotations

import json
import shutil
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "docs"))

import build_site

REAL_SCHEMAS = ROOT / "schemas" / "v3"
PREFIX = "https://srobroek.github.io/slopvac/schemas/v3/"


@pytest.fixture
def site_dir(tmp_path: Path) -> Path:
    rules = tmp_path / "rules"
    rules.mkdir()
    (rules / "demo.yml").write_text(
        "id: demo\ntitle: Demo\ndescription: A demo.\nrules:\n"
        "  - id: no-foo\n    name: Avoid foo\n    kind: pattern\n"
        "    message: 'foo \"quoted\"'\n"
    )
    schemas = tmp_path / "schemas"
    schemas.mkdir()
    (schemas / "a.schema.json").write_text(
        json.dumps({"$id": f"{PREFIX}a.schema.json", "description": "d", "$defs": {}})
    )
    for name in ("plugin-protocol", "pack-manifest", "common"):
        shutil.copy(REAL_SCHEMAS / f"{name}.schema.json", schemas)
    packs = tmp_path / "packs" / "demo"
    packs.mkdir(parents=True)
    (packs / "slopvac-pack.toml").write_text(
        'schema_version = 3\nid = "demo"\nversion = "1.0.0"\nslopvac_api = 1\n'
        'kind = "data"\n'
    )
    config = tmp_path / "site.toml"
    config.write_text(
        'site_url = "https://docs.example.org/tools/"\napi_version = "v3"\n'
    )
    site = tmp_path / "site"
    code = build_site.main(
        [
            "--out", str(tmp_path / "out"),
            "--config", str(config),
            "--rules-dir", str(rules),
            "--schemas-dir", str(schemas),
            "--packs-dir", str(tmp_path / "packs"),
            "--starlight", str(site),
        ]
    )  # fmt: skip
    assert code == 0
    return site


def _page(site: Path, route: str) -> str:
    return (site / "src" / "content" / "docs" / "v3" / route).read_text()


def test_title_moves_into_front_matter(site_dir: Path) -> None:
    page = _page(site_dir, "rules/demo/no-foo.md")
    assert page.startswith('---\ntitle: "Avoid foo"\n---\n')
    assert "\n# Avoid foo" not in page


def test_links_become_route_relative_for_trailing_slash_urls(site_dir: Path) -> None:
    assert "(../)" in _page(site_dir, "rules/demo/no-foo.md")
    assert "(no-foo/)" in _page(site_dir, "rules/demo/index.md")
    assert "(demo/)" in _page(site_dir, "rules/index.md")
    assert "(rules/)" in _page(site_dir, "index.md")


def test_schema_index_links_to_the_raw_files_by_relative_path(site_dir: Path) -> None:
    index = _page(site_dir, "schemas/index.md")
    assert "(../../schemas/v3/a.schema.json)" in index
    published = json.loads((site_dir / "public/schemas/v3/a.schema.json").read_text())
    assert published["$id"] == "https://docs.example.org/tools/schemas/v3/a.schema.json"


def test_site_config_splits_origin_and_base(site_dir: Path) -> None:
    config = json.loads((site_dir / "generated" / "site.json").read_text())
    assert config == {"site": "https://docs.example.org", "base": "/tools"}


def test_stale_generated_pages_are_removed_on_rebuild(site_dir: Path) -> None:
    stale = site_dir / "src/content/docs/v3/old.md"
    stale.write_text("x")
    (site_dir / "public/schemas/v3/old.schema.json").write_text("{}")
    build_site.main(
        [
            "--out", str(site_dir.parent / "out"),
            "--config", str(site_dir.parent / "site.toml"),
            "--rules-dir", str(site_dir.parent / "rules"),
            "--schemas-dir", str(site_dir.parent / "schemas"),
            "--starlight", str(site_dir),
        ]
    )  # fmt: skip
    assert not stale.exists()
    assert not (site_dir / "public/schemas/v3/old.schema.json").exists()


def test_pack_pages_keep_the_published_packs_urls(site_dir: Path) -> None:
    # /v3/plugins/packs/ lists the pack, and each pack sits one level below it.
    assert "(demo/)" in _page(site_dir, "plugins/packs/index.md")
    page = _page(site_dir, "plugins/packs/demo/index.md")
    assert "(../../manifest/)" in page
    assert "(../../protocol/)" in page
