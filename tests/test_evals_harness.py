"""Offline tests for the eval harness: statistics, scoring, A/B and the runner."""

from __future__ import annotations

import json

import pytest

from evals import ab, metrics, runner, scoring, splits


def doc(doc_id, origin, *, defects=None, controls=None, parents=None, fmt="markdown"):
    source = {}
    if defects is not None:
        source["defects"] = defects
    if controls is not None:
        source["controls"] = controls
    if parents:
        source["parents"] = parents
    return {
        "id": doc_id,
        "origin": origin,
        "format": fmt,
        "source": source,
        "committed": False,
    }


def finding(rule, line):
    return {"rule": rule, "line": line, "severity": "warning"}


def result(words, *findings):
    return {"words": words, "findings": list(findings)}


R1, R2 = "cat.one", "cat.two"

MANIFEST = {
    "documents": [
        doc("h1", "human"),
        doc(
            "s1",
            "seeded-defect",
            defects=[
                {"rule": R1, "line": 10, "text": "x"},
                {"rule": R1, "line": 20, "text": "y"},
                {"rule": R2, "line": 30, "text": "z"},
            ],
            parents=["h1"],
        ),
        doc(
            "c1",
            "clean-control",
            controls=[
                {"rule": R1, "line": 10, "text": "x"},
                {"rule": R2, "line": 30, "text": "z"},
            ],
            parents=["h1"],
        ),
    ]
}
RESULTS = {
    "schema": 1,
    "engine": {"name": "fake"},
    "corpus": {"manifest_sha256": runner.manifest_digest(MANIFEST)},
    "documents": {
        "h1": result(200, finding(R1, 4), finding(R1, 9)),
        # R1 fires at 11 (within slack of 10) but not at 20; R2 misses 30.
        "s1": result(100, finding(R1, 11), finding(R2, 5)),
        # R1 fires on its control at 10; R2 stays quiet.
        "c1": result(100, finding(R1, 10)),
    },
}


# 2 of 3 seeded defects hit by hand: R1 line 10 yes, line 20 no, R2 line 30 no.
def test_recall_counts_hits_within_line_slack():
    card = scoring.score(MANIFEST, RESULTS, resamples=50)
    recall = card["rules"][R1]["recall"]
    assert (recall["hits"], recall["n"]) == (1, 2)
    assert card["rules"][R2]["recall"]["hits"] == 0


def test_control_false_positive_rate_is_per_rule():
    card = scoring.score(MANIFEST, RESULTS, resamples=50)
    assert card["rules"][R1]["control_fpr"]["rate"] == 1.0
    assert card["rules"][R2]["control_fpr"]["rate"] == 0.0


def test_human_baseline_is_findings_per_100_words():
    card = scoring.score(MANIFEST, RESULTS, resamples=50)
    assert card["rules"][R1]["human_per_100_words"] == 1.0  # 2 findings in 200 words


def test_scores_are_deterministic():
    a = scoring.score(MANIFEST, RESULTS, resamples=50)
    assert a == scoring.score(MANIFEST, RESULTS, resamples=50)


def test_split_follows_the_root_parent():
    parent = doc("h-root", "human")
    child = doc("s", "seeded-defect", parents=["h-root"])
    twin = {"id": "t", "origin": "model-twin", "source": {"outline_of": "h-root"}}
    assert splits.split_of(child) == splits.split_of(parent) == splits.split_of(twin)


def test_holdout_share_is_near_the_target():
    share = sum(
        splits.split_of(doc(f"d{i}", "human")) == "holdout" for i in range(2000)
    )
    assert 0.15 < share / 2000 < 0.25


def test_split_filter_keeps_only_that_side():
    mine = splits.split_of(MANIFEST["documents"][0])
    other = "tuning" if mine == "holdout" else "holdout"
    assert scoring.score(MANIFEST, RESULTS, split=other, resamples=10)["rules"] == {}


def test_wilson_interval_matches_a_hand_computed_value():
    low, high = metrics.wilson(8, 10)
    assert low == pytest.approx(0.4902, abs=1e-3)
    assert high == pytest.approx(0.9433, abs=1e-3)


def test_wilson_of_nothing_is_undefined():
    assert metrics.wilson(0, 0) is None


def test_percentile_interpolates():
    assert metrics.percentile([0.0, 10.0], 0.25) == 2.5


def test_bootstrap_of_a_constant_has_a_zero_width_interval():
    out = metrics.cluster_bootstrap([(1, 1)] * 5, scoring._ratio, resamples=50)
    assert (out["point"], out["low"], out["high"]) == (1.0, 1.0, 1.0)


def test_ab_flags_a_recall_loss_and_ignores_identical_arms():
    worse = json.loads(json.dumps(RESULTS))
    worse["documents"]["s1"]["findings"] = []
    same = ab.ab(MANIFEST, RESULTS, RESULTS, resamples=50)
    assert ab.regressions(same) == []
    report = ab.ab(MANIFEST, RESULTS, worse, resamples=50)
    assert report["rules"][R1]["recall"]["point"] == -0.5
    assert f"{R1}:recall" in ab.regressions(report)


def _llm_card():
    manifest = {
        "documents": [
            doc("h1", "human"),
            doc("l1", "stated-llm"),
            doc("t1", "model-twin", parents=["h1"]),
        ]
    }
    results = {
        "schema": 1,
        "engine": {"name": "fake"},
        "corpus": {"manifest_sha256": runner.manifest_digest(manifest)},
        "documents": {
            "h1": result(200, finding(R1, 4)),
            "l1": result(100, finding(R1, 1), finding(R1, 2), finding(R1, 3)),
            "t1": result(100, finding(R1, 7), finding(R2, 8)),
        },
    }
    return scoring.score(manifest, results, resamples=10)


def test_llm_rate_pools_stated_llm_and_model_twin_text():
    # R1: 3 findings on the stated-LLM answer and 1 on the twin, in 200 words.
    entry = _llm_card()["rules"][R1]
    assert entry["llm_findings"] == 4
    assert entry["llm_per_100_words"] == 2.0
    assert entry["human_per_100_words"] == 0.5  # the human count is untouched


def test_llm_rate_lists_a_rule_that_fires_only_on_llm_text():
    entry = _llm_card()["rules"][R2]
    assert (entry["llm_findings"], entry["llm_per_100_words"]) == (1, 0.5)
    assert entry["human_findings"] == 0


def test_llm_rate_is_undefined_without_llm_words():
    card = scoring.score(MANIFEST, RESULTS, resamples=10)
    assert card["rules"][R1]["llm_findings"] == 0
    assert card["rules"][R1]["llm_per_100_words"] is None
