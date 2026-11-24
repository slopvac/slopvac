"""The Rust pin check fails when mise.toml and rust-toolchain.toml disagree."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "ci" / "check_rust_pins.py"

TOOLCHAIN = (
    '[toolchain]\nchannel = "9.8.7"\ncomponents = ["clippy", "rustfmt"]\n'
    'profile = "minimal"\n'
)


def run_check(root: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), str(root)],
        capture_output=True,
        text=True,
        check=False,
    )


def write_pins(tmp_path: Path, mise_rust: str, toolchain: str = TOOLCHAIN) -> Path:
    (tmp_path / "rust-toolchain.toml").write_text(toolchain)
    (tmp_path / "mise.toml").write_text(f"[tools]\nrust = {mise_rust}\n")
    return tmp_path


BARE_TOOLCHAIN = '[toolchain]\nchannel = "9.8.7"\n'


@pytest.mark.parametrize(
    ("mise_rust", "toolchain"),
    [
        pytest.param('"9.8.7"', BARE_TOOLCHAIN, id="bare-pin-both-sides-default"),
        pytest.param('{ version = "9.8.7" }', BARE_TOOLCHAIN, id="table-without-options"),
        pytest.param(
            '{ version = "9.8.7", profile = "default" }',
            BARE_TOOLCHAIN,
            id="explicit-default-profile",
        ),
    ],
)
def test_omitted_fields_default_on_both_sides(
    tmp_path: Path, mise_rust: str, toolchain: str
) -> None:
    result = run_check(write_pins(tmp_path, mise_rust, toolchain))
    assert (result.returncode, result.stdout, result.stderr) == (0, "", "")


@pytest.mark.parametrize(
    "mise_rust",
    [
        pytest.param('"9.8.6"', id="bare-version"),
        pytest.param('{ version = "9.8.6" }', id="table-version"),
    ],
)
def test_version_mismatch_fails_when_other_fields_default(
    tmp_path: Path, mise_rust: str
) -> None:
    result = run_check(write_pins(tmp_path, mise_rust, BARE_TOOLCHAIN))
    assert result.returncode == 1
    assert "differs from rust-toolchain.toml" in result.stderr


@pytest.mark.parametrize(
    "mise_rust",
    [
        pytest.param(
            '{ version = "9.8.7", components = "clippy,rustfmt", profile = "minimal" }',
            id="comma-string",
        ),
        pytest.param(
            '{ version = "9.8.7", components = ["rustfmt", "clippy"], profile = "minimal" }',
            id="array",
        ),
    ],
)
def test_matching_pins_pass_quietly(tmp_path: Path, mise_rust: str) -> None:
    result = run_check(write_pins(tmp_path, mise_rust))
    assert (result.returncode, result.stdout, result.stderr) == (0, "", "")


@pytest.mark.parametrize(
    "mise_rust",
    [
        pytest.param(
            '{ version = "9.8.6", components = "clippy,rustfmt", profile = "minimal" }',
            id="version",
        ),
        pytest.param(
            '{ version = "9.8.7", components = "clippy", profile = "minimal" }',
            id="components",
        ),
        pytest.param(
            '{ version = "9.8.7", components = "clippy,rustfmt", profile = "default" }',
            id="profile",
        ),
        pytest.param('"9.8.7"', id="bare-version-without-components"),
    ],
)
def test_differing_pins_fail(tmp_path: Path, mise_rust: str) -> None:
    result = run_check(write_pins(tmp_path, mise_rust))
    assert result.returncode == 1
    assert "differs from rust-toolchain.toml" in result.stderr


def test_repository_pins_agree() -> None:
    result = run_check(ROOT)
    assert result.returncode == 0, result.stderr
