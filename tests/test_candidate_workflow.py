"""The candidate workflow and the stable gate must agree on what a candidate is."""

from __future__ import annotations

import fnmatch
import importlib.util
import json
import os
import re
import subprocess
import sys
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "candidate.yml"
SCRIPT = ROOT / "scripts" / "release_provenance.py"
SPEC = importlib.util.spec_from_file_location("release_provenance", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
provenance = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = provenance
SPEC.loader.exec_module(provenance)

# BaseLoader keeps every scalar a string, so `on` stays a key and not a boolean.
DOC = yaml.load(WORKFLOW.read_text(), Loader=yaml.BaseLoader)
TRIGGERS = DOC["on"]
JOBS = DOC["jobs"]
CANDIDATE_TAG = "rc/v3.0.0-rc.2"
SHA = "a" * 40


def step(job: str, name: str) -> dict:
    return next(s for s in JOBS[job]["steps"] if s.get("name") == name)


def test_pushing_a_candidate_tag_is_the_only_automatic_trigger():
    assert set(TRIGGERS) == {"push", "workflow_dispatch"}
    assert TRIGGERS["push"] == {"tags": ["rc/v*"]}


@pytest.mark.parametrize("tag", ["rc/v3.0.0-rc.1", "rc/v10.20.30-rc.12"])
def test_the_tag_filter_catches_every_candidate_tag(tag):
    provenance.parse_candidate_tag(tag)
    assert fnmatch.fnmatchcase(tag, TRIGGERS["push"]["tags"][0])


@pytest.mark.parametrize("tag", ["v3.0.0", "v3.0.0-rc.1", "release/v3.0.0-rc.1"])
def test_the_tag_filter_ignores_stable_and_foreign_tags(tag):
    assert not fnmatch.fnmatchcase(tag, TRIGGERS["push"]["tags"][0])


def test_dispatch_defaults_to_a_dry_run():
    inputs = TRIGGERS["workflow_dispatch"]["inputs"]
    assert inputs["dry-run"]["default"] == "true"
    assert inputs["tag"]["required"] == "true"


def test_no_run_script_interpolates_an_expression():
    scripts = [
        (job, s.get("name", "?"), s["run"])
        for job, body in JOBS.items()
        for s in body.get("steps", [])
        if "run" in s
    ]
    assert scripts, "expected run steps to check"
    assert [(job, name) for job, name, run in scripts if "${{" in run] == []


def test_the_tag_is_validated_first_and_only_through_the_environment():
    parse = JOBS["validate"]["steps"][1]
    assert parse["id"] == "parse"
    assert parse["env"] == {"TAG": "${{ inputs.tag || github.ref_name }}"}
    assert "validate" not in JOBS["validate"].get("needs", [])
    for job, body in JOBS.items():
        if job != "validate":
            assert "validate" in body["needs"], job


def test_workflow_default_permissions_grant_nothing():
    assert DOC["permissions"] == {}


def test_each_job_asks_for_the_least_it_needs():
    granted = {job: body["permissions"] for job, body in JOBS.items()}
    assert granted == {
        "validate": {"contents": "read"},
        "evals": {"contents": "read"},
        "wheels": {"contents": "read"},
        "archives": {"contents": "read"},
        "publish": {"id-token": "write"},
        "prerelease": {
            "contents": "write",
            "id-token": "write",
            "attestations": "write",
        },
    }


def test_publishing_waits_for_the_pypi_environment_and_every_build():
    publish = JOBS["publish"]
    assert publish["environment"]["name"] == "pypi"
    assert set(publish["needs"]) == {"validate", "evals", "wheels", "archives"}
    assert publish["if"] == "${{ needs.validate.outputs.publish == 'true' }}"
    assert set(JOBS["prerelease"]["needs"]) >= {"publish", "wheels", "archives"}


def test_every_external_action_is_pinned_by_commit():
    refs = [
        s["uses"]
        for body in JOBS.values()
        for s in body.get("steps", [])
        if "uses" in s
    ]
    assert refs
    unpinned = [r for r in refs if not re.fullmatch(r"[\w./-]+@[0-9a-f]{40}", r)]
    assert unpinned == []


def run_publish_decision(
    tmp_path: Path, **env: str
) -> subprocess.CompletedProcess[str]:
    script = step("validate", "Decide whether this run may publish")["run"]
    output = tmp_path / "github_output"
    output.write_text("")
    done = subprocess.run(
        ["bash", "-c", script],
        env={"PATH": os.environ["PATH"], "GITHUB_OUTPUT": str(output), **env},
        capture_output=True,
        text=True,
        check=False,
    )
    done.stdout = output.read_text().strip()
    return done


@pytest.mark.parametrize(
    ("env", "published"),
    [
        ({"EVENT": "push"}, "publish=true"),
        ({"EVENT": "workflow_dispatch", "DRY_RUN": "true"}, "publish=false"),
        (
            {
                "EVENT": "workflow_dispatch",
                "DRY_RUN": "false",
                "INPUT_TAG": CANDIDATE_TAG,
            },
            "publish=true",
        ),
    ],
)
def test_a_tag_push_or_a_real_dispatch_on_the_tag_publishes(tmp_path, env, published):
    base = {
        "DRY_RUN": "",
        "INPUT_TAG": "",
        "REF_TYPE": "tag",
        "REF_NAME": CANDIDATE_TAG,
    }
    done = run_publish_decision(tmp_path, **{**base, **env})
    assert done.returncode == 0, done.stderr
    assert done.stdout == published


@pytest.mark.parametrize(
    ("ref_type", "ref_name"),
    [("branch", "main"), ("branch", CANDIDATE_TAG), ("tag", "rc/v3.0.0-rc.9")],
)
def test_a_real_dispatch_run_must_start_on_the_input_tag(tmp_path, ref_type, ref_name):
    done = run_publish_decision(
        tmp_path,
        EVENT="workflow_dispatch",
        DRY_RUN="false",
        INPUT_TAG=CANDIDATE_TAG,
        REF_TYPE=ref_type,
        REF_NAME=ref_name,
    )
    assert done.returncode == 1
    assert "dry-run is off" in done.stderr
    assert done.stdout == ""


# The contract with scripts/release_provenance.py (check-stable).


def run_the_workflow_would_record(event: str = "push") -> dict:
    """The fields of the API run that a pushed candidate tag produces."""
    return {
        "event": event,
        "status": "completed",
        "conclusion": "success",
        "head_sha": SHA,
        "head_branch": CANDIDATE_TAG,
        "path": WORKFLOW.relative_to(ROOT).as_posix(),
    }


def gate_runner(runs: list[dict]):
    def runner(argv):
        if argv[:2] == ["git", "tag"]:
            return CANDIDATE_TAG
        if argv[:2] == ["git", "rev-parse"]:
            return "tree1" if argv[2].endswith("^{tree}") else SHA
        assert argv[:2] == ["gh", "api"]
        return json.dumps({"workflow_runs": runs})

    return runner


def test_the_gate_accepts_a_passed_push_run_of_this_workflow_file():
    runner = gate_runner([run_the_workflow_would_record()])
    assert provenance.check_stable("v3.0.0", "o/r", SHA, runner) == CANDIDATE_TAG


def test_the_gate_does_not_count_a_dispatch_run():
    runner = gate_runner([run_the_workflow_would_record(event="workflow_dispatch")])
    with pytest.raises(ValueError, match="no passed candidate"):
        provenance.check_stable("v3.0.0", "o/r", SHA, runner)


def test_the_parse_step_emits_the_outputs_the_jobs_read():
    lines = subprocess.run(
        [sys.executable, str(SCRIPT), "parse-tag", CANDIDATE_TAG],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.splitlines()
    emitted = {line.split("=", 1)[0] for line in lines}
    read = {
        name
        for key in ("cargo-version", "pypi-version")
        for name in re.findall(
            r"steps\.parse\.outputs\.(\w+)", JOBS["validate"]["outputs"][key]
        )
    }
    assert read == emitted == {"cargo", "pypi"}
