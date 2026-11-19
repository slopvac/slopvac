"""A job may read a minisign signing secret only from inside a deployment environment.

The signing key is stored on environments, never on the repository, so a deployment
policy (the refs that may deploy) and optional reviewers guard every read. A job
without `environment:` falls back to repository secrets, which a workflow run on any
ref can read. This test parses each workflow and flags a `MINISIGN_*` secret reference
in a job that names no environment, wherever the reference sits: job or step `env`,
step `with`, `run`, or a reusable-workflow `secrets` map.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest
import yaml

WORKFLOWS = Path(__file__).resolve().parent.parent / ".github" / "workflows"

# `secrets.MINISIGN_X`, `secrets['MINISIGN_X']` and `secrets["MINISIGN_X"]`; the secrets
# context is case-insensitive.
SECRET_READ = re.compile(r"""secrets\s*(?:\.|\[\s*['"])\s*MINISIGN_""", re.IGNORECASE)
SECRET = "${{ secrets.MINISIGN_SECRET_KEY }}"


def load_workflows(directory: Path = WORKFLOWS) -> dict[str, dict]:
    # BaseLoader keeps every scalar a string, so `on` stays a key and not a boolean.
    paths = sorted([*directory.glob("*.yml"), *directory.glob("*.yaml")])
    return {p.name: yaml.load(p.read_text(), Loader=yaml.BaseLoader) for p in paths}


def reads_signing_secret(node: object) -> bool:
    """True when any string under `node` (keys included) names a MINISIGN secret."""
    if isinstance(node, str):
        return SECRET_READ.search(node) is not None
    if isinstance(node, dict):
        return any(
            reads_signing_secret(k) or reads_signing_secret(v) for k, v in node.items()
        )
    if isinstance(node, list):
        return any(reads_signing_secret(item) for item in node)
    return False


def has_environment(job: dict) -> bool:
    environment = job.get("environment")
    if isinstance(environment, dict):
        return bool(environment.get("name"))
    return bool(environment)


def violations(workflows: dict[str, dict]) -> list[str]:
    found = []
    for name, workflow in workflows.items():
        # Anything outside `jobs` (workflow-level env) reaches jobs with no environment.
        outside = {k: v for k, v in workflow.items() if k != "jobs"}
        if reads_signing_secret(outside):
            found.append(f"{name} reads a MINISIGN secret outside any job")
        for job_id, job in (workflow.get("jobs") or {}).items():
            if reads_signing_secret(job) and not has_environment(job):
                found.append(
                    f"{name}:{job_id} reads a MINISIGN secret without an environment"
                )
    return found


def test_every_job_that_reads_a_signing_secret_declares_an_environment():
    assert violations(load_workflows()) == []


def test_the_scan_sees_the_signing_jobs_it_is_meant_to_guard():
    readers = {
        f"{name}:{job_id}"
        for name, workflow in load_workflows().items()
        for job_id, job in workflow["jobs"].items()
        if reads_signing_secret(job)
    }
    assert {
        "release.yml:sign",
        "revocations.yml:build-sign",
        "candidate.yml:sign",
    } <= readers


def synthetic(job: dict, **workflow) -> dict[str, dict]:
    return {"x.yml": {"jobs": {"j": job}, **workflow}}


@pytest.mark.parametrize(
    "job",
    [
        pytest.param({"steps": [{"env": {"K": SECRET}}]}, id="step-env"),
        pytest.param({"env": {"K": SECRET}, "steps": []}, id="job-env"),
        pytest.param(
            {"steps": [{"uses": "a/b@1", "with": {"key": SECRET}}]}, id="step-with"
        ),
        pytest.param(
            {"steps": [{"run": 'echo "${{ secrets.MINISIGN_PASSWORD }}"'}]}, id="run"
        ),
        pytest.param(
            {"steps": [{"run": "echo ${{ secrets['MINISIGN_SECRET_KEY'] }}"}]},
            id="index",
        ),
        pytest.param(
            {"steps": [{"env": {"K": "${{ Secrets.minisign_secret_key }}"}}]},
            id="case",
        ),
        pytest.param(
            {"uses": "./.github/workflows/x.yml", "secrets": {"k": SECRET}},
            id="reusable",
        ),
        pytest.param(
            {"environment": "", "steps": [{"env": {"K": SECRET}}]},
            id="empty-environment",
        ),
    ],
)
def test_a_signing_secret_read_without_an_environment_is_reported(job):
    assert violations(synthetic(job)) == [
        "x.yml:j reads a MINISIGN secret without an environment"
    ]


@pytest.mark.parametrize(
    "environment",
    [
        pytest.param("release", id="string"),
        pytest.param({"name": "release"}, id="mapping"),
    ],
)
def test_a_signing_secret_read_inside_an_environment_is_allowed(environment):
    job = {"environment": environment, "steps": [{"env": {"K": SECRET}}]}
    assert violations(synthetic(job)) == []


def test_a_workflow_level_signing_secret_is_reported_even_with_job_environments():
    job = {"environment": "release", "steps": []}
    assert violations(synthetic(job, env={"K": SECRET})) == [
        "x.yml reads a MINISIGN secret outside any job"
    ]


def test_other_secrets_need_no_environment():
    job = {"steps": [{"env": {"T": "${{ secrets.GITHUB_TOKEN }}"}}]}
    assert violations(synthetic(job)) == []
