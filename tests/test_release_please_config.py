from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def _extra_files() -> list[dict]:
    config = json.loads(
        (ROOT / "release-please-config.json").read_text(encoding="utf-8")
    )
    return [f for f in config["packages"]["."]["extra-files"] if isinstance(f, dict)]


def test_every_workspace_crate_version_is_bumped_in_cargo_lock():
    crates = {
        p.name for p in (ROOT / "crates").iterdir() if (p / "Cargo.toml").is_file()
    }
    bumped = {
        f["jsonpath"].split("'")[1] for f in _extra_files() if f["path"] == "Cargo.lock"
    }
    assert crates == bumped


def test_workspace_version_is_bumped_in_cargo_toml():
    paths = {(f["path"], f["jsonpath"]) for f in _extra_files() if f["type"] == "toml"}
    assert ("Cargo.toml", "$.workspace.package.version") in paths
