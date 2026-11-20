"""A model answer is scored as the model wrote it; storage keeps a redacted copy.

The journal holds the redacted copy and the score computed from the original, so a
replay reproduces the fresh score. A rescore from the stored copy is also checked.
"""

from __future__ import annotations

import json
import shlex
import sys

import pytest

from evals import models

FAKE = "FAKEsess1234"
SOURCE = "Cache 40 items. Use `max_retries` 3 times."
TASK = {
    "id": "a1",
    "type": "rewrite",
    "role": "author",
    "doc": "d",
    "genre": "g",
    "prompt": "Rewrite the passage: " + SOURCE,
    "source": SOURCE,
    "rule": "r",
}
ROSTER = [
    {"name": n, "family": f"fam-{n}", "tier": "t", "command": "unused"}
    for n in ("a", "b")
]


def no_findings(text):
    return []


def command(text: str) -> str:
    script = f"import sys; sys.stdout.write({text!r})"
    return shlex.join([sys.executable, "-c", script])


def asks(text: str):
    return lambda model: models.command_ask(command(text), attempts=1)


def must_not_ask(model):
    def ask(prompt):
        raise AssertionError("replay asked a model")

    return ask


def run(path, ask_for, *, tasks=(TASK,)):
    return models.execute(
        list(tasks),
        ROSTER,
        3,
        no_findings,
        ask_for=ask_for,
        journal=models.Journal(path),
    )


def scores(results):
    return [(r["task"], r["model"], r["repeat"], r["score"]) for r in results["runs"]]


def journal_entries(path):
    return [json.loads(line) for line in path.read_text("utf-8").splitlines()]


ANSWER_WITH_CREDENTIAL = (
    f'Keep 40 items cached. Use `max_retries` 3 times. {{"sessionToken": "{FAKE}"}}'
)


def test_replay_reproduces_the_fresh_score_and_never_asks_again(tmp_path):
    path = tmp_path / "run.journal.jsonl"
    fresh = run(path, asks(ANSWER_WITH_CREDENTIAL))
    replay = run(path, must_not_ask)
    assert scores(replay) == scores(fresh)
    assert all(r["score"]["success"] for r in fresh["runs"] if r["role"] == "author")


def test_the_journal_keeps_only_the_redacted_copy_and_flags_it(tmp_path):
    path = tmp_path / "run.journal.jsonl"
    run(path, asks(ANSWER_WITH_CREDENTIAL))
    assert FAKE not in path.read_text("utf-8")
    flagged = [e for e in journal_entries(path) if e.get("credential_suspected")]
    assert flagged
    assert all("Keep 40 items cached." in e["answer"] for e in flagged)


def test_a_rescore_from_the_stored_copy_equals_the_fresh_score(tmp_path):
    path = tmp_path / "run.journal.jsonl"
    fresh = run(path, asks(ANSWER_WITH_CREDENTIAL))
    stored = journal_entries(path)[0]["answer"]
    rescored = models.score_author(TASK, stored, no_findings)
    first = next(r for r in fresh["runs"] if r["role"] == "author")
    assert rescored == first["score"]


def test_replay_keeps_a_score_that_depends_on_text_the_copy_no_longer_has(tmp_path):
    source = "Set `token: 12345` before the first call."
    task = TASK | {"source": source, "prompt": "Rewrite the passage: " + source}
    answer = "Before the first call, set `token: 12345`."
    path = tmp_path / "run.journal.jsonl"
    fresh = run(path, asks(answer), tasks=[task])
    stored = journal_entries(path)[0]["answer"]
    assert "12345" not in stored
    fresh_row = next(r for r in fresh["runs"] if r["role"] == "author")
    assert fresh_row["score"]["success"] is True
    # A rescore of the redacted copy loses the fact; replay does not.
    assert models.score_author(task, stored, no_findings)["success"] is False
    assert scores(run(path, must_not_ask, tasks=[task])) == scores(fresh)


def test_ordinary_answers_are_journaled_unchanged_and_not_flagged(tmp_path):
    path = tmp_path / "run.journal.jsonl"
    run(path, asks("Keep 40 items cached. Use `max_retries` 3 times."))
    for entry in journal_entries(path):
        if entry.get("kind") == "score":
            continue
        assert entry["answer"] == "Keep 40 items cached. Use `max_retries` 3 times."
        assert not entry.get("credential_suspected")


def test_reviews_of_a_credentialed_rewrite_replay_without_asking(tmp_path):
    path = tmp_path / "run.journal.jsonl"
    fresh = run(path, asks(ANSWER_WITH_CREDENTIAL))
    assert any(r.get("variant") == "authored" for r in fresh["runs"])
    replay = run(path, must_not_ask)
    assert scores(replay) == scores(fresh)


def test_the_cost_executions_ignore_score_lines(tmp_path):
    path = tmp_path / "run.journal.jsonl"
    fresh = run(path, asks(ANSWER_WITH_CREDENTIAL))
    assert len(fresh["executions"]) == fresh["planned_calls"]
    assert all(e["job"] for e in models.Journal(path).executions())


@pytest.mark.parametrize(
    ("version", "kept"),
    [
        (models.SCORER_VERSION, True),
        ("15.17", False),
        ("14.0", False),
        (None, False),
    ],
    ids=[
        "same-version-is-reused",
        "containment-scorer-predecessor-is-recomputed",
        "other-version-is-recomputed",
        "no-version-is-recomputed",
    ],
)
def test_a_stored_score_is_reused_only_for_the_same_scorer_version(
    tmp_path, version, kept
):
    path = tmp_path / "run.journal.jsonl"
    run(path, asks("Keep 40 items cached. Use `max_retries` 3 times."))
    lines = journal_entries(path)
    for line in lines:
        if line.get("kind") == "score":
            line["score"] = {"success": "MARKER"}
            if version:
                line["scorer_version"] = version
            else:
                line.pop("scorer_version", None)
    path.write_text("".join(json.dumps(x, sort_keys=True) + "\n" for x in lines))
    replay = run(path, must_not_ask)
    authors = [r["score"]["success"] for r in replay["runs"] if r["role"] == "author"]
    assert (authors == ["MARKER"] * len(authors)) is kept
    assert (True in authors) is (not kept)
