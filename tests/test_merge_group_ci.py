"""Required checks must mean the same thing in a merge queue run as in a PR run.

A queue entry runs on ``refs/heads/gh-readonly-queue/main/pr-N-<sha>`` with a
``merge_group`` payload: no ``pull_request`` object, and the diff base is
``merge_group.base_sha``.
"""

from __future__ import annotations

import json
import math
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


# --- a typed subset of the Actions expression language -------------------------
#
# Semantics follow docs.github.com/en/actions/reference/workflows-and-actions/
# expressions: values keep their types (null, boolean, number, string, object).
# `==` and `!=` compare strings case-insensitively; operands of different types are
# both coerced to numbers (null 0, false 0, true 1, a string is parsed as a number,
# '' is 0, anything else is NaN, which equals nothing). `a || b` and `a && b`
# return an operand, not a boolean. A missing property is null. Interpolation
# writes null as '', booleans as 'true'/'false'.

_TOKEN = re.compile(
    r"\s*(?:(?P<str>'(?:[^']|'')*')|(?P<num>-?\d+(?:\.\d+)?)"
    r"|(?P<op>\|\||&&|==|!=|!|\(|\))|(?P<name>[A-Za-z_][\w.-]*))"
)


def _tokens(text: str) -> list[tuple[str, str]]:
    out, pos, text = [], 0, text.strip()
    while pos < len(text):
        m = _TOKEN.match(text, pos)
        assert m and m.end() > pos, f"unsupported expression at {text[pos:]!r}"
        out.append((m.lastgroup, m.group(m.lastgroup)))
        pos = m.end()
    return out


class _Parser:
    def __init__(self, text: str, context: dict):
        self.tokens, self.i, self.context = _tokens(text), 0, context

    def peek(self) -> str | None:
        return self.tokens[self.i][1] if self.i < len(self.tokens) else None

    def take(self) -> tuple[str, str]:
        self.i += 1
        return self.tokens[self.i - 1]

    def parse(self):
        value = self.or_()
        assert self.i == len(self.tokens), f"trailing tokens {self.tokens[self.i :]}"
        return value

    def or_(self):
        value = self.and_()
        while self.peek() == "||":
            self.take()
            right = self.and_()
            value = value if truthy(value) else right
        return value

    def and_(self):
        value = self.compare()
        while self.peek() == "&&":
            self.take()
            right = self.compare()
            value = right if truthy(value) else value
        return value

    def compare(self):
        value = self.unary()
        if self.peek() in ("==", "!="):
            op = self.take()[1]
            equal = loose_equal(value, self.unary())
            value = equal if op == "==" else not equal
        return value

    def unary(self):
        if self.peek() == "!":
            self.take()
            return not truthy(self.unary())
        return self.primary()

    def primary(self):
        kind, text = self.take()
        if text == "(":
            value = self.or_()
            assert self.take()[1] == ")"
            return value
        if kind == "str":
            return text[1:-1].replace("''", "'")
        if kind == "num":
            return float(text) if "." in text else int(text)
        assert kind == "name", text
        if text in ("true", "false", "null"):
            return {"true": True, "false": False, "null": None}[text]
        value = self.context
        for key in text.split("."):
            value = value.get(key) if isinstance(value, dict) else None
        return value


def _number(value) -> float:
    if value is None:
        return 0.0
    if isinstance(value, bool):
        return float(value)
    if isinstance(value, (int, float)):
        return float(value)
    if isinstance(value, str):
        if value.strip() == "":
            return 0.0
        if re.fullmatch(r"\s*-?\d+(\.\d+)?([eE][-+]?\d+)?\s*", value):
            return float(value)
    return math.nan


def loose_equal(left, right) -> bool:
    if type(left) is type(right) or (
        isinstance(left, (int, float))
        and isinstance(right, (int, float))
        and not isinstance(left, bool)
        and not isinstance(right, bool)
    ):
        if isinstance(left, str):
            return left.casefold() == right.casefold()
        if isinstance(left, (dict, list)):
            return left is right
        return left == right
    return _number(left) == _number(right)


def truthy(value) -> bool:
    if isinstance(value, float) and math.isnan(value):
        return False
    return value not in (None, False, 0, "")


def value_of(expression: str, context: dict):
    """The typed value of one expression, with or without its `${{ }}` wrapper."""
    text = " ".join(expression.split())
    if m := re.fullmatch(r"\$\{\{(.*)\}\}", text):
        text = m.group(1)
    return _Parser(text, context).parse()


def condition(expression: str, context: dict) -> bool:
    """Whether an `if:` with this expression runs."""
    return truthy(value_of(expression, context))


def _interpolate(value) -> str:
    if value is None:
        return ""
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, float) and value.is_integer():
        return str(int(value))
    return str(value)


def evaluate(template: str, context: dict) -> str:
    """Interpolate every `${{ }}` in a template, as Actions writes it to a string."""
    assert "${{" in template, template
    return re.sub(
        r"\$\{\{(.*?)\}\}",
        lambda m: _interpolate(_Parser(m.group(1), context).parse()),
        template,
        flags=re.DOTALL,
    )


@pytest.mark.parametrize(
    ("expression", "context", "expected"),
    [
        ("x == 'true'", {"x": True}, False),
        ("x != 'true'", {"x": True}, True),
        ("x == true", {"x": True}, True),
        ("x != true", {}, True),
        ("x == false", {}, True),
        ("x == 'TRUE'", {"x": "true"}, True),
        ("x == 0", {"x": ""}, True),
        ("x == 'abc'", {"x": 0}, False),
        ("x || 'b'", {"x": ""}, "b"),
        ("x && 'b'", {"x": 0}, 0),
        ("!(x == 'a') && y", {"x": "b", "y": "c"}, "c"),
    ],
)
def test_evaluator_models_actions_types_and_coercion(expression, context, expected):
    assert value_of(expression, context) == expected


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


# --- per-event matrices -------------------------------------------------------

TEST_JOBS = load("test.yml")["jobs"]


def matrix_axis(job: str, axis: str, context: dict) -> list:
    template = " ".join(TEST_JOBS[job]["strategy"]["matrix"][axis].split())
    inner = re.fullmatch(r"\$\{\{ fromJSON\((.*)\) \}\}", template).group(1)
    return json.loads(evaluate("${{ " + inner + " }}", ctx(context)))


# Leg name -> (runner, cross target). Copied from the static matrix this replaced.
EVERY_RUST_LEG = {
    "linux-x64": ("ubuntu-latest", None),
    "linux-arm64": ("ubuntu-24.04-arm", None),
    "linux-x64-musl": ("ubuntu-latest", "x86_64-unknown-linux-musl"),
    "linux-arm64-musl": ("ubuntu-24.04-arm", "aarch64-unknown-linux-musl"),
    "macos-arm64": ("macos-latest", None),
    "macos-x64": ("macos-15-intel", None),
}
EVERY_PYTHON = ["3.11", "3.12", "3.13", "3.14"]


def test_pull_requests_test_rust_on_linux_x64_only():
    assert matrix_axis("rust", "include", PULL_REQUEST) == [
        {"name": "linux-x64", "os": "ubuntu-latest"}
    ]


@pytest.mark.parametrize("context", [MERGE_GROUP, PUSH_MAIN], ids=["queue", "main"])
def test_queue_and_main_test_every_rust_leg(context):
    legs = matrix_axis("rust", "include", context)
    by_name = {leg["name"]: (leg["os"], leg.get("target")) for leg in legs}
    assert by_name == EVERY_RUST_LEG
    assert len(legs) == len(EVERY_RUST_LEG)
    assert all(leg.get("musl", False) == ("target" in leg) for leg in legs)


@pytest.mark.parametrize(
    ("context", "versions"),
    [
        (PULL_REQUEST, ["3.11", "3.14"]),
        (MERGE_GROUP, EVERY_PYTHON),
        (PUSH_MAIN, EVERY_PYTHON),
    ],
    ids=["pull_request", "queue", "main"],
)
def test_python_versions_per_event(context, versions):
    assert matrix_axis("lint-package", "python", context) == versions


@pytest.mark.parametrize(
    "context",
    [PULL_REQUEST, MERGE_GROUP, PUSH_MAIN],
    ids=["pull_request", "queue", "main"],
)
def test_rules_reference_check_runs_on_exactly_one_python_leg(context):
    step = next(
        s
        for s in TEST_JOBS["lint-package"]["steps"]
        if s["name"] == "The rules reference is current"
    )
    runs = [
        v
        for v in matrix_axis("lint-package", "python", context)
        if evaluate("${{ " + step["if"] + " }}", {"matrix": {"python": v}})
    ]
    assert len(runs) == 1


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
    assert condition(queue["if"], ctx(MERGE_GROUP))
    assert not condition(queue["if"], ctx(PULL_REQUEST))
    assert evaluate(queue["with"]["base"], ctx(MERGE_GROUP)) == "b" * 40
    assert evaluate(queue["with"]["head"], ctx(MERGE_GROUP)) == "c" * 40
    pushed = next(s for s in steps if s["name"] == "Scan pushed commits")
    assert not condition(pushed["if"], ctx(MERGE_GROUP))


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
    # A boolean workflow_call input arrives as a boolean, not a string.
    full = {**ctx(context), "inputs": {"dependencies": dependencies}}
    assert condition(REVIEW_JOB["if"], full) is runs


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


# --- draft pull requests -----------------------------------------------------

DRAFT_PR = {**PULL_REQUEST, "event": {"pull_request": {"number": 7, "draft": True}}}
READY_PR = {**PULL_REQUEST, "event": {"pull_request": {"number": 7, "draft": False}}}
# A ready PR converted back to a draft.
CONVERTED = {
    **PULL_REQUEST,
    "event": {
        "action": "converted_to_draft",
        "pull_request": {"number": 7, "draft": True},
    },
}
MAIN_PUSH = {"event_name": "push", "ref": "refs/heads/main"}
TAG_PUSH = {"event_name": "push", "ref": "refs/tags/v3.0.0"}
NOT_DRAFT = [READY_PR, MERGE_GROUP, MAIN_PUSH, TAG_PUSH]
NOT_DRAFT_IDS = ["ready", "queue", "main", "tag"]


def pull_request_workflows() -> dict[str, dict]:
    found = {}
    for path in sorted(WORKFLOWS.glob("*.yml")):
        workflow = load(path.name)
        triggers = workflow["on"]
        if "pull_request" in (
            triggers if isinstance(triggers, (dict, list)) else [triggers]
        ):
            found[path.name] = workflow
    return found


def runs(expression: str, context: dict) -> bool:
    return condition(expression, ctx(context))


def test_every_pull_request_workflow_runs_when_a_draft_is_marked_ready():
    workflows = pull_request_workflows()
    assert set(workflows) == {
        "action-v3-test.yml",
        "release.yml",
        "test.yml",
        "wheels.yml",
    }
    for name, workflow in workflows.items():
        assert workflow["on"]["pull_request"]["types"] == [
            "opened",
            "synchronize",
            "reopened",
            "ready_for_review",
            "converted_to_draft",
        ], name


ROOT_JOBS = [
    ("action-v3-test.yml", "action"),
    ("release.yml", "route"),
    ("test.yml", "changes"),
    ("test.yml", "quick-lint"),
    ("wheels.yml", "build"),
]


def test_every_job_that_needs_nothing_is_listed_as_a_root():
    roots = [
        (name, job)
        for name, workflow in pull_request_workflows().items()
        for job, spec in workflow["jobs"].items()
        if "needs" not in spec
    ]
    assert roots == ROOT_JOBS


@pytest.mark.parametrize("context", [DRAFT_PR, CONVERTED], ids=["draft", "converted"])
@pytest.mark.parametrize(("workflow", "job"), ROOT_JOBS)
def test_root_jobs_skip_a_draft(workflow, job, context):
    assert not runs(load(workflow)["jobs"][job]["if"], context)


@pytest.mark.parametrize("context", NOT_DRAFT, ids=NOT_DRAFT_IDS)
@pytest.mark.parametrize(("workflow", "job"), ROOT_JOBS)
def test_root_jobs_run_for_everything_but_a_draft(workflow, job, context):
    assert runs(load(workflow)["jobs"][job]["if"], context)


GATE_DRAFT = load("test.yml")["jobs"]["gate"]["steps"][0]["env"]["DRAFT"]


def test_the_gate_sees_a_draft():
    assert evaluate(GATE_DRAFT, ctx(DRAFT_PR)) == "true"


@pytest.mark.parametrize("context", NOT_DRAFT, ids=NOT_DRAFT_IDS)
def test_the_gate_sees_no_draft_otherwise(context):
    assert evaluate(GATE_DRAFT, ctx(context)) == "false"


def literal_or_evaluated(value: str, context: dict) -> str:
    return evaluate(value, context) if "${{" in value else value


_CI_GATE_RUN = load("test.yml")["jobs"]["gate"]["steps"][0]["run"]
CI_GATE_SCRIPT = re.search(r"python3 - <<'PY'\n(.*?)\nPY\n", _CI_GATE_RUN, re.DOTALL)[1]


def test_converting_to_draft_replaces_the_ready_run_with_a_failing_one():
    concurrency = load("test.yml")["concurrency"]
    assert evaluate(concurrency["group"], ctx(CONVERTED)) == evaluate(
        concurrency["group"], ctx(READY_PR)
    )
    assert literal_or_evaluated(concurrency["cancel-in-progress"], ctx(CONVERTED)) == (
        "true"
    )
    draft = evaluate(GATE_DRAFT, ctx(CONVERTED))
    gate = subprocess.run(
        [sys.executable, "-c", CI_GATE_SCRIPT],
        env={**os.environ, "DRAFT": draft, "RESULTS": "{}"},
        capture_output=True,
        text=True,
        check=False,
    )
    assert gate.returncode == 1
    assert "draft pull request" in gate.stdout


# --- reviewdog posts only for open, ready pull requests -----------------------

REVIEWDOG = load("reviewdog.yml")["jobs"]
ELIGIBLE_STEP = REVIEWDOG["eligible"]["steps"][0]
SHA = "a" * 40
FAKE_GH = """#!/usr/bin/env bash
# Stands in for `gh api`: records the call and applies its --jq filter to $FAKE_PULLS.
printf '%s\\n' "$@" > "$FAKE_ARGS"
while (( $# )); do
  if [[ "$1" == --jq ]]; then jq -r "$2" <<< "$FAKE_PULLS"; exit; fi
  shift
done
exit 1
"""


def pull(draft: bool, sha: str = SHA, repo: str = "srobroek/slopvac") -> dict:
    return {
        "number": 7,
        "draft": draft,
        "head": {"sha": sha, "repo": {"full_name": repo}},
    }


def run_eligibility(tmp_path: Path, pulls: list[dict], head_sha: str = SHA):
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    (bin_dir / "gh").write_text(FAKE_GH)
    (bin_dir / "gh").chmod(0o755)
    output = tmp_path / "output"
    output.touch()
    env = {
        **os.environ,
        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
        "FAKE_PULLS": json.dumps(pulls),
        "FAKE_ARGS": str(tmp_path / "args"),
        "GITHUB_OUTPUT": str(output),
        "REPO": "srobroek/slopvac",
        "HEAD_SHA": head_sha,
        "HEAD_OWNER": "srobroek",
        "HEAD_REPO": "srobroek/slopvac",
        "HEAD_BRANCH": "feature",
    }
    proc = subprocess.run(
        ["bash", "-c", ELIGIBLE_STEP["run"]],
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    return proc, output.read_text()


@pytest.mark.parametrize(
    ("pulls", "eligible"),
    [
        ([pull(draft=False)], "true"),
        ([pull(draft=True)], "false"),
        ([], "false"),
        ([pull(draft=False, sha="b" * 40)], "false"),
        ([pull(draft=False, repo="fork/slopvac")], "false"),
        ([pull(draft=False), pull(draft=True)], "false"),
    ],
    ids=["ready", "draft", "none", "other-head", "other-repo", "one-draft"],
)
def test_reviewdog_eligibility_requires_an_open_ready_pull_request(
    tmp_path, pulls, eligible
):
    proc, output = run_eligibility(tmp_path, pulls)
    assert proc.returncode == 0, proc.stderr
    assert output == f"eligible={eligible}\n"


def test_reviewdog_eligibility_asks_for_open_pull_requests_from_the_head_branch(
    tmp_path,
):
    run_eligibility(tmp_path, [pull(draft=False)])
    args = (tmp_path / "args").read_text().splitlines()
    assert args[:3] == ["api", "-X", "GET"]
    assert "state=open" in args
    assert "head=srobroek:feature" in args


def test_reviewdog_eligibility_rejects_a_malformed_head_sha(tmp_path):
    proc, output = run_eligibility(tmp_path, [pull(draft=False)], head_sha="main")
    assert proc.returncode == 1
    assert output == ""


@pytest.mark.parametrize("job", ["comment", "comment-v3"])
def test_reviewdog_comment_jobs_wait_for_eligibility(job):
    assert REVIEWDOG[job]["needs"] == "eligible"


@pytest.mark.parametrize("conclusion", ["success", "failure"])
@pytest.mark.parametrize(
    ("eligible", "runs"), [("true", True), ("false", False), ("", False)]
)
@pytest.mark.parametrize("job", ["comment", "comment-v3"])
def test_reviewdog_comment_jobs_run_only_when_eligible(job, eligible, runs, conclusion):
    context = {
        "github": {
            "event": {
                "workflow_run": {"event": "pull_request", "conclusion": conclusion}
            }
        },
        "needs": {"eligible": {"outputs": {"eligible": eligible}}},
    }
    assert condition(REVIEWDOG[job]["if"], context) is runs


@pytest.mark.parametrize(
    ("event", "runs"), [("pull_request", True), ("push", False), ("merge_group", False)]
)
def test_reviewdog_eligibility_runs_for_pull_request_ci_only(event, runs):
    context = {"github": {"event": {"workflow_run": {"event": event}}}}
    assert condition(REVIEWDOG["eligible"]["if"], context) is runs


@pytest.mark.parametrize("operator", ["!=", "=="])
def test_a_guard_comparing_draft_to_a_string_is_caught(operator):
    # draft is a boolean: `draft != 'true'` is true for a draft (true is coerced to 1,
    # 'true' to NaN), so such a guard would run the job. The root-job tests catch it.
    guard = f"github.event.pull_request.draft {operator} 'true'"
    assert condition(guard, ctx(DRAFT_PR)) is (operator == "!=")
