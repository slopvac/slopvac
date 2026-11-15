"""test.yml routing: path classes, the Tests gate and the folded-in workflows agree."""

from __future__ import annotations

from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github" / "workflows"
# BaseLoader keeps every scalar a string, so `on` stays a key and not a boolean.
TEST = yaml.load((WORKFLOWS / "test.yml").read_text(), Loader=yaml.BaseLoader)
CLASSES = yaml.safe_load((ROOT / ".github" / "ci-paths.yml").read_text())
JOBS = TEST["jobs"]
GATE_SCRIPT = JOBS["gate"]["steps"][0]["run"]


def load(name: str) -> dict:
    return yaml.load((WORKFLOWS / name).read_text(), Loader=yaml.BaseLoader)


def test_every_path_class_is_a_changes_output():
    assert set(JOBS["changes"]["outputs"]) - {"force_all"} == set(CLASSES)


def test_gate_needs_every_routed_job():
    routed = set(JOBS) - {"gate", "lint-gate-compat", "security-gate-compat"}
    assert set(JOBS["gate"]["needs"]) == routed


@pytest.mark.parametrize("job", ["quick-lint", "evals-ratchet", "docs-site"])
def test_gate_expects_a_result_for_the_job(job):
    assert f'"{job}":' in GATE_SCRIPT


@pytest.mark.parametrize(
    ("job", "workflow", "klass"),
    [
        ("evals-ratchet", "evals-ratchet.yml", "evals"),
        ("docs-site", "docs-site.yml", "docs-site"),
    ],
)
def test_folded_workflow_is_called_on_its_class_only(job, workflow, klass):
    call = JOBS[job]
    assert call["uses"] == f"./.github/workflows/{workflow}"
    assert (
        f"outputs['{klass}'] == 'true'" in call["if"]
        or f"outputs.{klass} == 'true'" in call["if"]
    )
    assert "force_all == 'true'" in call["if"]
    assert call["permissions"] == {"contents": "read"}


@pytest.mark.parametrize("workflow", ["evals-ratchet.yml", "docs-site.yml"])
def test_folded_workflow_has_no_standalone_pr_or_push_trigger(workflow):
    triggers = load(workflow)["on"]
    assert "workflow_call" in triggers
    assert not {"pull_request", "pull_request_target", "push"} & set(triggers)


def test_pages_permissions_exist_only_on_the_main_deploy_job():
    for name in ("docs-site.yml", "docs-deploy.yml"):
        for job_name, job in load(name)["jobs"].items():
            perms = job.get("permissions") or {}
            if "pages" in perms or "id-token" in perms:
                assert (name, job_name) == ("docs-deploy.yml", "deploy")
                assert "refs/heads/main" in job["if"]
    triggers = load("docs-deploy.yml")["on"]
    assert "push" in triggers
    assert "pull_request" not in triggers
