"""Required checks must mean the same thing in a merge queue run as in a PR run.

A queue entry runs on ``refs/heads/gh-readonly-queue/main/pr-N-<sha>`` with a
``merge_group`` payload: no ``pull_request`` object, and the diff base is
``merge_group.base_sha``.
"""

from __future__ import annotations

from pathlib import Path

import pytest
import yaml

WORKFLOWS = Path(__file__).resolve().parents[1] / ".github" / "workflows"


def load(name: str) -> dict:
    # BaseLoader keeps `on` a string key instead of the boolean True.
    return yaml.load((WORKFLOWS / name).read_text(), Loader=yaml.BaseLoader)


def test_ci_runs_on_merge_group_checks_requested():
    assert load("test.yml")["on"]["merge_group"] == {"types": ["checks_requested"]}


def test_concurrency_group_is_unique_per_queue_entry():
    group = load("test.yml")["concurrency"]["group"]
    assert group == "ci-${{ github.event.pull_request.number || github.ref }}"

    def key(pull_request_number: str, ref: str) -> str:
        # Evaluate `a || b` as the expression language does: the first truthy operand.
        return "ci-" + (pull_request_number or ref)

    pr_run = key("7", "refs/pull/7/merge")
    entry_a = key("", "refs/heads/gh-readonly-queue/main/pr-7-aaaa")
    entry_b = key("", "refs/heads/gh-readonly-queue/main/pr-8-bbbb")
    assert pr_run == "ci-7"
    assert len({pr_run, entry_a, entry_b}) == 3


@pytest.mark.parametrize("workflow", ["test.yml", "lint.yml", "security.yml", "cli-e2e.yml"])
def test_no_job_condition_excludes_merge_group_by_ref_or_pull_request_only(workflow):
    for name, job in load(workflow)["jobs"].items():
        condition = str(job.get("if", ""))
        assert "refs/heads/main" not in condition, name
        if "'pull_request'" in condition:
            assert "'merge_group'" in condition, name


def test_agnix_diff_base_uses_the_merge_group_base():
    lint = load("lint.yml")["jobs"]["workflow-lint"]
    env = next(s for s in lint["steps"] if s.get("name") == "Check PR agnix inputs")["env"]
    assert " ".join(env["BASE_SHA"].split()) == (
        "${{ github.event.pull_request.base.sha || "
        "github.event.merge_group.base_sha || github.event.before }}"
    )


def test_secret_scan_of_a_queue_entry_is_bounded_by_the_group_range():
    steps = load("security.yml")["jobs"]["trufflehog"]["steps"]
    queue = next(s for s in steps if s["name"] == "Scan merge queue entry")
    assert queue["if"] == "github.event_name == 'merge_group'"
    assert queue["with"]["base"] == "${{ github.event.merge_group.base_sha }}"
    assert queue["with"]["head"] == "${{ github.event.merge_group.head_sha }}"
    pushed = next(s for s in steps if s["name"] == "Scan pushed commits")
    assert "github.event_name != 'merge_group'" in pushed["if"]


def test_security_gate_expects_dependency_review_on_merge_group():
    script = next(
        s for s in load("security.yml")["jobs"]["security-gate"]["steps"] if "run" in s
    )["run"]
    assert 'os.environ["EVENT_NAME"] in {"pull_request", "merge_group"}' in script
