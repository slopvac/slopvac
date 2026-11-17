"""publish-lint.yml builds the 2.x pure-Python package and must never publish 3.x."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import pytest
import yaml

from tests.workflow_expr import run_jobs

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "publish-lint.yml"
SCRIPT = ROOT / "scripts" / "release_provenance.py"
JOBS = yaml.load(WORKFLOW.read_text(encoding="utf-8"), Loader=yaml.BaseLoader)["jobs"]

STABLE_DISPATCH = {
    "github.event_name": "workflow_dispatch",
    "github.ref_type": "tag",
    "inputs.dry-run": False,
    "inputs.target": "pypi",
}
RC_PUSH = {"github.event_name": "push", "github.ref_type": "branch"}


def uploads(context: dict, **build: object) -> set[str]:
    ran = run_jobs(JOBS, context, {"build": build})
    return ran & {"pypi", "pypi-rc"}


@pytest.mark.parametrize(
    ("context", "rc", "job"),
    [(STABLE_DISPATCH, "false", "pypi"), (RC_PUSH, "true", "pypi-rc")],
    ids=["stable", "release-candidate"],
)
def test_a_version_below_major_three_uploads(context, rc, job):
    assert uploads(context, legacy="true", rc=rc) == {job}


@pytest.mark.parametrize(
    ("context", "rc"),
    [(STABLE_DISPATCH, "false"), (RC_PUSH, "true")],
    ids=["stable", "release-candidate"],
)
@pytest.mark.parametrize("legacy", ["false", "", None], ids=["3x", "empty", "missing"])
def test_a_version_from_major_three_or_an_unknown_one_never_uploads(
    context, rc, legacy
):
    build = {"rc": rc} if legacy is None else {"rc": rc, "legacy": legacy}
    assert uploads(context, **build) == set()


def test_a_dry_run_never_uploads():
    dry = {**STABLE_DISPATCH, "inputs.dry-run": True}
    assert uploads(dry, legacy="true", rc="false") == set()


def test_the_legacy_flag_comes_from_the_version_check_script():
    step = next(s for s in JOBS["build"]["steps"] if s.get("id") == "legacy")
    assert JOBS["build"]["outputs"]["legacy"] == "${{ steps.legacy.outputs.legacy }}"
    assert "release_provenance.py is-legacy" in step["run"]
    assert step["env"]["VERSION"] == "${{ steps.version.outputs.version }}"


@pytest.mark.parametrize("job", ["pypi-rc", "pypi"])
def test_an_upload_job_checks_the_files_it_uploads_against_the_built_version(job):
    names = [s.get("name") for s in JOBS[job]["steps"]]
    check = next(
        s for s in JOBS[job]["steps"] if s.get("name") == "Check the distributions"
    )
    assert check["env"]["VERSION"] == "${{ needs.build.outputs.version }}"
    assert check["run"] == (
        "python3 scripts/release_provenance.py check-dist dist "
        '--version "$VERSION" --sdist --wheels 1'
    )
    assert (
        names.index("Download distributions")
        < names.index("Check the distributions")
        < names.index("Publish")
    )


@pytest.mark.parametrize(
    ("version", "flag"),
    [
        ("2.19.3", "true"),
        ("2.19.3rc4", "true"),
        ("3.0.0", "false"),
        ("3.0.0rc1", "false"),
    ],
)
def test_the_script_the_workflow_calls_flags_3x_versions(version, flag):
    done = subprocess.run(
        [sys.executable, str(SCRIPT), "is-legacy", version],
        capture_output=True,
        text=True,
        check=True,
    )
    assert done.stdout == f"{flag}\n"
