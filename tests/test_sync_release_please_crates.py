from __future__ import annotations

import json
from pathlib import Path

import pytest

from scripts import sync_release_please_crates as sync


def _lock_entry(name: str) -> dict:
    return {
        "type": "toml",
        "path": "Cargo.lock",
        "jsonpath": f"$.package[?(@.name.value=='{name}')].version",
    }


def _write_workspace(
    root: Path, crates: list[str], entries: list[str], exclude: list[str] = ()
) -> None:
    excluded = ", ".join(f'"crates/{name}"' for name in exclude)
    (root / "Cargo.toml").write_text(
        f'[workspace]\nmembers = ["crates/*"]\nexclude = [{excluded}]\n'
    )
    for name in crates:
        crate = root / "crates" / name
        crate.mkdir(parents=True)
        (crate / "Cargo.toml").write_text(f'[package]\nname = "{name}"\n')
    config = {
        "packages": {
            ".": {
                "extra-files": [
                    "README.md",
                    *map(_lock_entry, entries),
                    {"type": "toml", "path": "Cargo.toml", "jsonpath": "$.v"},
                ]
            }
        }
    }
    (root / sync.CONFIG).write_text(json.dumps(config, indent=2) + "\n")


def _lock_names(root: Path) -> list[str]:
    config = json.loads((root / sync.CONFIG).read_text())
    return [
        e["jsonpath"].split("'")[1]
        for e in config["packages"]["."]["extra-files"]
        if isinstance(e, dict) and e["path"] == "Cargo.lock"
    ]


def test_in_sync_workspace_passes(tmp_path: Path, capsys) -> None:
    _write_workspace(tmp_path, ["a", "b"], ["a", "b"])
    assert sync.main(["--check", "--root", str(tmp_path)]) == 0
    assert capsys.readouterr().out == ""


def test_new_crate_without_entry_fails_with_regeneration_command(
    tmp_path: Path, capsys
) -> None:
    _write_workspace(tmp_path, ["a", "b", "c"], ["a", "b"])
    assert sync.main(["--check", "--root", str(tmp_path)]) == 1
    out = capsys.readouterr().out
    assert "name.value=='c'" in out
    assert f"Regenerate with: {sync.COMMAND}" in out


def test_regenerate_adds_entry_in_place_and_check_then_passes(tmp_path: Path) -> None:
    _write_workspace(tmp_path, ["a", "b", "c"], ["a", "c"])
    assert sync.main(["--root", str(tmp_path)]) == 0
    assert _lock_names(tmp_path) == ["a", "b", "c"]
    config = json.loads((tmp_path / sync.CONFIG).read_text())
    extra = config["packages"]["."]["extra-files"]
    assert extra[0] == "README.md"
    assert extra[-1]["path"] == "Cargo.toml"
    assert sync.main(["--check", "--root", str(tmp_path)]) == 0


def test_regenerate_drops_entry_for_removed_crate(tmp_path: Path) -> None:
    _write_workspace(tmp_path, ["a"], ["a", "gone"])
    assert sync.main(["--root", str(tmp_path)]) == 0
    assert _lock_names(tmp_path) == ["a"]


def test_repository_config_is_in_sync() -> None:
    assert sync.main(["--check"]) == 0


def test_excluded_crate_matched_by_member_glob_gets_no_entry(tmp_path: Path) -> None:
    _write_workspace(tmp_path, ["a", "skipped"], [], exclude=["skipped"])
    assert sync.main(["--root", str(tmp_path)]) == 0
    assert _lock_names(tmp_path) == ["a"]


def test_generated_entry_has_the_shape_release_please_expects(tmp_path: Path) -> None:
    _write_workspace(tmp_path, ["frontmatter-spans"], [])
    assert sync.main(["--root", str(tmp_path)]) == 0
    config = json.loads((tmp_path / sync.CONFIG).read_text())
    assert config["packages"]["."]["extra-files"][1] == {
        "type": "toml",
        "path": "Cargo.lock",
        "jsonpath": "$.package[?(@.name.value=='frontmatter-spans')].version",
    }
