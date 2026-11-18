"""cla.yml: which events start the CLA job.

The job's `if:` expression is evaluated against sample event payloads, so the
test checks behaviour, not text. The action accepts a signature only when the
whole trimmed comment body equals the sign phrase. The job filter must never be
stricter than that, or a valid signature would be dropped before the action
sees it.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
# BaseLoader keeps every scalar a string, so `on` stays a key and not a boolean.
CLA = yaml.load(
    (ROOT / ".github" / "workflows" / "cla.yml").read_text(), Loader=yaml.BaseLoader
)
PHRASE = "I have read the CLA Document and I hereby sign the CLA"

TOKEN = re.compile(r"\s*(\|\||&&|==|!=|!|\(|\)|,|'(?:[^']|'')*'|[A-Za-z_][\w.\-]*)")


def tokens(expr: str) -> list[str]:
    out, pos = [], 0
    while pos < len(expr.rstrip()):
        m = TOKEN.match(expr, pos)
        assert m, f"cannot tokenize at {expr[pos:]!r}"
        out.append(m.group(1))
        pos = m.end()
    return out


def evaluate(expr: str, ctx: dict) -> bool:
    """A small evaluator for the GitHub expression subset the job filter uses."""
    toks = tokens(expr)
    pos = 0

    def peek():
        return toks[pos] if pos < len(toks) else None

    def take(expected=None):
        nonlocal pos
        tok = toks[pos]
        assert expected is None or tok == expected, (tok, expected)
        pos += 1
        return tok

    def lookup(path: str):
        value = ctx
        for part in path.split("."):
            if not isinstance(value, dict) or part not in value:
                return None
            value = value[part]
        return value

    def primary():
        tok = take()
        if tok == "!":
            return not primary()
        if tok == "cancelled":
            take("(")
            take(")")
            return ctx.get("cancelled", False)
        if tok == "(":
            value = disjunction()
            take(")")
            return value
        if tok.startswith("'"):
            return tok[1:-1].replace("''", "'")
        if tok == "contains":
            take("(")
            haystack = disjunction()
            take(",")
            needle = disjunction()
            take(")")
            return isinstance(haystack, str) and needle.lower() in haystack.lower()
        return lookup(tok)

    def comparison():
        left = primary()
        if peek() in ("==", "!="):
            op = take()
            right = primary()
            return (left == right) if op == "==" else (left != right)
        return left

    def conjunction():
        value = comparison()
        while peek() == "&&":
            take()
            right = comparison()
            value = value and right
        return value

    def disjunction():
        value = conjunction()
        while peek() == "||":
            take()
            right = conjunction()
            value = value or right
        return value

    result = disjunction()
    assert pos == len(toks), f"trailing tokens {toks[pos:]}"
    return bool(result)


JOB_IF = CLA["jobs"]["CLA"]["if"]


def comment(body: str, on_pr: bool = True) -> dict:
    issue = {"number": 7, "pull_request": {"url": "x"}} if on_pr else {"number": 7}
    return {
        "github": {
            "event_name": "issue_comment",
            "event": {"issue": issue, "comment": {"body": body}},
        }
    }


@pytest.mark.parametrize(
    "body",
    [
        pytest.param(PHRASE, id="exact"),
        pytest.param(f"{PHRASE}\n", id="trailing-newline"),
        pytest.param(f"  {PHRASE}  ", id="padded"),
        pytest.param(f"\r\n{PHRASE}\r\n", id="crlf"),
        pytest.param("recheck", id="recheck"),
        pytest.param("recheck\n", id="recheck-newline"),
    ],
)
def test_every_comment_the_action_accepts_starts_the_job(body):
    assert evaluate(JOB_IF, comment(body))


@pytest.mark.parametrize(
    "body",
    [
        pytest.param("LGTM", id="unrelated"),
        pytest.param("Thanks for the review!", id="thanks"),
        pytest.param("", id="empty"),
    ],
)
def test_unrelated_comments_do_not_start_the_job(body):
    assert not evaluate(JOB_IF, comment(body))


def test_comments_on_plain_issues_do_not_start_the_job():
    assert not evaluate(JOB_IF, comment(PHRASE, on_pr=False))


@pytest.mark.parametrize("event", ["pull_request_target"])
def test_pull_request_events_start_the_job(event):
    assert evaluate(JOB_IF, {"github": {"event_name": event, "event": {}}})


def test_merge_group_runs_only_the_pass_through_job():
    ctx = {"github": {"event_name": "merge_group", "event": {}}}
    assert not evaluate(JOB_IF, ctx)
    assert evaluate(CLA["jobs"]["CLA-merge-queue"]["if"], ctx)


def test_both_jobs_report_the_required_check_name():
    assert {job["name"] for job in CLA["jobs"].values()} == {"CLA"}


def test_only_the_filtered_job_joins_the_concurrency_queue():
    assert "concurrency" not in CLA
    assert "concurrency" in CLA["jobs"]["CLA"]
    assert CLA["jobs"]["CLA"]["concurrency"]["cancel-in-progress"] == "false"


def test_the_filter_uses_the_phrase_the_contributing_guide_documents():
    guide = (ROOT / "CONTRIBUTING.md").read_text()
    assert PHRASE in guide
    assert f"'{PHRASE}'" in JOB_IF


def edit(new_body: str, old_body: str) -> dict:
    ctx = comment(new_body)
    ctx["github"]["event"]["action"] = "edited"
    ctx["github"]["event"]["changes"] = {"body": {"from": old_body}}
    return ctx


def test_editing_a_sign_comment_away_starts_the_job():
    assert evaluate(JOB_IF, edit("never mind", PHRASE))


def test_deleting_a_sign_comment_starts_the_job():
    ctx = comment(PHRASE)
    ctx["github"]["event"]["action"] = "deleted"
    assert evaluate(JOB_IF, ctx)


def test_editing_an_unrelated_comment_does_not_start_the_job():
    assert not evaluate(JOB_IF, edit("LGTM, thanks", "LGTM"))


RECHECK = next(
    s
    for s in CLA["jobs"]["CLA"]["steps"]
    if s.get("name") == "Re-check the pull request after a sign comment changed"
)


def test_edits_and_deletions_re_run_the_pull_request_check():
    steps = {s.get("name"): s for s in CLA["jobs"]["CLA"]["steps"]}
    recheck = steps["Re-check the pull request after a sign comment changed"]
    for action, expected in [("edited", True), ("deleted", True), ("created", False)]:
        ctx = comment(PHRASE)
        ctx["github"]["event"]["action"] = action
        assert evaluate(recheck["if"], ctx) is expected, action
    assert set(CLA["on"]["issue_comment"]["types"]) == {"created", "edited", "deleted"}


def test_the_re_check_runs_after_the_action_fails_but_not_after_a_cancel():
    ctx = comment(PHRASE)
    ctx["github"]["event"]["action"] = "deleted"
    # A withdrawn signature fails the action; the re-check must still run.
    assert evaluate(RECHECK["if"], ctx)
    ctx["cancelled"] = True
    assert not evaluate(RECHECK["if"], ctx)


def test_comment_runs_queue_apart_from_pull_request_runs():
    group = CLA["jobs"]["CLA"]["concurrency"]["group"]
    assert "github.event_name" in group


FAKE_GH = """#!/bin/sh
# A stand-in for gh: answers the three calls the re-check step makes.
echo "$*" >> "$GH_LOG"
case "$*" in
  "pr view 7 --repo slopvac/slopvac --json headRefOid --jq .headRefOid") echo abc123 ;;
  "pr view 7 --repo slopvac/slopvac --json headRefName --jq .headRefName") echo feature/x ;;
  api*) printf '%s' "$RUNS" | jq -r "$(echo "$*" | sed -n "s/.*--jq //p")" ;;
  "run rerun "*) ;;
  *) echo "unexpected gh call: $*" >&2; exit 1 ;;
esac
"""


def run_recheck(tmp_path, runs: list[dict]) -> list[str]:
    import json
    import os
    import subprocess

    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    gh = bin_dir / "gh"
    gh.write_text(FAKE_GH)
    gh.chmod(0o755)
    log = tmp_path / "gh.log"
    env = {
        **os.environ,
        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
        "GH_LOG": str(log),
        "RUNS": json.dumps({"workflow_runs": runs}),
        "REPO": "slopvac/slopvac",
        "PR": "7",
    }
    subprocess.run(["bash", "-e", "-c", RECHECK["run"]], env=env, check=True)
    return log.read_text().splitlines()


def test_the_re_check_re_runs_the_run_for_the_pull_request_branch(tmp_path):
    calls = run_recheck(
        tmp_path,
        [
            {"id": 11, "head_branch": "other"},
            {"id": 22, "head_branch": "feature/x"},
        ],
    )
    assert calls[-1] == "run rerun 22 --repo slopvac/slopvac"


def test_the_re_check_re_runs_nothing_without_a_matching_run(tmp_path):
    calls = run_recheck(tmp_path, [{"id": 11, "head_branch": "other"}])
    assert not any(c.startswith("run rerun") for c in calls)
