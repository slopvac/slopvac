"""publish-lint.yml builds the 2.x pure-Python package and must never publish 3.x."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "publish-lint.yml"
SCRIPT = ROOT / "scripts" / "release_provenance.py"
JOBS = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))["jobs"]


def uploading_jobs() -> dict[str, dict]:
    return {name: job for name, job in JOBS.items() if "environment" in job}


def test_the_workflow_still_has_its_two_2x_upload_jobs():
    assert set(uploading_jobs()) == {"pypi-rc", "pypi"}


@pytest.mark.parametrize("name", ["pypi-rc", "pypi"])
def test_an_upload_job_requires_the_version_to_be_below_major_three(name):
    assert "needs.build.outputs.legacy == 'true'" in uploading_jobs()[name]["if"]


def test_the_legacy_flag_comes_from_the_version_check_script():
    step = next(
        s for s in JOBS["build"]["steps"] if s.get("id") == "legacy"
    )
    assert JOBS["build"]["outputs"]["legacy"] == "${{ steps.legacy.outputs.legacy }}"
    assert "release_provenance.py is-legacy" in step["run"]
    assert step["env"]["VERSION"] == "${{ steps.version.outputs.version }}"


@pytest.mark.parametrize(
    ("version", "flag"),
    [("2.19.3", "true"), ("2.19.3rc4", "true"), ("3.0.0", "false"), ("3.0.0rc1", "false")],
)
def test_the_script_the_workflow_calls_flags_3x_versions(version, flag):
    done = subprocess.run(
        [sys.executable, str(SCRIPT), "is-legacy", version],
        capture_output=True,
        text=True,
        check=True,
    )
    assert done.stdout == f"{flag}\n"
