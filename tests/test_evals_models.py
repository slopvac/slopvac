"""Model-eval tests: fact alignment, task building, scoring, planning and summaries.

No model is called. Fake models answer from the prompt.
"""

from __future__ import annotations

import json
import os
import random
import re
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path

import pytest

from evals import __main__ as cli
from evals import build_derived, facts, models, runner, textops
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
        ("mechanical", "reviewer"),
        ("update", "reviewer"),
    }
    assert len({t["id"] for t in a}) == len(a)


def test_planted_review_tasks_carry_both_texts_a_quote_may_come_from(corpus):
    manifest, cache = corpus
    planted = [
        t
        for t in models.build_tasks(manifest, cache, per_type=4)
        if t.get("variant") == "planted"
    ]
    assert planted
    for task in planted:
        assert all(e["sentence"] in task["passage"] for e in task["edits"])
        assert task["passage"].count("\n") >= 1


@pytest.mark.parametrize("kind", ["mechanical", "update"])
def test_planted_review_tasks_of_the_author_types_plant_damage_the_candidate_lacks(
    corpus, kind
):
    manifest, cache = corpus
    planted = [
        t
        for t in models.build_tasks(manifest, cache, per_type=6)
        if t.get("variant") == "planted" and t["type"] == kind
    ]
    assert planted
    for task in planted:
        assert task["edits"]
        for edit in task["edits"]:
            assert edit["sentence"] in task["original"]
            assert edit["sentence"] not in task["candidate"]
        assert task["passage"] == task["original"] + "\n" + task["candidate"]


@pytest.mark.parametrize(
    ("damage", "kind"),
    [("dropped", "drop"), ("changed", "number")],
)
def test_damage_sentence_plants_the_named_kind(damage, kind):
    sentence = "The queue holds 63 jobs."
    kept, detail = build_derived.damage_sentence(sentence, kind, [], random.Random("x"))
    if kind == "drop":
        assert (kept, detail) == (None, "dropped")
    else:
        # 63 plus one of 1, 2, 5 or 10.
        assert kept in {f"The queue holds {n} jobs." for n in (64, 65, 68, 73)}


@pytest.mark.parametrize(
    ("sentence", "kind", "expected"),
    [
        ("The service must restart.", "negation", "The service must not restart."),
        ("Use `aa` here.", "entity", "Use `bb` here."),
    ],
)
def test_damage_sentence_rewrites_one_fact(sentence, kind, expected):
    kept, _ = build_derived.damage_sentence(
        sentence, kind, ["aa", "bb"], random.Random("x")
    )
    assert kept == expected


@pytest.mark.parametrize(
    ("sentence", "kind"),
    [
        ("The service restarts quickly.", "drop"),
        ("The service restarts quickly.", "number"),
        ("The service restarts quickly.", "negation"),
        ("The service restarts quickly.", "entity"),
        ("The service must not restart.", "negation"),
    ],
)
def test_damage_sentence_declines_a_sentence_with_nothing_to_damage(sentence, kind):
    assert (
        build_derived.damage_sentence(sentence, kind, ["zz"], random.Random("x"))
        is None
    )


def planted_tasks_from(tmp_path, kind, parent_text, **source):
    """The planted review tasks of `kind` that `build_tasks` makes from one parent."""
    docs = tmp_path / "docs"
    docs.mkdir()
    (docs / "parent.txt").write_text(parent_text + "\n")
    (docs / "pair.txt").write_text(parent_text + "\n")
    manifest = {
        "documents": [
            {
                "id": "parent",
                "origin": "seeded-defect" if kind == "mechanical" else "human",
                "format": "text",
                "genre": "g",
                "source": source.get("parent", {}),
            },
            {
                "id": "pair",
                "origin": "rewrite-pair",
                "format": "text",
                "genre": "g",
                "source": {"parents": ["parent"], "edits": source.get("edits", [])},
            },
        ]
    }
    built = models.build_tasks(manifest, tmp_path, per_type=12)
    return [t for t in built if t["type"] == kind and t["role"] == "reviewer"]


MECHANICAL_TEXT = (
    "# Title\n\nThe queue holds 63 jobs before it blocks writers. "
    "We leverage the cache to speed up reads.\n\nTail."
)


def test_a_mechanical_planted_task_leaves_the_flagged_sentence_alone(tmp_path):
    flagged = "We leverage the cache to speed up reads."
    (task,) = planted_tasks_from(
        tmp_path,
        "mechanical",
        MECHANICAL_TEXT,
        parent={"defects": [{"line": 3, "rule": "r", "text": flagged}]},
    )
    # The only other sentence with a fact is the queue one: it is the damage.
    assert [e["sentence"] for e in task["edits"]] == [
        "The queue holds 63 jobs before it blocks writers."
    ]
    assert flagged in task["candidate"]
    assert "63" not in task["candidate"]


UPDATE_TEXT = (
    "Intro paragraph here. The cache holds 40 items in memory. "
    "Run `purge` to empty it before restart."
)


@pytest.fixture
def update_planted(tmp_path):
    (task,) = planted_tasks_from(
        tmp_path,
        "update",
        UPDATE_TEXT,
        edits=[
            {
                "kind": "number",
                "detail": "40 -> 50",
                "original_sentence": "The cache holds 40 items in memory.",
                "line": 1,
            }
        ],
    )
    return task


def test_an_update_planted_task_makes_the_stated_change_and_plants_damage_elsewhere(
    update_planted,
):
    assert update_planted["candidate"] == (
        "Intro paragraph here. The cache holds 50 items in memory."
    )
    assert [e["sentence"] for e in update_planted["edits"]] == [
        "Run `purge` to empty it before restart."
    ]


def test_an_update_planted_review_prompt_names_the_change_as_intended(update_planted):
    assert "replace 40 with 50" in update_planted["prompt"]
    assert update_planted["original"] in update_planted["prompt"]
    assert update_planted["candidate"] in update_planted["prompt"]


def update_tasks_of(tmp_path, paragraph, old="40", new="50", line=1):
    """The update planted tasks from a one-paragraph parent whose edit starts `line`."""
    return planted_tasks_from(
        tmp_path,
        "update",
        paragraph,
        edits=[
            {
                "kind": "number",
                "detail": f"{old} -> {new}",
                "original_sentence": textops.split_sentences(paragraph)[0],
                "line": line,
            }
        ],
    )


def one_update_task(tmp_path, paragraph, old="40", new="50"):
    (task,) = update_tasks_of(tmp_path, paragraph, old, new)
    return task


QUEUE_40 = "The queue holds 40 jobs before it blocks writers."
QUEUE_50 = "The queue holds 50 jobs before it blocks writers."
QUEUE_63 = "The queue holds 63 jobs before it blocks writers."
QUEUE_68 = "The queue holds 68 jobs before it blocks writers."
CACHE_12 = "The cache keeps 12 entries per user today."
CACHE_17 = "The cache keeps 17 entries per user today."
PURGE = "Run `purge` to empty it before restart."


def intended_task(sentence=CACHE_12, after=CACHE_17):
    return {
        "type": "update",
        "variant": "planted",
        "passage": f"{QUEUE_40}\n{sentence}\n{QUEUE_50}\n{after}",
        "edits": [{"kind": "number", "sentence": sentence, "after": after}],
        "intended": [QUEUE_40, QUEUE_50],
    }


@pytest.mark.parametrize(
    ("quote", "found", "alarms"),
    [
        (CACHE_12, 1, 0),  # the damaged sentence as the original has it
        (CACHE_17, 1, 0),  # the damaged sentence as the candidate has it
        (QUEUE_40, 0, 0),  # the intended change, original side: not an alarm
        (QUEUE_50, 0, 0),  # the intended change, candidate side
        ("Totally unrelated heading text here", 0, 1),
    ],
)
def test_a_finding_counts_by_what_it_matches_not_by_which_is_closer(
    quote, found, alarms
):
    answer = json.dumps([{"quote": quote.rstrip("."), "problem": "number changed"}])
    scored = models.score_review(intended_task(), answer)
    assert (scored["found"], scored["false_alarms"]) == (found, alarms)


def test_a_candidate_side_quote_of_damage_that_differs_from_the_change_by_numbers_only():
    # The damaged candidate sentence overlaps both intended sentences as much as it
    # overlaps the damaged original, so only the planted text can tell them apart.
    task = intended_task(QUEUE_63, QUEUE_68)
    answer = json.dumps([{"quote": QUEUE_68.rstrip("."), "problem": "number changed"}])
    scored = models.score_review(task, answer)
    assert (scored["found"], scored["false_alarms"], scored["success"]) == (1, 0, True)


def test_the_builder_rejects_damage_it_cannot_tell_from_the_intended_change(
    tmp_path, caplog
):
    # Both queue sentences differ by a number only: either could be the damage.
    assert update_tasks_of(tmp_path, f"{QUEUE_40} {QUEUE_63}") == []
    assert [r.getMessage() for r in caplog.records] == [
        (
            "skipped planted review candidates "
            "(update: no sentence to damage that the stated change leaves distinct x1)"
        )
    ]


def test_the_builder_plants_the_damage_on_a_sentence_apart_from_the_intended_change(
    tmp_path, caplog
):
    task = one_update_task(tmp_path, f"{QUEUE_40} {QUEUE_63} {PURGE}")
    assert [e["sentence"] for e in task["edits"]] == [PURGE]
    assert task["intended"] == [QUEUE_40, QUEUE_50]
    assert caplog.records == []
    only_intended = json.dumps([{"quote": QUEUE_40[:-1], "problem": "number changed"}])
    scored = models.score_review(task, only_intended)
    assert (scored["found"], scored["false_alarms"], scored["success"]) == (
        0,
        0,
        False,
    )
    damaged = json.dumps([{"quote": PURGE[:-1], "problem": "dropped"}])
    assert models.score_review(task, damaged)["success"]


def test_shared_words_with_the_damaged_sentence_are_not_a_detection(tmp_path):
    intended = "The service holds 40 items in the shared cache during startup."
    damaged = "The shared cache keeps 12 entries per user today."
    task = one_update_task(tmp_path, f"{intended} {damaged}")
    assert [e["sentence"] for e in task["edits"]] == [damaged]
    # Three of these five words are also in the damaged sentence, but the quote
    # lies in the intended sentence, and the complaint is about 40 -> 50 only.
    answer = json.dumps(
        [{"quote": "the shared cache during startup", "problem": "40 became 50"}]
    )
    scored = models.score_review(task, answer)
    assert (scored["found"], scored["success"]) == (0, False)


def test_a_quote_inside_the_damaged_sentence_still_detects_it(tmp_path):
    intended = "The service holds 40 items in the shared cache during startup."
    damaged = "The shared cache keeps 12 entries per user today."
    task = one_update_task(tmp_path, f"{intended} {damaged}")
    answer = json.dumps([{"quote": "keeps 12 entries per user", "problem": "number"}])
    assert models.score_review(task, answer)["success"]


COPY_PAIRS = [("40", "50"), ("7", "9"), ("12", "13"), ("21", "22"), ("64", "65")]


@pytest.mark.parametrize(("old", "new"), COPY_PAIRS)
def test_a_change_that_turns_one_sentence_into_a_copy_of_another_leaves_no_damage_site(
    tmp_path, old, new
):
    first = f"The shared cache holds {old} items in memory today."
    second = f"The shared cache holds {new} items in memory today."
    assert update_tasks_of(tmp_path, f"{first} {second}", old, new) == []


@pytest.mark.parametrize(("old", "new"), COPY_PAIRS)
def test_damage_beside_a_copy_lands_on_the_sentence_that_is_not_one(tmp_path, old, new):
    first = f"The shared cache holds {old} items in memory today."
    second = f"The shared cache holds {new} items in memory today."
    task = one_update_task(tmp_path, f"{first} {second} {PURGE}", old, new)
    assert [e["sentence"] for e in task["edits"]] == [PURGE]
    assert task["intended"] == [first, second]
    for quote in (first, second):
        answer = json.dumps([{"quote": quote.rstrip("."), "problem": "x"}])
        scored = models.score_review(task, answer)
        assert (scored["found"], scored["false_alarms"]) == (0, 0)


def test_the_builder_reports_a_paragraph_it_cannot_find(tmp_path, caplog):
    assert update_tasks_of(tmp_path, f"{QUEUE_40} {PURGE}", line=5) == []
    assert [r.getMessage() for r in caplog.records] == [
        "skipped planted review candidates (update: stated paragraph not found x1)"
    ]


@pytest.mark.parametrize(
    "neighbour", ["40.5", "40,500", "1,40", "3.40", "40.5.1", ".40"]
)
def test_the_update_changes_whole_numbers_only(tmp_path, neighbour):
    paragraph = (
        f"The limit is {neighbour} seconds and the quota is 40 items. "
        "Run `purge` to empty it before restart."
    )
    task = one_update_task(tmp_path, paragraph)
    assert task["candidate"] == (
        f"The limit is {neighbour} seconds and the quota is 50 items."
    )


def test_update_tasks_carry_the_stated_change_and_it_is_in_the_passage(corpus):
    manifest, cache = corpus
    for task in (
        t
        for t in models.build_tasks(manifest, cache, per_type=6)
        if t["type"] == "update" and t["role"] == "author"
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


@pytest.mark.parametrize(
    "output",
    [
        "Cache 40 items now.",
        "Cache   40\nitems now.\n",
        "```\nCache 40 items now.\n```",
    ],
)
def test_an_unchanged_rewrite_fails(output):
    task = {"type": "rewrite", "source": "Cache 40 items now.", "expected": []}
    scored = models.score_author(task, output, NO_FINDINGS)
    assert (scored["unchanged"], scored["success"]) == (True, False)


def test_author_scores_are_labelled_a_lexical_retention_smoke_check():
    task = {"type": "rewrite", "source": "Cache 40 items now.", "expected": []}
    scored = models.score_author(task, "Keep 40 items cached.", NO_FINDINGS)
    assert scored["metric"] == "lexical retention smoke check"


LEXICAL_ORACLE = pytest.mark.xfail(
    strict=True,
    reason="lexical oracle; meaning-level oracle tracked in slopvac-8l4.15.24",
)


@LEXICAL_ORACLE
@pytest.mark.parametrize(
    ("source", "output"),
    [
        ("Set min 5, max 10 for the pool.", "Set min 10, max 5 for the pool."),
        (
            "Never skip the check; always run it.",
            "Always skip the check; never run it.",
        ),
    ],
    ids=["swapped-limits", "moved-negation"],
)
def test_a_rewrite_that_changes_meaning_without_losing_tokens_fails(source, output):
    task = {"type": "rewrite", "source": source, "expected": []}
    assert not models.score_author(task, output, NO_FINDINGS)["success"]


def test_a_code_fence_around_the_answer_is_ignored():
    task = {"type": "rewrite", "source": "Cache 40 items."}
    scored = models.score_author(task, "```markdown\nCache 40 items.\n```", NO_FINDINGS)
    assert scored["fact_rate"] == 1.0


PLANTED_PASSAGE = (
    "The queue holds 63 jobs before it blocks writers. "
    "Run the migration with the `--offline` flag first.\n"
    "The queue holds 68 jobs before it blocks writers. "
    "Run the migration with the flag first."
)


def planted_task():
    return {
        "variant": "planted",
        "passage": PLANTED_PASSAGE,
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
    task = {
        "variant": "authored",
        "lost": ["--offline", "3"],
        "passage": "Run with the --offline flag first.",
    }
    answer = json.dumps(
        [{"quote": "with the --offline flag", "problem": "the --offline flag vanished"}]
    )
    scored = models.score_review(task, answer)
    assert (scored["found"], scored["planted"]) == (1, 2)


def test_a_denial_with_the_right_quote_does_not_count_as_a_finding():
    answer = json.dumps(
        [
            {
                "quote": "The queue holds 63 jobs before it blocks writers",
                "problem": "The 63 is preserved, so there is no change here.",
            }
        ]
    )
    scored = models.score_review(planted_task(), answer)
    assert (scored["found"], scored["false_alarms"]) == (0, 0)


QUEUE_QUOTE = "The queue holds 63 jobs before it blocks writers"


def planted_found(problem):
    answer = json.dumps([{"quote": QUEUE_QUOTE, "problem": problem}])
    return models.score_review(planted_task(), answer)["found"]


@pytest.mark.parametrize(
    "problem",
    [
        "The original still says 63, but the candidate changed the cap to 68.",
        "The 63 is not preserved: it became 68.",
        "The cap is unchanged in name, but the number changed from 63 to 68.",
        "The candidate no longer says 63.",
        "The limit isn't kept; the candidate now says 68.",
        "Not unchanged: 63 was replaced by 68.",
        "The original still says 63, but the candidate omits that limit.",
        "The limit is preserved in the original; the candidate drops it.",
        "The candidate keeps the heading but removes the 63 cap.",
        "The original has 63 and is unchanged, yet the candidate deletes it.",
        "The original still says max 63. The candidate omitted it.",
        "The original still states 63, and the candidate dropped it.",
        "The cap is preserved in the source, but the candidate removed it.",
        "The source still reads 63; the candidate has deleted the cap.",
        "The original still says 63, but the candidate loses the limit.",
    ],
)
def test_an_explanation_that_affirms_a_change_counts(problem):
    assert planted_found(problem) == 1


SHORT_PASSAGE = "Run the migration with the `--offline` flag first. Retry 3 times."


def authored_found(quote, lost, passage=SHORT_PASSAGE):
    task = {"variant": "authored", "lost": [lost], "passage": passage}
    answer = json.dumps([{"quote": quote, "problem": f"{lost} was dropped"}])
    return models.score_review(task, answer)["found"]


@pytest.mark.parametrize(
    ("quote", "lost"),
    [("--offline", "--offline"), ("3", "3"), ("`--offline`", "--offline")],
    ids=["flag", "number", "code-span"],
)
def test_a_short_genuine_quote_of_a_whole_fact_counts(quote, lost):
    assert authored_found(quote, lost) == 1


@pytest.mark.parametrize(
    ("quote", "lost", "passage"),
    [
        ("flag first the with migration", "flag", SHORT_PASSAGE),
        ("Retry times 3", "3", SHORT_PASSAGE),
        ("3", "3", "Retry 13 times."),
        ("an invented sentence about the --offline flag", "--offline", SHORT_PASSAGE),
    ],
    ids=["reordered-words", "reordered-fact", "inside-longer-number", "invented"],
)
def test_a_quote_that_is_not_contiguous_in_the_passage_does_not_count(
    quote, lost, passage
):
    assert authored_found(quote, lost, passage) == 0


def test_a_planted_quote_of_real_words_in_another_order_is_a_false_alarm():
    answer = json.dumps(
        [{"quote": "jobs queue the 63 holds blocks it writers", "problem": "changed"}]
    )
    scored = models.score_review(planted_task(), answer)
    assert (scored["found"], scored["false_alarms"]) == (0, 1)


def test_a_planted_quote_found_in_neither_passage_is_a_false_alarm():
    answer = json.dumps(
        [{"quote": "The cache evicts entries after an hour", "problem": "changed"}]
    )
    scored = models.score_review(planted_task(), answer)
    assert (scored["found"], scored["false_alarms"]) == (0, 1)


def test_a_planted_quote_of_a_single_digit_fact_locates_the_edit():
    task = {
        "variant": "planted",
        "edits": [
            {"kind": "number", "sentence": "Set `replicas` to 1 in the ConfigMap."}
        ],
        "passage": (
            "Set `replicas` to 1 in the ConfigMap.\n"
            "Set `replicas` to 2 in the ConfigMap."
        ),
    }
    answer = json.dumps([{"quote": "1", "problem": "the value changed to 2"}])
    scored = models.score_review(task, answer)
    assert (scored["found"], scored["false_alarms"]) == (1, 0)


def test_a_planted_quote_of_a_stray_letter_is_still_a_false_alarm():
    task = {
        "variant": "planted",
        "edits": [{"kind": "number", "sentence": "Set a limit of 1 in the file."}],
        "passage": "Set a limit of 1 in the file.\nSet a limit of 2 in the file.",
    }
    answer = json.dumps([{"quote": "a", "problem": "changed"}])
    scored = models.score_review(task, answer)
    assert (scored["found"], scored["false_alarms"]) == (0, 1)


def test_a_short_planted_quote_of_the_dropped_flag_locates_the_edit():
    answer = json.dumps([{"quote": "--offline", "problem": "the flag was dropped"}])
    scored = models.score_review(planted_task(), answer)
    assert (scored["found"], scored["false_alarms"]) == (1, 0)


@pytest.mark.parametrize(
    "problem",
    [
        "The 63 is preserved.",
        "No change here.",
        "This sentence is unchanged.",
        "The candidate keeps the 63.",
        "Nothing is lost; the original still says 63.",
        "The number remains intact and is the same as the original.",
    ],
)
def test_an_explanation_that_only_denies_the_change_does_not_count(problem):
    assert planted_found(problem) == 0


def test_an_authored_review_needs_a_quote_that_is_in_the_passage():
    task = {
        "variant": "authored",
        "lost": ["--offline"],
        "passage": "Run the migration with the `--offline` flag first. Original text.",
    }
    elsewhere = json.dumps(
        [{"quote": "an invented sentence about weather", "problem": "--offline lost"}]
    )
    quoted = json.dumps(
        [
            {
                "quote": "Run the migration with the --offline flag first",
                "problem": "the --offline flag vanished",
            }
        ]
    )
    assert models.score_review(task, elsewhere)["found"] == 0
    assert models.score_review(task, quoted)["found"] == 1


def test_an_authored_denial_does_not_count_as_a_finding():
    task = {
        "variant": "authored",
        "lost": ["--offline"],
        "passage": "Run the migration with the `--offline` flag first.",
    }
    answer = json.dumps(
        [
            {
                "quote": "Run the migration with the --offline flag first",
                "problem": "the --offline flag still says what it did",
            }
        ]
    )
    assert models.score_review(task, answer)["found"] == 0


def test_reviewer_scores_are_labelled_detection_by_quotation():
    assert (
        models.score_review(planted_task(), "[]")["metric"] == "detection by quotation"
    )


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
        "passage": "alpha beta gamma delta\nalpha beta gamma epsilon",
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
            return (
                "Keep 40 items cached." if model["name"] == "good" else "Cache items."
            )
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


def counting_ask(calls):
    def make(model):
        inner = fake_ask(model)

        def ask(prompt):
            calls.append(model["name"])
            return inner(prompt)

        return ask

    return make


def row_keys(out):
    return {(r["task"], r["model"], r["repeat"]) for r in out["runs"]}


def test_plan_for_one_model_counts_only_the_calls_it_adds():
    # Model "c" among 3, 3 repeats, tasks: update, rewrite, planted review.
    # Own answers 3x3=9; "c" is reviewed on its rewrite by 2 models (3x2=6) and
    # reviews the 2 others' rewrites from the earlier journal (6). The others
    # make no author calls: 9 + 6 + 6 = 21.
    counted = models.plan(tiny_tasks(), roster("a", "b", "c"), 3, only_model="c")
    assert counted["total_calls"] == 21
    assert counted["authored_review_calls"] == 12
    assert list(counted["per_model"]) == ["c"]


def test_plan_rejects_an_only_model_outside_the_roster():
    with pytest.raises(models.ModelEvalError, match="not in the models file"):
        models.plan(tiny_tasks(), roster("a", "b"), 3, only_model="z")


def test_only_model_reuses_the_earlier_rewrites_and_joins_the_earlier_run(tmp_path):
    tasks = tiny_tasks()
    path = tmp_path / "earlier.journal.jsonl"
    earlier = models.execute(
        tasks,
        roster("a", "b"),
        3,
        NO_FINDINGS,
        ask_for=fake_ask,
        date="2026-11-14",
        journal=models.Journal(path),
    )
    before = path.read_bytes()
    calls: list[str] = []
    added = models.execute(
        tasks,
        roster("a", "b", "c"),
        3,
        NO_FINDINGS,
        ask_for=counting_ask(calls),
        date="2026-11-17",
        only_model="c",
        candidates=models.Journal(path),
    )
    assert len(calls) == added["planned_calls"] == 21
    # c answers 9 tasks and reviews 6 rewrites; a and b review c's rewrite 3 times each.
    assert {m: calls.count(m) for m in "abc"} == {"a": 3, "b": 3, "c": 15}
    assert len(added["runs"]) == 21
    assert all("c" in (r["model"], r.get("author_model")) for r in added["runs"])
    assert path.read_bytes() == before
    full = models.execute(
        tasks, roster("a", "b", "c"), 3, NO_FINDINGS, ask_for=fake_ask, date="d"
    )
    merged = models.merge_results(earlier, added)
    assert row_keys(merged) == row_keys(full)
    assert merged["date"] == "2026-11-17"
    assert [m["name"] for m in merged["models"]] == ["a", "b", "c"]


def test_a_missing_candidate_stops_the_run_before_any_call(tmp_path):
    calls: list[str] = []
    with pytest.raises(models.ModelEvalError, match="missing.*no call was made"):
        models.execute(
            tiny_tasks(),
            roster("a", "b", "c"),
            3,
            NO_FINDINGS,
            ask_for=counting_ask(calls),
            only_model="c",
            candidates=models.Journal(tmp_path / "empty.journal.jsonl"),
        )
    assert calls == []


def test_only_model_needs_a_candidates_journal_and_the_reverse(tmp_path):
    with pytest.raises(models.ModelEvalError, match="go together"):
        models.execute(
            tiny_tasks(),
            roster("a", "b"),
            3,
            NO_FINDINGS,
            ask_for=fake_ask,
            only_model="b",
        )


@pytest.fixture
def frozen(tmp_path):
    """A finished earlier run of a and b: its journal, as --candidates-from reads it."""
    path = tmp_path / "earlier.journal.jsonl"
    models.execute(
        tiny_tasks(),
        roster("a", "b"),
        3,
        NO_FINDINGS,
        ask_for=fake_ask,
        journal=models.Journal(path),
    )
    return path


def add_c_run(monkeypatch, candidates, out, *, command="run"):
    """Drive `models run` for the new model c through the CLI; return its exit code."""
    monkeypatch.setattr(
        cli, "_model_tasks", lambda args: (tiny_tasks(), roster("a", "b", "c"))
    )
    argv = ["models", command, "--models", "unused"]
    argv += ["--only-model", "c", "--candidates-from", str(candidates)]
    if command == "run":
        argv += ["--confirm-calls", "21", "--max-invocations", "30", "--out", str(out)]
    return cli.main(argv)


@pytest.mark.parametrize(
    "alias",
    ["out", "journal", "ledger", "lock", "hard link to the journal"],
)
def test_the_frozen_journal_is_never_a_writable_destination(
    alias, frozen, tmp_path, monkeypatch, capsys
):
    out = tmp_path / "x.json"
    if alias == "hard link to the journal":
        out = tmp_path / "other.json"
        os.link(frozen, tmp_path / "other.journal.jsonl")
        candidates = frozen
    else:
        candidates = {
            "out": out,
            "journal": tmp_path / "x.journal.jsonl",
            "ledger": tmp_path / "x.ledger.jsonl",
            "lock": tmp_path / "x.ledger.jsonl.lock",
        }[alias]
        candidates.write_bytes(frozen.read_bytes())
    before = candidates.read_bytes()
    assert add_c_run(monkeypatch, candidates, out) == 2
    assert "read-only" in capsys.readouterr().err
    assert candidates.read_bytes() == before
    assert not (tmp_path / "x.ledger.jsonl").exists() or alias == "ledger"


@pytest.fixture
def frozen_with_c(tmp_path):
    """A finished earlier run that already includes model c."""
    path = tmp_path / "with-c.journal.jsonl"
    models.execute(
        tiny_tasks(),
        roster("a", "b", "c"),
        3,
        NO_FINDINGS,
        ask_for=fake_ask,
        journal=models.Journal(path),
    )
    return path


@pytest.mark.parametrize("command", ["plan", "run"])
def test_a_selection_the_frozen_journal_already_ran_is_refused_before_any_call(
    command, frozen_with_c, tmp_path, monkeypatch, capsys
):
    # c ran 9 answers (3 tasks x 3 repeats) and reviewed 2 authors x 3 repeats = 6.
    assert (
        add_c_run(monkeypatch, frozen_with_c, tmp_path / "y.json", command=command) == 2
    )
    assert "15 jobs of c already ran" in capsys.readouterr().err
    assert not (tmp_path / "y.ledger.jsonl").exists()


def failing_c(model):
    if model["name"] != "c":
        return fake_ask(model)

    def ask(prompt):
        raise models.CallFailed("boom", 1)

    return ask


@pytest.fixture
def frozen_with_failed_c(tmp_path):
    """An earlier run in which every call of model c failed."""
    path = tmp_path / "failed-c.journal.jsonl"
    models.execute(
        tiny_tasks(),
        roster("a", "b", "c"),
        3,
        NO_FINDINGS,
        ask_for=failing_c,
        journal=models.Journal(path),
    )
    return path


@pytest.mark.parametrize("command", ["plan", "run"])
def test_failed_calls_in_the_frozen_journal_still_count_as_already_run(
    command, frozen_with_failed_c, tmp_path, monkeypatch, capsys
):
    out = tmp_path / "z.json"
    assert add_c_run(monkeypatch, frozen_with_failed_c, out, command=command) == 2
    assert "15 jobs of c already ran" in capsys.readouterr().err
    assert not (tmp_path / "z.ledger.jsonl").exists()


def test_execute_makes_no_call_for_a_model_whose_frozen_calls_all_failed(
    frozen_with_failed_c,
):
    calls: list[str] = []
    with pytest.raises(models.ModelEvalError, match="15 jobs of c already ran"):
        models.execute(
            tiny_tasks(),
            roster("a", "b", "c"),
            3,
            NO_FINDINGS,
            ask_for=counting_ask(calls),
            only_model="c",
            candidates=models.Journal(frozen_with_failed_c),
        )
    assert calls == []


def test_resume_on_the_runs_own_journal_still_retries_failed_jobs(tmp_path):
    journal = tmp_path / "own.journal.jsonl"
    first = models.execute(
        tiny_tasks(),
        roster("a", "b", "c"),
        3,
        NO_FINDINGS,
        ask_for=failing_c,
        journal=models.Journal(journal),
    )
    assert first["failed_calls"] > 0
    calls: list[str] = []
    second = models.execute(
        tiny_tasks(),
        roster("a", "b", "c"),
        3,
        NO_FINDINGS,
        ask_for=counting_ask(calls),
        journal=models.Journal(journal),
    )
    assert second["failed_calls"] == 0
    assert calls.count("c") >= 9  # c's own failed answers were asked again


def test_execute_makes_no_call_for_a_selection_the_frozen_journal_ran(frozen_with_c):
    calls: list[str] = []
    with pytest.raises(models.ModelEvalError, match="15 jobs of c already ran"):
        models.execute(
            tiny_tasks(),
            roster("a", "b", "c"),
            3,
            NO_FINDINGS,
            ask_for=counting_ask(calls),
            only_model="c",
            candidates=models.Journal(frozen_with_c),
        )
    assert calls == []


def test_execute_refuses_a_journal_that_is_also_the_candidates(frozen):
    calls: list[str] = []
    with pytest.raises(models.ModelEvalError, match="read-only"):
        models.execute(
            tiny_tasks(),
            roster("a", "b", "c"),
            3,
            NO_FINDINGS,
            ask_for=counting_ask(calls),
            only_model="c",
            candidates=models.Journal(frozen),
            journal=models.Journal(frozen),
        )
    assert calls == []


def results_file(model, *, scorer=None, run_scorer=None, **fields):
    """A small results file for one model, built apart from `execute`."""
    run = {
        "task": "t1",
        "type": "rewrite",
        "role": "author",
        "variant": None,
        "model": model,
        "repeat": 0,
        "score": {"success": True},
    }
    if run_scorer is not None:
        run["scorer_version"] = run_scorer
    doc = {
        "schema": models.RESULTS_SCHEMA,
        "date": "2026-11-17",
        "prompt_version": "1",
        "tasks_sha256": "h",
        "repeats": 3,
        "models": [{"name": model, "family": "f", "tier": "t"}],
        "runs": [run],
    }
    if scorer is not None:
        doc["scorer_version"] = scorer
    return doc | fields


def test_a_join_of_two_scorer_versions_is_refused():
    with pytest.raises(models.ModelEvalError, match="scorer versions differ"):
        models.merge_results(
            results_file("a", scorer="2"), results_file("b", scorer="3")
        )


def test_legacy_results_without_a_scorer_version_join_only_each_other():
    models.merge_results(results_file("a"), results_file("b"))
    with pytest.raises(models.ModelEvalError, match="legacy"):
        models.merge_results(results_file("a"), results_file("b", scorer="2"))


def test_a_run_row_with_another_scorer_version_than_its_file_is_refused():
    with pytest.raises(models.ModelEvalError, match="disagrees with its file header"):
        models.merge_results(
            results_file("a", scorer="2"), results_file("b", scorer="2", run_scorer="1")
        )


def test_a_row_without_its_own_version_takes_the_file_headers():
    a = results_file("a", scorer="2")  # the row has no scorer_version field
    b = results_file("b", scorer="2", run_scorer="2")
    assert (
        models.summarize(models.merge_results(a, b), resamples=5)["scorer_version"]
        == "2"
    )
    with pytest.raises(models.ModelEvalError, match="scorer versions differ"):
        models.merge_results(a, results_file("c", scorer="3", run_scorer="3"))


def test_a_file_without_a_header_is_legacy_and_its_rows_cannot_claim_a_version():
    assert "scorer_version" not in models.summarize(results_file("a"), resamples=5)
    with pytest.raises(models.ModelEvalError, match="legacy"):
        models.summarize(results_file("a", run_scorer="2"), resamples=5)


def test_a_row_that_disagrees_with_its_header_fails_even_in_a_single_file(
    tmp_path, capsys
):
    path = tmp_path / "bad.json"
    path.write_text(json.dumps(results_file("a", scorer="2", run_scorer="1")))
    assert cli.main(["models", "summarize", str(path)]) == 2
    assert "disagrees with its file header" in capsys.readouterr().err


def test_the_joined_summary_carries_the_shared_scorer_version():
    joined = models.merge_results(
        results_file("a", scorer="2"), results_file("b", scorer="2")
    )
    assert models.summarize(joined, resamples=5)["scorer_version"] == "2"
    legacy = models.merge_results(results_file("a"), results_file("b"))
    assert "scorer_version" not in models.summarize(legacy, resamples=5)


def test_summarize_exits_2_for_a_join_across_scorer_versions(tmp_path, capsys):
    paths = []
    for name, version in (("a", "2"), ("b", "3")):
        path = tmp_path / f"{name}.json"
        path.write_text(json.dumps(results_file(name, scorer=version)))
        paths.append(str(path))
    assert cli.main(["models", "summarize", *paths]) == 2
    assert "scorer versions differ" in capsys.readouterr().err


@pytest.mark.parametrize(
    ("field", "other", "message"),
    [
        ("schema", models.RESULTS_SCHEMA + 1, "schema differs"),
        ("prompt_version", "2", "prompt_version differs"),
        ("repeats", 5, "repeats differs"),
        ("tasks_sha256", "other-task-set", "tasks_sha256 differs"),
    ],
)
def test_a_join_refuses_files_that_differ_in_one_comparability_field(
    field, other, message
):
    with pytest.raises(models.ModelEvalError, match=message):
        models.merge_results(results_file("a"), results_file("b", **{field: other}))


@pytest.mark.parametrize("field", ["family", "tier"])
def test_a_join_refuses_a_model_whose_metadata_differs_between_files(field):
    changed = results_file("a")
    changed["models"] = [{"name": "a", "family": "f", "tier": "t", field: "other"}]
    changed["runs"][0]["repeat"] = 1  # no shared run, so only the model differs
    with pytest.raises(models.ModelEvalError, match="model 'a' differs"):
        models.merge_results(results_file("a"), changed)


def test_merge_refuses_results_from_another_task_set_or_with_a_repeated_run():
    out = models.execute(
        tiny_tasks(), roster("a", "b"), 3, NO_FINDINGS, ask_for=fake_ask, date="d"
    )
    with pytest.raises(models.ModelEvalError, match="tasks_sha256 differs"):
        models.merge_results(out, out | {"tasks_sha256": "other"})
    with pytest.raises(models.ModelEvalError, match="duplicate run"):
        models.merge_results(out, out)


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


def fresh_results():
    tasks = [t for t in tiny_tasks() if t["id"] != "a1"]
    return models.execute(
        tasks,
        roster("good", "poor"),
        3,
        NO_FINDINGS,
        ask_for=fake_ask,
        date="2026-11-14",
    )


def as_legacy(results):
    """The shape of a results file written before scorer versions existed."""
    legacy = {k: v for k, v in results.items() if k != "scorer_version"}
    legacy["runs"] = [
        {k: v for k, v in r.items() if k != "scorer_version"} for r in results["runs"]
    ]
    return legacy


def test_results_and_every_run_row_record_the_scorer_version():
    out = fresh_results()
    assert out["scorer_version"] == models.SCORER_VERSION
    assert {r["scorer_version"] for r in out["runs"]} == {models.SCORER_VERSION}


def test_a_summary_of_freshly_scored_runs_reports_the_scorer_and_metric_names():
    summary = models.summarize(fresh_results(), resamples=20)
    assert summary["scorer_version"] == models.SCORER_VERSION
    labels = {(r["role"], r["metric"], r["scorer_version"]) for r in summary["rows"]}
    assert labels == {
        ("author", "lexical retention smoke check", models.SCORER_VERSION),
        ("reviewer", "detection by quotation", models.SCORER_VERSION),
    }


def test_a_summary_of_legacy_scores_does_not_wear_the_new_metric_names():
    summary = models.summarize(as_legacy(fresh_results()), resamples=20)
    assert "scorer_version" not in summary
    assert {r["metric"] for r in summary["rows"]} == {"legacy (pre-15.17)"}
    assert {r["scorer_version"] for r in summary["rows"]} == {"legacy"}


def test_the_committed_legacy_results_are_labelled_legacy():
    path = Path(models.RESULTS_DIR) / "2026-11-14.json"
    summary = models.summarize(models.load(path), resamples=20)
    assert {r["metric"] for r in summary["rows"]} == {"legacy (pre-15.17)"}


def without_row_versions(results):
    out = dict(results)
    out["runs"] = [
        {k: v for k, v in r.items() if k != "scorer_version"} for r in results["runs"]
    ]
    return out


def disagreeing(results):
    out = dict(results)
    out["runs"] = [dict(r) for r in results["runs"]]
    out["runs"][0]["scorer_version"] = "14.0"
    return out


def test_a_row_without_a_version_inherits_the_file_header():
    summary = models.summarize(without_row_versions(fresh_results()), resamples=20)
    assert summary["scorer_version"] == models.SCORER_VERSION
    assert {r["metric"] for r in summary["rows"]} == {
        "lexical retention smoke check",
        "detection by quotation",
    }


def test_a_row_that_disagrees_with_the_header_is_refused_before_any_ranking():
    with pytest.raises(models.ModelEvalError, match="disagrees"):
        models.summarize(disagreeing(fresh_results()), resamples=20)


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
    task = {
        "variant": "authored",
        "lost": ["3"],
        "passage": "Retry up to three times before the lock gives up.",
    }
    answer = json.dumps(
        [{"quote": "Retry up to three times before the lock", "problem": reply}]
    )
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
    return [t for t in built if t["type"] == "update" and t["role"] == "author"]


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
    return [t for t in built if t["type"] == "update" and t["role"] == "author"]


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


CACHE_USAGE = {
    "input": 200_000,
    "output": 100_000,
    "cache_read": 4_000_000,
    "cache_write": 3_000_000,
}


def cache_priced(**cache):
    return {"models": {"big": {"input": 2, "output": 10, **cache}}}


def test_cache_tokens_are_priced_when_the_model_has_cache_prices(tmp_path, capsys):
    path = write_results(tmp_path, [{"model": "big", "usage": CACHE_USAGE}])
    prices = cache_priced(cache_read=0.2, cache_write=2.5)
    _, report = cost_cli(tmp_path, capsys, path, prices)
    # 0.2M x $2 + 0.1M x $10 + 4M x $0.2 + 3M x $2.5 = 0.4 + 1.0 + 0.8 + 7.5 = 9.7.
    assert report["models"]["big"]["cost_usd"] == pytest.approx(9.7)
    assert report["models"]["big"]["tokens"] == CACHE_USAGE
    assert report["complete"] is True


@pytest.mark.parametrize(
    "cache",
    [{}, {"cache_read": 0.2}, {"cache_write": 2.5}],
    ids=["neither", "read-only", "write-only"],
)
def test_cache_tokens_without_a_cache_price_make_the_cost_incomplete(
    tmp_path, capsys, cache
):
    path = write_results(tmp_path, [{"model": "big", "usage": CACHE_USAGE}])
    _, report = cost_cli(tmp_path, capsys, path, cache_priced(**cache))
    assert report["models"]["big"]["complete"] is False
    assert report["complete"] is False


def test_a_model_without_cache_usage_stays_complete_with_no_cache_price(
    tmp_path, capsys
):
    path = write_results(
        tmp_path, [{"model": "big", "usage": {"input": 1_000_000, "output": 0}}]
    )
    _, report = cost_cli(tmp_path, capsys, path, cache_priced())
    assert report["complete"] is True
    assert report["models"]["big"]["cost_usd"] == pytest.approx(2)


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


def test_recommendations_keep_reviewer_rows_only_with_a_rate_per_model():
    results = results_with({"t1": [True, False, True], "t2": [True, True, True]})
    results["runs"] += [
        {
            "type": "rewrite",
            "role": "reviewer",
            "model": "m",
            "task": task,
            "score": {"success": ok, "planted": 1},
        }
        for task, ok in (("r1", True), ("r1", True), ("r1", False), ("r2", False))
    ]
    out = models.recommendations(results, publish=True)
    assert out["schema_version"] == 2
    assert out["status"] == "published"
    assert out["date"] == "d"
    assert out["repeats"] == 3
    # r1 averages 2/3 and r2 is 0, so the rate is 1/3. Author rows are dropped.
    assert out["rows"] == [
        {
            "review_type": "rewrite",
            "family": "f",
            "tier": "t",
            "model": "m",
            "success_rate": 0.3333,
            "tasks": 2,
            "runs": 4,
        }
    ]


def test_recommendations_publish_nothing_unless_asked():
    results = results_with({"t1": [True, True, True]})
    results["runs"].append(
        {
            "type": "rewrite",
            "role": "reviewer",
            "model": "m",
            "task": "r1",
            "score": {"success": True, "planted": 1},
        }
    )
    assert models.recommendations(results) == {
        "schema_version": 2,
        "status": "pending-reevaluation",
        "date": "d",
        "repeats": 3,
        "rows": [],
    }


def test_the_committed_recommendations_match_the_committed_results():
    results = models.load(Path(models.RESULTS_DIR) / "2026-11-14.json")
    data = lib.EVALS_DIR.parent / "crates/slopvac-agents/data/model-evals.json"
    expected = json.dumps(models.recommendations(results), indent=1, sort_keys=True)
    assert data.read_text(encoding="utf-8") == expected + "\n"


def recommend_results(tmp_path):
    results = results_with({"t1": [True, True, True]})
    results["schema"] = models.RESULTS_SCHEMA
    results["runs"].append(
        {
            "type": "rewrite",
            "role": "reviewer",
            "model": "m",
            "task": "r1",
            "score": {"success": True, "planted": 1},
        }
    )
    path = tmp_path / "results.json"
    path.write_text(json.dumps(results))
    return path


def test_recommend_command_prints_a_pending_file_without_the_flag(tmp_path, capsys):
    path = recommend_results(tmp_path)
    assert cli.main(["models", "recommend", str(path)]) == 0
    assert json.loads(capsys.readouterr().out) == {
        "schema_version": 2,
        "status": "pending-reevaluation",
        "date": "d",
        "repeats": 3,
        "rows": [],
    }


def test_recommend_command_publishes_rows_with_the_flag(tmp_path, capsys):
    path = recommend_results(tmp_path)
    assert cli.main(["models", "recommend", str(path), "--publish"]) == 0
    out = json.loads(capsys.readouterr().out)
    assert out["status"] == "published"
    assert out["rows"] == [
        {
            "review_type": "rewrite",
            "family": "f",
            "tier": "t",
            "model": "m",
            "success_rate": 1.0,
            "tasks": 1,
            "runs": 1,
        }
    ]


def test_recommend_command_writes_the_file_named_by_out(tmp_path, capsys):
    path = recommend_results(tmp_path)
    target = tmp_path / "model-evals.json"
    assert cli.main(["models", "recommend", str(path), "--out", str(target)]) == 0
    assert capsys.readouterr().out == ""
    assert json.loads(target.read_text())["status"] == "pending-reevaluation"


@pytest.mark.parametrize("kind", ["mechanical", "update", "rewrite"])
def test_the_summary_ranks_planted_reviewers_for_every_review_type(kind):
    task = tiny_tasks()[2] | {"type": kind}
    out = models.execute(
        [task], roster("a", "b"), 3, NO_FINDINGS, ask_for=fake_ask, date="2026-11-19"
    )
    ranking = models.summarize(out, resamples=1)["ranking"]
    assert set(ranking[f"{kind}/reviewer"]) == {"a", "b"}
