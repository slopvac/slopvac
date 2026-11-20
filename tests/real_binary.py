"""Helpers and fixtures that run the real ``crates/slopvac`` binary.

This is not a ``conftest.py``: a root ``tests/conftest.py`` would shadow the
plugin package's own ``conftest`` module, which its tests import by name. Tests
that need the fixtures import them from here.

Build it with ``cargo build -p slopvac``. The tests find it through
``SLOPVAC_BIN`` or ``target/{release,debug}/slopvac``. Without one they skip on
a developer machine. With ``SLOPVAC_REQUIRE_BIN=1`` (set in CI) a missing binary
fails the run, so the real-binary tests cannot skip there unnoticed. A
``SLOPVAC_BIN`` that points nowhere always fails.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from collections.abc import Mapping
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / ".github" / "action-fixtures"


def binary_required(env: Mapping[str, str]) -> bool:
    return env.get("SLOPVAC_REQUIRE_BIN") == "1"


def locate_binary(env: Mapping[str, str], root: Path) -> Path | None:
    """Return the built binary, ``None`` when there is none and it is not required."""
    declared = env.get("SLOPVAC_BIN")
    if declared:
        if not Path(declared).is_file():
            pytest.fail(f"SLOPVAC_BIN={declared!r} is not a file")
        return Path(declared)
    built = (root / "target" / profile / "slopvac" for profile in ("release", "debug"))
    found = next((path for path in built if path.is_file()), None)
    if found is None and binary_required(env):
        pytest.fail(
            "no built slopvac binary and SLOPVAC_REQUIRE_BIN=1; "
            "build it with `cargo build -p slopvac` and set SLOPVAC_BIN"
        )
    return found


@pytest.fixture(scope="session")
def slopvac_binary() -> Path:
    found = locate_binary(os.environ, ROOT)
    if found is None:
        pytest.skip(
            "no built slopvac binary, so the real-binary tests do not run; "
            "run `cargo build -p slopvac` first. "
            "SLOPVAC_REQUIRE_BIN=1 fails here instead of skipping"
        )
    return found


@pytest.fixture(scope="session")
def slop_project(tmp_path_factory) -> Path:
    """A 3.0 project with one sloppy and one clean document.

    It has its own config so the repository's 2.x ``slopvac.toml`` is never
    the nearest one.
    """
    project = tmp_path_factory.mktemp("slop-project")
    (project / "slopvac.toml").write_text(
        'schema_version = 3\nprofile = "normal"\n', encoding="utf-8"
    )
    shutil.copy(FIXTURES / "slop-laden.md", project / "slop.md")
    shutil.copy(FIXTURES / "clean.md", project / "clean.md")
    return project


@pytest.fixture(scope="session")
def lint_with_binary(slopvac_binary):
    """Run ``slopvac lint --format F --out FILE`` in a project; return the exit code."""

    def go(project: Path, fmt: str, out: Path, *targets: str) -> int:
        command = [str(slopvac_binary), "--offline", "lint", "--format", fmt]
        command += ["--out", str(out), "--", *targets]
        return subprocess.run(command, cwd=project, check=False).returncode

    return go
