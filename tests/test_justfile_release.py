"""`just release` sends each version to the workflow that can publish it."""

from __future__ import annotations

import os
import shutil
import stat
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
JUST = shutil.which("just")
pytestmark = pytest.mark.skipif(JUST is None, reason="just is not installed")

FAKE = """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$FAKE_LOG"
if [[ "$(basename "$0")" == gh ]]; then echo "https://example.test/run/1"; fi
exit "${FAKE_EXIT:-0}"
"""


def release(tmp_path: Path, version: str, tag_exists: bool = True):
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    for name in ("git", "gh"):
        tool = bin_dir / name
        tool.write_text(FAKE)
        tool.chmod(tool.stat().st_mode | stat.S_IEXEC)
    log = tmp_path / "calls.log"
    env = {
        **os.environ,
        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
        "FAKE_LOG": str(log),
        "FAKE_EXIT": "0" if tag_exists else "2",
    }
    done = subprocess.run(
        [JUST, "release", version],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    calls = log.read_text().splitlines() if log.exists() else []
    return done, [c for c in calls if not c.startswith("ls-remote")]


@pytest.mark.parametrize("version", ["3.0.0", "3.4.12", "10.0.1"])
def test_version_three_and_later_dispatch_the_wheel_publisher_with_its_tag(
    tmp_path, version
):
    done, calls = release(tmp_path, version)
    assert done.returncode == 0, done.stderr
    assert calls == [
        f"workflow run publish-pypi.yml --ref v{version} -f tag=v{version} -f dry-run=false"
    ]
    assert done.stdout.splitlines() == [
        "https://example.test/run/1",
        "Run checks for a passed candidate, then waits for the pypi environment approval.",
    ]


@pytest.mark.parametrize("version", ["2.19.3", "2.0.0", "0.9.1"])
def test_earlier_versions_dispatch_the_python_publisher(tmp_path, version):
    done, calls = release(tmp_path, version)
    assert done.returncode == 0, done.stderr
    assert calls == [
        f"workflow run publish-lint.yml --ref v{version} -f dry-run=false -f target=pypi"
    ]
    assert done.stdout.splitlines()[1] == "Run waits for the pypi environment approval."


@pytest.mark.parametrize("version", ["3.0", "v3.0.0", "03.0.0", "3.0.0-rc.1", "x"])
def test_a_version_that_is_not_plain_xyz_dispatches_nothing(tmp_path, version):
    done, calls = release(tmp_path, version)
    assert done.returncode == 2
    assert calls == []


def test_a_missing_tag_dispatches_nothing(tmp_path):
    done, calls = release(tmp_path, "3.0.0", tag_exists=False)
    assert done.returncode == 1
    assert "does not exist on origin" in done.stderr
    assert calls == []
