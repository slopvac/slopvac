"""Model-eval tests: fact alignment, task building, scoring, planning and summaries.

No model is called. Fake models answer from the prompt.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys

import pytest

from evals import __main__ as cli
from evals import build_derived, facts, models
from evals import corpuslib as lib

# --- facts --------------------------------------------------------------------------


def test_extract_finds_each_kind():
    got = facts.extract(
        "The `parse_config` call must not retry more than 3 times at 10% load; pass --offline."
    )
    assert got["code"] == {"parse_config": 1}
    assert got["number"] == {"3": 1, "10%": 1}
    assert got["polarity"] == {"must": 1, "not": 1}
    assert got["identifier"] == {"--offline": 1}


def test_contractions_count_as_negation():
    assert facts.extract("It doesn't retry.")["polarity"] == {"not": 1}
    assert facts.extract("It does not retry.")["polarity"] == {"not": 1}


def test_preservation_lists_exactly_the_lost_facts():
    source = "Retry 3 times, never more. Use `max_retries` and `--offline`."
    output = "Retry up to 4 times. Use `max_retries`."
    result = facts.preservation(source, output)
    assert result["lost"] == [
        {"kind": "code", "fact": "--offline", "missing": 1},
        {"kind": "number", "fact": "3", "missing": 1},
        {"kind": "polarity", "fact": "never", "missing": 1},
    ]
    assert (result["kept"], result["facts"]) == (1, 4)


def test_an_intended_change_is_not_a_loss():
    expected = [{"kind": "number", "old": "1", "new": "2"}]
    result = facts.preservation("Option 1 is faster.", "Option 2 is faster.", expected)
    assert result["lost"] == [] and result["changes_applied"] == 1


def test_an_intended_change_that_is_not_made_is_reported():
    expected = [{"kind": "number", "old": "1", "new": "2"}]
    result = facts.preservation("Option 1 is faster.", "Option 1 is faster.", expected)
    assert result["changes_applied"] == 0 and result["lost"]


# --- tasks over the real corpus -------------------------------------------------------


@pytest.fixture(scope="module")
def corpus(tmp_path_factory):
    cache = tmp_path_factory.mktemp("cache")
    manifest = lib.load_manifest()
    code = build_derived.main(["--materialise", "--cache", str(cache)])
    assert code == 0
    return manifest, cache


def test_task_building_is_deterministic_and_covers_every_type(corpus):
    manifest, cache = corpus
    a = models.build_tasks(manifest, cache, per_type=4)
    assert a == models.build_tasks(manifest, cache, per_type=4)
    kinds = {(t["type"], t["role"]) for t in a}
    assert kinds == {
        ("mechanical", "author"),
        ("update", "author"),
        ("rewrite", "author"),
        ("rewrite", "reviewer"),
    }
    assert len({t["id"] for t in a}) == len(a)


def test_update_tasks_carry_the_stated_change_and_it_is_in_the_passage(corpus):
    manifest, cache = corpus
    for task in (
        t
        for t in models.build_tasks(manifest, cache, per_type=6)
        if t["type"] == "update"
    ):
        change = task["expected"][0]
        assert re.search(rf"(?<!\d){re.escape(change['old'])}(?!\d)", task["source"])


# --- scoring ---------------------------------------------------------------------------

NO_FINDINGS = lambda text: []


def test_update_succeeds_only_when_the_change_is_applied_and_nothing_else_moves():
    task = {
        "type": "update",
        "source": "Use `max_retries` 3 times.",
        "expected": [{"kind": "number", "old": "3", "new": "5"}],
    }
    good = models.score_author(task, "Use `max_retries` 5 times.", NO_FINDINGS)
    unchanged = models.score_author(task, "Use `max_retries` 3 times.", NO_FINDINGS)
    dropped = models.score_author(task, "Use it 5 times.", NO_FINDINGS)
    assert (good["success"], unchanged["success"], dropped["success"]) == (
        True,
        False,
        False,
    )


def test_mechanical_needs_the_rule_cleared_and_every_fact_kept():
    task = {
        "type": "mechanical",
        "source": "It serves as a cache of 40 items.",
        "rule": "r.x",
    }
    fixed = models.score_author(task, "It is a cache of 40 items.", NO_FINDINGS)
    still = models.score_author(
        task, "It is a cache of 40 items.", lambda t: [{"rule": "r.x"}]
    )
    lost = models.score_author(task, "It is a cache.", NO_FINDINGS)
    assert (fixed["success"], still["success"], lost["success"]) == (True, False, False)


def test_rewrite_fails_when_slop_rises_even_if_facts_survive():
    task = {"type": "rewrite", "source": "Cache 40 items today."}

    def lint(text):
        return [{"rule": "r"}] * (3 if "Delve" in text else 0)

    ok = models.score_author(task, "Cache 40 items now.", lint)
    worse = models.score_author(task, "Delve into it: cache 40 items now.", lint)
    assert (ok["success"], worse["success"]) == (True, False)


def test_a_code_fence_around_the_answer_is_ignored():
    task = {"type": "rewrite", "source": "Cache 40 items."}
    scored = models.score_author(task, "```markdown\nCache 40 items.\n```", NO_FINDINGS)
    assert scored["fact_rate"] == 1.0


def planted_task():
    return {
        "variant": "planted",
        "edits": [
            {
                "kind": "number",
                "sentence": "The queue holds 63 jobs before it blocks writers.",
            },
            {
                "kind": "drop",
                "sentence": "Run the migration with the `--offline` flag first.",
            },
        ],
    }


def test_review_finds_planted_edits_by_the_words_it_quotes():
    answer = json.dumps(
        [
            {
                "quote": "The queue holds 68 jobs before it blocks writers",
                "problem": "number",
            },
            {
                "quote": "Run the migration with the --offline flag first",
                "problem": "dropped",
            },
        ]
    )
    scored = models.score_review(planted_task(), answer)
    assert (
        scored["found"],
        scored["planted"],
        scored["false_alarms"],
        scored["success"],
    ) == (2, 2, 0, True)


def test_review_counts_false_alarms_and_misses():
    answer = json.dumps(
        [
            {
                "quote": "The queue holds 63 jobs before it blocks writers",
                "problem": "x",
            },
            {"quote": "Totally unrelated heading text here", "problem": "y"},
            {"quote": "Another unrelated sentence about colours", "problem": "z"},
        ]
    )
    scored = models.score_review(planted_task(), answer)
    assert (scored["found"], scored["false_alarms"], scored["success"]) == (1, 2, False)


@pytest.mark.parametrize("answer", ["", "no problems", "[not json", "{}"])
def test_unparseable_review_answers_find_nothing(answer):
    assert models.score_review(planted_task(), answer)["found"] == 0


def test_authored_review_matches_lost_facts_by_name():
    task = {"variant": "authored", "lost": ["--offline", "3"]}
    answer = json.dumps([{"quote": "x", "problem": "the --offline flag vanished"}])
    scored = models.score_review(task, answer)
    assert (scored["found"], scored["planted"]) == (1, 2)


# --- planning, running and summarising ------------------------------------------------------


def roster(*names):
    return [
        {"name": n, "family": f"fam-{n}", "tier": "frontier", "command": f"echo {n}"}
        for n in names
    ]


PROMPTS = {"update": "Update the passage: ", "rewrite": "Rewrite the passage: "}


def tiny_tasks():
    author = lambda i, kind: {
        "id": f"a{i}",
        "type": kind,
        "role": "author",
        "doc": "d",
        "genre": "g",
        "prompt": PROMPTS[kind] + "x" * 400,
        "source": "Cache 40 items.",
        "rule": "r",
    }
    reviewer = {
        "id": "p1",
        "type": "rewrite",
        "role": "reviewer",
        "variant": "planted",
        "doc": "d",
        "genre": "g",
        "prompt": "y" * 800,
        "edits": [{"kind": "n", "sentence": "alpha beta gamma delta"}],
    }
    return [author(1, "update"), author(2, "rewrite"), reviewer]


def test_plan_counts_calls_by_hand():
    # 2 models x 3 repeats: authors 2x3=6 each, planted reviews 1x3=3 each -> 18.
    # Authored reviews: 1 rewrite task x 3 repeats x 2 authors x 1 other reviewer = 6.
    counted = models.plan(tiny_tasks(), roster("a", "b"), repeats=3)
    assert counted["total_calls"] == 24
    assert counted["authored_review_calls"] == 6
    assert (
        counted["per_model"]["a"]["input_tokens_estimate"] == (420 + 421 + 800) * 3 // 4
    )


def test_plan_refuses_fewer_than_three_repeats():
    with pytest.raises(models.ModelEvalError, match="at least 3"):
        models.plan(tiny_tasks(), roster("a"), repeats=2)


def fake_ask(model):
    def ask(prompt):
        if "Update the passage" in prompt:
            return "Cache 40 items."
        if "Rewrite the passage" in prompt:
            return "Cache 40 items." if model["name"] == "good" else "Cache items."
        return json.dumps([{"quote": "alpha beta gamma delta", "problem": "changed"}])

    return ask


def test_execute_runs_every_task_per_model_and_reviews_other_models_rewrites():
    tasks = [t for t in tiny_tasks() if t["id"] != "a1"]  # drop the update task
    out = models.execute(
        tasks,
        roster("good", "poor"),
        3,
        NO_FINDINGS,
        ask_for=fake_ask,
        date="2026-11-14",
    )
    assert out["date"] == "2026-11-14"
    authored = [r for r in out["runs"] if r.get("variant") == "authored"]
    assert {(r["model"], r["author_model"]) for r in authored} == {
        ("good", "poor"),
        ("poor", "good"),
    }
    assert len(out["runs"]) == out["planned_calls"]


def test_summary_ranks_models_by_success_per_task():
    tasks = [t for t in tiny_tasks() if t["id"] != "a1"]
    out = models.execute(
        tasks,
        roster("good", "poor"),
        3,
        NO_FINDINGS,
        ask_for=fake_ask,
        date="2026-11-14",
    )
    summary = models.summarize(out, resamples=50)
    rows = {(r["type"], r["role"], r["model"]): r for r in summary["rows"]}
    assert rows[("rewrite", "author", "good")]["success_rate"] == 1.0
    assert rows[("rewrite", "author", "poor")]["success_rate"] == 0.0
    assert summary["ranking"]["rewrite/author"] == ["good", "poor"]
    assert summary["date"] == "2026-11-14"


def test_authored_reviews_with_nothing_lost_are_left_out_of_the_summary():
    tasks = [t for t in tiny_tasks() if t["id"] == "a2"]
    out = models.execute(
        tasks, roster("good", "poor"), 3, NO_FINDINGS, ask_for=fake_ask, date="d"
    )
    summary = models.summarize(out, resamples=10)
    assert all(
        r["role"] == "author" or r["type"] != "rewrite" or r["tasks"] > 0
        for r in summary["rows"]
    )
    reviewers = [r for r in summary["rows"] if r["role"] == "reviewer"]
    assert [r["model"] for r in reviewers] == [
        "good"
    ]  # only the poor rewrite lost facts


def test_run_command_needs_the_exact_call_count(tmp_path, monkeypatch):
    path = tmp_path / "models.json"
    path.write_text(json.dumps({"models": roster("a")}))
    monkeypatch.setattr(cli.lib, "load_manifest", lambda: {"documents": []})
    base = [
        "models",
        "run",
        "--models",
        str(path),
        "--cache",
        str(tmp_path),
        "--per-type",
        "1",
    ]
    assert cli.main([*base, "--confirm-calls", "999", "--max-invocations", "5"]) == 2


def test_models_file_needs_every_field(tmp_path):
    path = tmp_path / "m.json"
    path.write_text(json.dumps({"models": [{"name": "a", "family": "f"}]}))
    with pytest.raises(models.ModelEvalError, match="tier"):
        models.load_models(path)


def test_the_cli_never_starts_a_model_for_plan(tmp_path, monkeypatch, capsys):
    path = tmp_path / "models.json"
    path.write_text(
        json.dumps({"models": [{**m, "command": "false"} for m in roster("a")]})
    )
    monkeypatch.setattr(cli.lib, "load_manifest", lambda: {"documents": []})
    calls = []
    monkeypatch.setattr(subprocess, "run", lambda *a, **k: calls.append(a))
    assert (
        cli.main(["models", "plan", "--models", str(path), "--cache", str(tmp_path)])
        == 0
    )
    assert calls == [] and json.loads(capsys.readouterr().out)["total_calls"] == 0


@pytest.mark.parametrize(
    ("reply", "found"),
    [
        ("the 13 limit changed", 0),
        ("the limit 3.5 changed", 0),
        ("retries 3 is gone", 1),
        ("lost: 3.", 1),
    ],
)
def test_authored_review_matches_whole_fact_tokens(reply, found):
    task = {"variant": "authored", "lost": ["3"]}
    answer = json.dumps([{"quote": "x", "problem": reply}])
    assert models.score_review(task, answer)["found"] == found


def test_authored_review_rows_cluster_by_source_task_and_author_not_by_output():
    tasks = [t for t in tiny_tasks() if t["id"] == "a2"]
    answers = iter(["Cache items.", "Cache 40 items.", "Cache one item."] * 4)

    def ask_for(model):
        def ask(prompt):
            if prompt.startswith("Rewrite the passage"):
                return next(answers)
            return json.dumps([{"quote": "z", "problem": "40 lost"}])

        return ask

    out = models.execute(
        tasks, roster("a", "b"), 3, NO_FINDINGS, ask_for=ask_for, date="d"
    )
    keys = {r["task"] for r in out["runs"] if r.get("variant") == "authored"}
    stable = models.task_id("authored", "a2")
    assert keys == {f"{stable}:a", f"{stable}:b"}


def test_opencode_events_join_text_parts_and_report_errors():
    from evals import opencode_model

    stream = "\n".join(
        [
            json.dumps({"type": "step_start"}),
            json.dumps({"type": "text", "part": {"text": "Hello "}}),
            json.dumps({"type": "text", "part": {"text": "world"}}),
            "not json",
        ]
    )
    assert opencode_model.parse_events(stream) == "Hello world"
    failure = json.dumps({"type": "error", "error": {"message": "throttled"}})
    with pytest.raises(opencode_model.ModelCallError, match="throttled"):
        opencode_model.parse_events(failure)


def test_a_flaky_command_is_retried_and_a_dead_one_fails_after_three_attempts(tmp_path):
    counter = tmp_path / "n"
    counter.write_text("0")
    script = tmp_path / "flaky.py"
    script.write_text(
        "import sys, pathlib\n"
        f"p = pathlib.Path({str(counter)!r})\n"
        "n = int(p.read_text()) + 1\n"
        "p.write_text(str(n))\n"
        "if n < 3:\n    sys.exit(1)\n"
        "print('ok')\n"
    )
    ask = models.command_ask(f"{sys.executable} {script}")
    assert ask("x").text.strip() == "ok" and counter.read_text() == "3"
    dead = models.command_ask(f"{sys.executable} -c 'import sys; sys.exit(1)'")
    with pytest.raises(models.ModelEvalError, match="exited 1"):
        dead("x")
