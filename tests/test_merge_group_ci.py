"""Required checks must mean the same thing in a merge queue run as in a PR run.

A queue entry runs on ``refs/heads/gh-readonly-queue/main/pr-N-<sha>`` with a
``merge_group`` payload: no ``pull_request`` object, and the diff base is
``merge_group.base_sha``.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
from pathlib import Path

import pytest
import yaml

WORKFLOWS = Path(__file__).resolve().parents[1] / ".github" / "workflows"


def load(name: str) -> dict:
    # BaseLoader keeps `on` a string key instead of the boolean True.
    return yaml.load((WORKFLOWS / name).read_text(), Loader=yaml.BaseLoader)


def evaluate(template: str, context: dict) -> str:
    """Evaluate a `${{ }}` template over the `||`, `&&`, `==`, `!=` and property subset.

    Missing properties are empty strings; `a || b` yields the first truthy operand.
    """
    assert "${{" in template, template
    return re.sub(
        r"\$\{\{(.*?)\}\}",
        lambda m: _expression(m.group(1), context),
        template,
        flags=re.DOTALL,
    )


def _expression(text: str, context: dict) -> str:
    expr = " ".join(text.split())
    while inner := re.search(r"\(([^()]*)\)", expr):
        value = _or(inner.group(1), context)
        expr = expr[: inner.start()] + f"'{value}'" + expr[inner.end() :]
    return _or(expr, context)


def _or(expr: str, context: dict) -> str:
    result = ""
    for part in expr.split(" || "):
        result = _and(part, context)
        if result:
            return result
    return result


def _and(expr: str, context: dict) -> str:
    result = ""
    for part in expr.split(" && "):
        result = _compare(part.strip(), context)
        if not result:
            return result
    return result


def _compare(expr: str, context: dict) -> str:
    for op in ("==", "!="):
        if f" {op} " in expr:
            left, right = (_operand(x.strip(), context) for x in expr.split(f" {op} "))
            return "true" if (left == right) == (op == "==") else ""
    return _operand(expr, context)


def _operand(token: str, context: dict) -> str:
    if token.startswith("'"):
        return token.strip("'")
    value = context
    for key in token.split("."):
        value = value.get(key, "") if isinstance(value, dict) else ""
    return str(value)


PULL_REQUEST = {
    "event_name": "pull_request",
    "ref": "refs/pull/7/merge",
    "event": {"pull_request": {"number": 7}},
}
MERGE_GROUP = {
    "event_name": "merge_group",
    "ref": "refs/heads/gh-readonly-queue/main/pr-7-aaaa",
    "event": {"merge_group": {"base_sha": "b" * 40, "head_sha": "c" * 40}},
}
OTHER_ENTRY = {
    "event_name": "merge_group",
    "ref": "refs/heads/gh-readonly-queue/main/pr-8-dddd",
    "event": {"merge_group": {"base_sha": "c" * 40, "head_sha": "e" * 40}},
}


def ctx(context: dict) -> dict:
    return {"github": context}


def test_ci_runs_on_merge_group_checks_requested():
    assert load("test.yml")["on"]["merge_group"] == {"types": ["checks_requested"]}


PUSH_MAIN = {"event_name": "push", "ref": "refs/heads/main", "sha": "1" * 40}
NEXT_PUSH = {"event_name": "push", "ref": "refs/heads/main", "sha": "2" * 40}


def test_concurrency_group_is_unique_per_run():
    template = load("test.yml")["concurrency"]["group"]
    contexts = (PULL_REQUEST, MERGE_GROUP, OTHER_ENTRY, PUSH_MAIN, NEXT_PUSH)
    groups = [evaluate(template, ctx(c)) for c in contexts]
    assert groups == [
        "ci-7",
        "ci-refs/heads/gh-readonly-queue/main/pr-7-aaaa",
        "ci-refs/heads/gh-readonly-queue/main/pr-8-dddd",
        "ci-" + "1" * 40,
        "ci-" + "2" * 40,
    ]


@pytest.mark.parametrize(
    ("context", "cancels"),
    [(PULL_REQUEST, "true"), (MERGE_GROUP, ""), (PUSH_MAIN, "")],
)
def test_only_pull_request_runs_cancel_a_superseded_run(context, cancels):
    template = load("test.yml")["concurrency"]["cancel-in-progress"]
    assert evaluate(template, ctx(context)) == cancels


def rust_cache_steps() -> dict[tuple[str, str], dict]:
    found = {}
    for workflow in ("test.yml", "docs-site.yml"):
        for name, job in load(workflow)["jobs"].items():
            for step in job.get("steps", []):
                if step.get("uses", "").startswith("Swatinem/rust-cache@"):
                    found[(workflow, name)] = step
    return found


def test_every_rust_cache_step_is_checked():
    assert set(rust_cache_steps()) == {
        ("test.yml", "rust-static"),
        ("test.yml", "rust"),
        ("test.yml", "rust-msrv"),
        ("docs-site.yml", "build"),
    }


@pytest.mark.parametrize(
    ("context", "saves"),
    [(PUSH_MAIN, "true"), (PULL_REQUEST, ""), (MERGE_GROUP, "")],
)
@pytest.mark.parametrize("where", sorted(rust_cache_steps()))
def test_rust_cache_is_saved_only_from_main(where, context, saves):
    step = rust_cache_steps()[where]
    assert evaluate(step["with"]["save-if"], ctx(context)) == saves


@pytest.mark.parametrize(
    "workflow", ["test.yml", "lint.yml", "security.yml", "cli-e2e.yml"]
)
def test_no_job_condition_excludes_merge_group_by_ref_or_pull_request_only(workflow):
    for name, job in load(workflow)["jobs"].items():
        condition = str(job.get("if", ""))
        assert "refs/heads/main" not in condition, name
        if "'pull_request'" in condition:
            assert "'merge_group'" in condition, name


def test_agnix_diff_base_uses_the_merge_group_base():
    lint = load("lint.yml")["jobs"]["workflow-lint"]
    env = next(s for s in lint["steps"] if s.get("name") == "Check PR agnix inputs")[
        "env"
    ]
    base = env["BASE_SHA"]
    assert evaluate(base, ctx(MERGE_GROUP)) == "b" * 40
    pr = {**PULL_REQUEST, "event": {"pull_request": {"base": {"sha": "a" * 40}}}}
    assert evaluate(base, ctx(pr)) == "a" * 40


def test_secret_scan_of_a_queue_entry_is_bounded_by_the_group_range():
    steps = load("security.yml")["jobs"]["trufflehog"]["steps"]
    queue = next(s for s in steps if s["name"] == "Scan merge queue entry")
    assert evaluate("${{ " + queue["if"] + " }}", ctx(MERGE_GROUP)) == "true"
    assert evaluate("${{ " + queue["if"] + " }}", ctx(PULL_REQUEST)) == ""
    assert evaluate(queue["with"]["base"], ctx(MERGE_GROUP)) == "b" * 40
    assert evaluate(queue["with"]["head"], ctx(MERGE_GROUP)) == "c" * 40
    pushed = next(s for s in steps if s["name"] == "Scan pushed commits")
    assert (
        evaluate("${{ " + " ".join(pushed["if"].split()) + " }}", ctx(MERGE_GROUP))
        == ""
    )


# --- dependency review -------------------------------------------------------

REVIEW_JOB = load("security.yml")["jobs"]["dependency-review"]
REVIEW_STEP = next(
    s for s in REVIEW_JOB["steps"] if "dependency-review-action" in s.get("uses", "")
)


def test_dependency_review_receives_the_group_range_on_merge_group():
    inputs = REVIEW_STEP["with"]
    assert evaluate(inputs["base-ref"], ctx(MERGE_GROUP)) == "b" * 40
    assert evaluate(inputs["head-ref"], ctx(MERGE_GROUP)) == "c" * 40


def test_dependency_review_refs_stay_unset_on_pull_request():
    inputs = REVIEW_STEP["with"]
    assert evaluate(inputs["base-ref"], ctx(PULL_REQUEST)) == ""
    assert evaluate(inputs["head-ref"], ctx(PULL_REQUEST)) == ""


@pytest.mark.parametrize(
    ("context", "dependencies", "runs"),
    [
        (PULL_REQUEST, True, True),
        (MERGE_GROUP, True, True),
        (MERGE_GROUP, False, False),
        ({"event_name": "push"}, True, False),
    ],
)
def test_dependency_review_job_condition(context, dependencies, runs):
    full = {**ctx(context), "inputs": {"dependencies": "true" if dependencies else ""}}
    assert bool(evaluate(REVIEW_JOB["if"], full)) is runs


# --- security-gate script ----------------------------------------------------

_GATE = load("security.yml")["jobs"]["security-gate"]["steps"][0]["run"]
GATE_SCRIPT = re.search(r"python3 - <<'PY'\n(.*?)\n\s*PY\n", _GATE, re.DOTALL).group(1)
GATE_SCRIPT = "\n".join(
    line.removeprefix(" " * 10) for line in GATE_SCRIPT.splitlines()
)


def run_security_gate(event: str, dependencies: bool, review: str) -> int:
    expected_osv = "success" if dependencies else "skipped"
    needs = {
        "dependency-review": {"result": review},
        "trufflehog": {"result": "skipped"},
        "codeql": {"result": "skipped"},
        "osv": {"result": expected_osv},
        "trivy": {"result": "skipped"},
    }
    env = {
        "RESULTS": json.dumps(needs),
        "EVENT_NAME": event,
        "CODE": "false",
        "DEPENDENCIES": "true" if dependencies else "false",
        "FILESYSTEM": "false",
        "SECRETS": "false",
    }
    return subprocess.run(
        [sys.executable, "-c", GATE_SCRIPT],
        env={**os.environ, **env},
        capture_output=True,
        text=True,
        check=False,
    ).returncode


@pytest.mark.parametrize("event", ["pull_request", "merge_group"])
@pytest.mark.parametrize("dependencies", [True, False])
@pytest.mark.parametrize("review", ["success", "skipped", "failure"])
def test_security_gate_requires_dependency_review_exactly_when_classified(
    event, dependencies, review
):
    accepted = review == ("success" if dependencies else "skipped")
    assert (run_security_gate(event, dependencies, review) == 0) is accepted


def test_security_gate_expects_dependency_review_skipped_on_push():
    assert run_security_gate("push", True, "skipped") == 0
    assert run_security_gate("push", True, "success") == 1
