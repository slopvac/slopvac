"""Fail when slopvac-migrate's 2.x mapping data drifts from the contract.

`crates/slopvac-migrate` bundles the rename table, the `core:legacy-<profile>` styles,
the 2.x rule catalogue and the `--preserve-behavior` vectors. Each is generated from
`docs/contract/compat/profiles/`, the 2.x ruleset, the rule converter and
`tests/contract/preserve_expansion.py`. This test fails until the files are regenerated
with `crates/slopvac-migrate/tests/fixtures/generate.py`.

The other tests run the generator on a copy of the generated files (`--out`), so they
never touch the crate.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

import pytest

CRATE = Path(__file__).resolve().parents[2] / "crates/slopvac-migrate"
SCRIPT = CRATE / "tests/fixtures/generate.py"
# Every file the generator writes, relative to the crate.
OUTPUTS = (
    "data/renames.toml",
    "data/legacy/catalogue.toml",
    "data/legacy/legacy-strict.toml",
    "data/legacy/legacy-normal.toml",
    "data/legacy/legacy-relaxed.toml",
    "tests/fixtures/preserve/refused-blocklist.json",
    "tests/fixtures/preserve/vectors.json",
)


def _generate(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        capture_output=True,
        text=True,
        check=False,
    )


def _snapshot(root: Path) -> dict[str, bytes]:
    return {
        p.relative_to(root).as_posix(): p.read_bytes()
        for p in sorted(root.rglob("*"))
        if p.is_file()
    }


@pytest.fixture
def copy(tmp_path: Path) -> Path:
    """The committed outputs, copied into a scratch crate layout."""
    for name in OUTPUTS:
        dest = tmp_path / name
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(CRATE / name, dest)
    return tmp_path


def test_migrate_mapping_data_is_current() -> None:
    result = _generate("--check")
    assert result.returncode == 0, result.stderr or result.stdout


def _make_stale(root: Path) -> None:
    path = root / "data/renames.toml"
    path.write_text(path.read_text(encoding="utf-8") + "# edited\n", encoding="utf-8")


def _make_missing(root: Path) -> None:
    (root / "data/legacy/catalogue.toml").unlink()


@pytest.mark.parametrize(
    ("damage", "name"),
    [(_make_stale, "data/renames.toml"), (_make_missing, "data/legacy/catalogue.toml")],
    ids=["stale", "missing"],
)
def test_check_names_the_file_and_changes_nothing(
    copy: Path, damage, name: str
) -> None:
    damage(copy)
    before = _snapshot(copy)
    result = _generate("--check", "--out", str(copy))
    assert result.returncode == 1, result.stderr or result.stdout
    assert name in result.stderr
    others = [o for o in OUTPUTS if o != name]
    assert not any(o in result.stderr for o in others), result.stderr
    assert _snapshot(copy) == before


@pytest.mark.parametrize(
    "damage", [_make_stale, _make_missing], ids=["stale", "missing"]
)
def test_regenerating_makes_check_pass(copy: Path, damage) -> None:
    damage(copy)
    assert _generate("--out", str(copy)).returncode == 0
    result = _generate("--check", "--out", str(copy))
    assert result.returncode == 0, result.stderr or result.stdout
    assert _snapshot(copy) == {name: (CRATE / name).read_bytes() for name in OUTPUTS}
