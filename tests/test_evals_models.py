"""Model-eval tests: fact alignment, task building, scoring, planning and summaries.

No model is called. Fake models answer from the prompt.
"""

from __future__ import annotations

import json
import os
import re
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path

import pytest

from evals import __main__ as cli
from evals import build_derived, facts, models, runner
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


# --- an incomplete lint is not a pass ---------------------------------------------------


def fake_engine(tmp_path, *, unchecked):
    script = tmp_path / "engine.py"
    report = {
        "documents": [{"path": "candidate.md", "findings": [], "unchecked": unchecked}]
    }
    script.write_text(
        f"import sys\nprint({json.dumps(report)!r})\nsys.exit({2 if unchecked else 0})\n"
    )
    return runner.Engine("fake", (sys.executable, str(script)))


def test_a_lint_that_left_rules_unchecked_raises_instead_of_reporting_no_findings(
    tmp_path,
):
    lint = models.make_linter(fake_engine(tmp_path, unchecked=["vale: not installed"]))
    with pytest.raises(models.LintIncomplete, match="vale: not installed"):
        lint("It is a cache of 40 items.")


def test_a_complete_lint_still_returns_its_findings(tmp_path):
    assert models.make_linter(fake_engine(tmp_path, unchecked=[]))("text") == []


def test_a_run_whose_lint_was_incomplete_is_unscored_and_left_out_of_the_summary():
    def lint(text):
        raise models.LintIncomplete("vale: not installed")

    tasks = [t for t in tiny_tasks() if t["id"] == "a2"]
    out = models.execute(
        tasks, roster("good", "poor"), 3, lint, ask_for=fake_ask, date="d"
    )
    authors = [r for r in out["runs"] if r["role"] == "author"]
    assert len(authors) == 6
    assert all(r["score"] is None and "vale" in r["unscored"] for r in authors)
    assert out["unscored_runs"] == 6
    # The rewrite author rows are unscored; only the authored reviews remain scored.
    summary = models.summarize(out, resamples=10)
    assert [r for r in summary["rows"] if r["role"] == "author"] == []


# --- Ctrl-C stops the run -----------------------------------------------------------------


def test_ctrl_c_kills_running_model_commands_and_starts_no_more(tmp_path):
    script = tmp_path / "slow.py"
    script.write_text(
        "import os, sys, time, pathlib\n"
        f"pathlib.Path({str(tmp_path)!r}, f'pid-{{os.getpid()}}').write_text('x')\n"
        "sys.stdin.read()\n"
        "time.sleep(60)\n"
    )
    roster_ = [
        {
            "name": "slow",
            "family": "f",
            "tier": "t",
            "command": f"{sys.executable} {script}",
        }
    ]

    signalled: list[float] = []

    def interrupt_once_two_commands_run():
        deadline = time.monotonic() + 20
        while len(list(tmp_path.glob("pid-*"))) < 2 and time.monotonic() < deadline:
            time.sleep(0.02)
        signalled.append(time.monotonic())
        os.kill(os.getpid(), signal.SIGINT)

    threading.Thread(target=interrupt_once_two_commands_run, daemon=True).start()
    budget = models.Budget(100)
    with pytest.raises(KeyboardInterrupt):
        models.execute(
            [t for t in tiny_tasks() if t["id"] == "a1"],
            roster_,
            3,
            NO_FINDINGS,
            workers=2,
            budget=budget,
        )
    assert time.monotonic() - signalled[0] < 15
    pids = [int(p.name.removeprefix("pid-")) for p in tmp_path.glob("pid-*")]
    assert len(pids) == 2 and budget.spent == 2
    for pid in pids:
        with pytest.raises(ProcessLookupError):
            os.kill(pid, 0)


# --- summary intervals --------------------------------------------------------------------


def results_with(successes_by_task):
    runs = [
        {
            "type": "update",
            "role": "author",
            "model": "m",
            "task": task,
            "score": {"success": bool(ok), "planted": 0},
        }
        for task, outcomes in successes_by_task.items()
        for ok in outcomes
    ]
    return {
        "date": "d",
        "prompt_version": "1",
        "repeats": 3,
        "models": [{"name": "m", "family": "f", "tier": "t"}],
        "runs": runs,
    }


# Four tasks with three repeats each. Task means: 2/3, 0, 1, 1/3, so the point
# estimate is 6/12 = 0.5. The interval comes from 200 draws of 4 task means with
# random.Random("slopvac-evals-bootstrap-1"), worked out in a separate script, and
# the 2.5th and 97.5th percentiles are 1/6 and 5/6.
TASK_OUTCOMES = {
    "t1": [1, 1, 0],
    "t2": [0, 0, 0],
    "t3": [1, 1, 1],
    "t4": [1, 0, 0],
}


def test_summary_interval_resamples_tasks_with_repeats_averaged_inside_each():
    row = models.summarize(results_with(TASK_OUTCOMES), resamples=200)["rows"][0]
    assert (row["tasks"], row["runs"]) == (4, 12)
    assert row["success_rate"] == pytest.approx(0.5)
    # Resampling the 12 rows instead would give [0.25, 0.75] on the same draws.
    assert row["interval"] == pytest.approx([1 / 6, 5 / 6])


# --- task-set hash ------------------------------------------------------------------------


@pytest.mark.parametrize(
    ("field", "value"),
    [
        ("source", "Cache 41 items."),
        ("prompt", "Rewrite the passage: changed"),
        ("expected", [{"kind": "number", "old": "1", "new": "9"}]),
        ("rule", "other-rule"),
        ("edits", [{"kind": "drop", "sentence": "different sentence"}]),
    ],
)
def test_task_digest_changes_when_task_content_changes(field, value):
    tasks = tiny_tasks()
    before = models.tasks_digest(tasks)
    tasks[1][field] = value
    assert models.tasks_digest(tasks) != before


def test_task_digest_ignores_key_order():
    tasks = tiny_tasks()
    reordered = [dict(reversed(list(t.items()))) for t in tasks]
    assert models.tasks_digest(reordered) == models.tasks_digest(tasks)


def test_the_committed_results_file_still_loads_and_summarises():
    path = Path(models.RESULTS_DIR) / "2026-11-14.json"
    summary = models.summarize(models.load(path), resamples=20)
    assert summary["rows"]


# --- round 2: submission, repeated numbers, number facts, authored-review ids -----------


def slow_command(tmp_path):
    script = tmp_path / "slow.py"
    script.write_text(
        "import os, sys, time, pathlib\n"
        f"pathlib.Path({str(tmp_path)!r}, f'pid-{{os.getpid()}}').write_text('x')\n"
        "sys.stdin.read()\n"
        "time.sleep(60)\n"
    )
    return f"{sys.executable} {script}"


def test_ctrl_c_during_job_submission_still_kills_started_commands_and_keeps_the_ledger(
    tmp_path, monkeypatch
):
    submitted = []
    signalled: list[float] = []

    class InterruptingPool(models.ThreadPoolExecutor):
        def submit(self, fn, *args, **kwargs):
            if len(submitted) == 2:
                # The first two jobs have started their commands by now.
                deadline = time.monotonic() + 20
                while (
                    len(list(tmp_path.glob("pid-*"))) < 2
                    and time.monotonic() < deadline
                ):
                    time.sleep(0.02)
                signalled.append(time.monotonic())
                raise KeyboardInterrupt
            future = super().submit(fn, *args, **kwargs)
            submitted.append(future)
            return future

    monkeypatch.setattr(models, "ThreadPoolExecutor", InterruptingPool)
    ledger = tmp_path / "run.ledger.jsonl"
    roster_ = [
        {"name": "slow", "family": "f", "tier": "t", "command": slow_command(tmp_path)}
    ]
    with models.Budget(100, ledger) as budget, pytest.raises(KeyboardInterrupt):
        models.execute(
            [t for t in tiny_tasks() if t["id"] == "a1"],
            roster_,
            3,
            NO_FINDINGS,
            workers=2,
            budget=budget,
        )
    assert time.monotonic() - signalled[0] < 15
    pids = [int(p.name.removeprefix("pid-")) for p in tmp_path.glob("pid-*")]
    assert len(pids) == 2
    for pid in pids:
        with pytest.raises(ProcessLookupError):
            os.kill(pid, 0)
    events = [json.loads(line) for line in ledger.read_text().splitlines()]
    assert [e["n"] for e in events if e["event"] == "reserve"] == [1, 2]
    assert [e["status"] for e in events if e["event"] == "finish"] == [
        "interrupted",
        "interrupted",
    ]


def test_a_number_change_applies_to_every_occurrence_of_the_old_value():
    source = "See [#1098](https://example.org/issues/1098) for details."
    expected = [{"kind": "number", "old": "1098", "new": "1103"}]
    both = "See [#1103](https://example.org/issues/1103) for details."
    one = "See [#1103](https://example.org/issues/1098) for details."
    assert facts.preservation(source, both, expected)["changes_applied"] == 1
    assert facts.preservation(source, both, expected)["lost"] == []
    assert facts.preservation(source, one, expected)["changes_applied"] == 0


@pytest.fixture(scope="module")
def update_tasks(corpus):
    manifest, cache = corpus
    built = models.build_tasks(manifest, cache, per_type=12)
    return [t for t in built if t["type"] == "update"]


def apply_change(task):
    change = task["expected"][0]
    return re.sub(
        rf"(?<![\w.]){re.escape(change['old'])}(?![\w])",
        change["new"],
        task["source"],
    )


def test_every_update_task_passes_when_the_change_is_made_everywhere(update_tasks):
    assert update_tasks
    for task in update_tasks:
        scored = models.score_author(task, apply_change(task), NO_FINDINGS)
        assert scored["success"], task["source"]
        assert not models.score_author(task, task["source"], NO_FINDINGS)["success"]


def update_tasks_for(tmp_path, passage, old):
    """The update tasks `build_tasks` makes from one rewrite pair editing `old` to 9999."""
    docs = tmp_path / "docs"
    docs.mkdir()
    (docs / "parent.txt").write_text(passage + "\n")
    (docs / "pair.txt").write_text(passage.replace(old, "9999") + "\n")
    manifest = {
        "documents": [
            {
                "id": "parent",
                "origin": "human",
                "format": "text",
                "genre": "g",
                "source": {},
            },
            {
                "id": "pair",
                "origin": "rewrite-pair",
                "format": "text",
                "genre": "g",
                "source": {
                    "parents": ["parent"],
                    "edits": [
                        {
                            "kind": "number",
                            "detail": f"{old} -> 9999",
                            "original_sentence": passage,
                            "line": 1,
                        }
                    ],
                },
            },
        ]
    }
    built = models.build_tasks(manifest, tmp_path, per_type=12)
    return [t for t in built if t["type"] == "update"]


@pytest.mark.parametrize(
    ("passage", "old", "eligible"),
    [
        # The 40 is part of a code span: it cannot be changed as a bare number.
        ("Set `retries=40` before you start.", "40", False),
        ("Run `limit 40` to cap the list.", "40", False),
        # Even a lone bare 40 elsewhere does not rescue a span that also holds one.
        ("Set `retries=40` to 40 here.", "40", False),
        # The number is glued into an identifier token.
        ("See issue_1234 for the background.", "1234", False),
        ("Call Foo.bar1234 to reset it.", "1234", False),
        # A bare number in running prose can be changed on its own.
        ("The cache holds 40 items.", "40", True),
        # Code elsewhere in the passage does not matter.
        ("Set `retries` to 40 seconds.", "40", True),
        # An issue reference in running prose is a bare number.
        ("Fixed in #1234 last week.", "1234", True),
        (
            "See [Requests #1234](https://github.com/psf/requests/issues/1234).",
            "1234",
            True,
        ),
        # An issue reference inside a code span is code, alone or inside a link.
        ("Fixed in `#1234` last week.", "1234", False),
        ("See [`#1234`](https://github.com/psf/requests/issues/1234).", "1234", False),
        # The old value must appear at all.
        ("The cache holds 41 items.", "40", False),
    ],
)
def test_update_tasks_only_change_numbers_that_stand_alone_in_the_passage(
    tmp_path, passage, old, eligible
):
    tasks = update_tasks_for(tmp_path, passage, old)
    expected = [[{"kind": "number", "old": old, "new": "9999"}]] if eligible else []
    assert [t["expected"] for t in tasks] == expected


def test_a_bare_issue_reference_in_prose_is_scored_as_a_number_update():
    task = {
        "type": "update",
        "source": "Fixed in #1234 last week.",
        "expected": [{"kind": "number", "old": "1234", "new": "1301"}],
    }
    done = models.score_author(task, "Fixed in #1301 last week.", NO_FINDINGS)
    undone = models.score_author(task, "Fixed in #1234 last week.", NO_FINDINGS)
    assert (done["success"], undone["success"]) == (True, False)


def test_the_pydantic_update_with_a_repeated_number_is_well_posed(update_tasks):
    task = next(t for t in update_tasks if "pydantic/issues/1098" in t["source"])
    assert facts.extract(task["source"])["number"]["1098"] == 2
    assert models.score_author(task, apply_change(task), NO_FINDINGS)["success"]


def test_authored_reviews_of_identical_candidates_get_distinct_journal_jobs(tmp_path):
    tasks = [t for t in tiny_tasks() if t["id"] == "a2"]
    asked = []

    def ask_for(model):
        def ask(prompt):
            if prompt.startswith("Rewrite the passage"):
                return "Cache items."  # every author writes the same candidate
            asked.append((model["name"], prompt))
            return json.dumps([{"quote": "z", "problem": "40 lost"}])

        return ask

    journal = models.Journal(tmp_path / "j.jsonl")
    out = models.execute(
        tasks,
        roster("a", "b", "c"),
        3,
        NO_FINDINGS,
        ask_for=ask_for,
        journal=journal,
    )
    # 3 reviewers x 2 other authors x 3 repeats = 18 reviews, none served from
    # another author's journal entry.
    assert len(asked) == 18
    assert len([r for r in out["runs"] if r.get("variant") == "authored"]) == 18


def test_identical_review_prompts_from_two_source_tasks_stay_independent(tmp_path):
    first = next(t for t in tiny_tasks() if t["id"] == "a2")
    second = {**first, "id": "a3"}  # same passage and prompt, another source task
    tasks = [first, second]
    asked = []
    lock = threading.Lock()

    def ask_for(model):
        def ask(prompt):
            if prompt.startswith("Rewrite the passage"):
                return "Cache items."
            with lock:
                asked.append(prompt)
                # A different answer on every call, so a reused entry shows.
                problem = f"40 lost, call {len(asked)}"
            return json.dumps([{"quote": "z", "problem": problem}])

        return ask

    def run(journal):
        return models.execute(
            tasks,
            roster("a", "b", "c"),
            3,
            NO_FINDINGS,
            ask_for=ask_for,
            journal=journal,
        )

    def reviews(out):
        return {
            (r["task"], r["model"], r["repeat"]): r["output_sha256"]
            for r in out["runs"]
            if r.get("variant") == "authored"
        }

    path = tmp_path / "j.jsonl"
    before = run(models.Journal(path))
    # 2 source tasks x 3 authors x 2 other reviewers x 3 repeats.
    assert len(asked) == 36
    assert len(set(reviews(before).values())) == 36

    asked.clear()
    after = run(models.Journal(path))
    assert asked == []  # the resume asked nothing
    assert reviews(after) == reviews(before)  # each entry went back to its own job


# --- token usage and cost ------------------------------------------------------------


PRICES = {
    "source": "https://example.org/prices",
    "models": {
        "big": {"input": 2, "output": 10},
        "small": {"input": 0.1, "output": 0.5},
    },
}


def cost_cli(tmp_path, capsys, records_path, prices=PRICES):
    prices_path = tmp_path / "prices.json"
    prices_path.write_text(json.dumps(prices))
    code = cli.main(["models", "cost", str(records_path), "--prices", str(prices_path)])
    return code, json.loads(capsys.readouterr().out)


def write_results(tmp_path, runs):
    path = tmp_path / "r.json"
    path.write_text(json.dumps({"schema": models.RESULTS_SCHEMA, "runs": runs}))
    return path


def test_cost_prices_input_and_output_per_million_tokens_by_model(tmp_path, capsys):
    path = write_results(
        tmp_path,
        [
            {"model": "big", "usage": {"input": 1_000_000, "output": 500_000}},
            {"model": "big", "usage": {"input": 500_000, "output": 0}},
            {"model": "small", "usage": {"input": 2_000_000, "output": 1_000_000}},
        ],
    )
    code, report = cost_cli(tmp_path, capsys, path)
    assert code == 0
    # big: 1.5M in x $2 + 0.5M out x $10 = 3 + 5 = 8. small: 2M x 0.1 + 1M x 0.5 = 0.7.
    assert report["models"]["big"]["cost_usd"] == 8
    assert report["models"]["big"]["tokens"] == {"input": 1_500_000, "output": 500_000}
    assert report["models"]["small"]["cost_usd"] == pytest.approx(0.7)
    assert report["total_cost_usd"] == pytest.approx(8.7)
    assert report["complete"] is True


def test_reasoning_tokens_are_billed_at_the_output_price(tmp_path, capsys):
    path = write_results(
        tmp_path, [{"model": "big", "usage": {"output": 100_000, "reasoning": 300_000}}]
    )
    _, report = cost_cli(tmp_path, capsys, path)
    assert report["models"]["big"]["cost_usd"] == 4  # 0.4M x $10


def test_an_invocation_without_usage_marks_the_cost_incomplete(tmp_path, capsys):
    path = write_results(
        tmp_path,
        [
            {"model": "big", "usage": {"input": 1_000_000}},
            {"model": "big", "usage": None},
            {"model": "small"},
        ],
    )
    _, report = cost_cli(tmp_path, capsys, path)
    assert report["complete"] is False
    assert report["models"]["big"]["missing_usage"] == 1
    assert report["models"]["big"]["complete"] is False
    assert report["models"]["small"]["missing_usage"] == 1
    assert report["models"]["big"]["cost_usd"] == 2


def test_a_model_missing_from_the_price_list_has_no_cost_and_is_incomplete(
    tmp_path, capsys
):
    path = write_results(tmp_path, [{"model": "other", "usage": {"input": 5}}])
    _, report = cost_cli(tmp_path, capsys, path)
    assert report["models"]["other"]["cost_usd"] is None
    assert report["complete"] is False


def journal_lines(tmp_path, lines):
    path = tmp_path / "r.journal.jsonl"
    path.write_text("".join(json.dumps(line) + "\n" for line in lines))
    return path


def test_cost_counts_every_execution_of_a_resumed_job(tmp_path, capsys):
    path = journal_lines(
        tmp_path,
        [
            {
                "job": "a",
                "model": "big",
                "error": "boom",
                "attempts": 3,
                "usage": {"input": 500_000},
            },
            {
                "job": "a",
                "model": "big",
                "error": None,
                "attempts": 1,
                "usage": {"input": 1_000_000},
            },
            {
                "job": "b",
                "model": "small",
                "error": None,
                "attempts": 1,
                "usage": {"output": 2_000_000},
            },
        ],
    )
    _, report = cost_cli(tmp_path, capsys, path)
    assert report["models"]["big"]["invocations"] == 2
    assert report["models"]["big"]["tokens"] == {"input": 1_500_000}
    # big: 1.5M x $2 = 3. small: 2M x $0.5 = 1.
    assert report["total_cost_usd"] == pytest.approx(4)
    assert report["complete"] is True


def test_an_earlier_attempt_without_usage_makes_a_resumed_job_incomplete(
    tmp_path, capsys
):
    path = journal_lines(
        tmp_path,
        [
            {"job": "a", "model": "big", "error": "boom", "attempts": 3, "usage": None},
            {
                "job": "a",
                "model": "big",
                "error": None,
                "attempts": 1,
                "usage": {"input": 1_000_000},
            },
        ],
    )
    _, report = cost_cli(tmp_path, capsys, path)
    assert report["models"]["big"]["missing_usage"] == 1
    assert report["complete"] is False


def test_results_cost_comes_from_the_executions_section_not_the_rows(tmp_path, capsys):
    path = tmp_path / "r.json"
    path.write_text(
        json.dumps(
            {
                "schema": models.RESULTS_SCHEMA,
                "runs": [{"model": "big", "usage": {"input": 1_000_000}}],
                "executions": [
                    {"job": "a", "model": "big", "usage": {"input": 500_000}},
                    {"job": "a", "model": "big", "usage": {"input": 1_000_000}},
                    {"job": "gone", "model": "big", "usage": {"input": 500_000}},
                ],
            }
        )
    )
    _, report = cost_cli(tmp_path, capsys, path)
    assert report["models"]["big"]["invocations"] == 3
    assert report["models"]["big"]["tokens"] == {"input": 2_000_000}
    assert report["total_cost_usd"] == pytest.approx(4)


def test_cache_tokens_are_reported_but_do_not_change_the_cost(tmp_path, capsys):
    path = write_results(
        tmp_path,
        [
            {
                "model": "big",
                "usage": {
                    "input": 200_000,
                    "output": 100_000,
                    "cache_read": 4_000_000,
                    "cache_write": 3_000_000,
                },
            }
        ],
    )
    _, report = cost_cli(tmp_path, capsys, path)
    # 0.2M x $2 + 0.1M x $10 = 0.4 + 1.0 = 1.4, whatever the cache holds.
    assert report["models"]["big"]["cost_usd"] == pytest.approx(1.4)
    assert report["models"]["big"]["tokens"] == {
        "input": 200_000,
        "output": 100_000,
        "cache_read": 4_000_000,
        "cache_write": 3_000_000,
    }


def test_cost_on_a_journal_from_before_usage_was_recorded_is_incomplete(
    tmp_path, capsys
):
    path = tmp_path / "old.journal.jsonl"
    path.write_text('{"job": "a", "answer": "x", "error": null, "attempts": 1}\n')
    code, report = cost_cli(tmp_path, capsys, path)
    assert code == 0 and report["complete"] is False


def test_the_shipped_price_list_names_every_model_in_the_roster():
    prices = json.loads((lib.EVALS_DIR / "prices.json").read_text())
    names = {m["name"] for m in models.load_models(lib.EVALS_DIR / "models.json")}
    assert names <= set(prices["models"])
    assert prices["source"] == "https://aws.amazon.com/bedrock/pricing"
