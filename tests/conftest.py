"""Fixtures that run the real ``crates/slopvac`` binary.

Build it with ``cargo build -p slopvac``. The tests find it through
``SLOPVAC_BIN`` or ``target/{release,debug}/slopvac``. They skip when none
exists, unless ``SLOPVAC_BIN`` is set: a path that points nowhere fails, so CI
cannot skip the real-binary tests by mistake.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / ".github" / "action-fixtures"


def _locate() -> Path | None:
    declared = os.environ.get("SLOPVAC_BIN")
    if declared:
        if not Path(declared).is_file():
            pytest.fail(f"SLOPVAC_BIN={declared!r} is not a file")
        return Path(declared)
    built = (ROOT / "target" / profile / "slopvac" for profile in ("release", "debug"))
    return next((path for path in built if path.is_file()), None)


@pytest.fixture(scope="session")
def slopvac_binary() -> Path:
    found = _locate()
    if found is None:
        pytest.skip("no built slopvac binary; run `cargo build -p slopvac`")
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
