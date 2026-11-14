"""Offline tests for the eval harness: statistics, scoring, A/B and the runner."""

from __future__ import annotations

import json
import sys

import pytest

from evals import __main__ as cli
from evals import ab, metrics, runner, scoring, splits


def doc(doc_id, origin, *, defects=None, controls=None, parents=None, words=100):
    source = {}
    if defects is not None:
        source["defects"] = defects
    if controls is not None:
        source["controls"] = controls
    if parents:
        source["parents"] = parents
    return {"id": doc_id, "origin": origin, "source": source, "committed": False}


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
    "engine": {"name": "fake"},
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


FAKE_ENGINE = """\
import json, sys
paths = [a for a in sys.argv[2:] if not a.startswith("--") and a != "json"]
docs = [{"path": p, "words": 7, "findings": [
    {"rule_id": "cat.one", "line": 3, "severity": "warning", "matched_text": "t"}]}
    for p in paths]
print(json.dumps({"slopvac_version": "9.9.9", "documents": docs}))
sys.exit(1)
"""


def test_runner_stores_slim_findings_from_any_engine(tmp_path):
    script = tmp_path / "fake.py"
    script.write_text(FAKE_ENGINE)
    cache = tmp_path / "cache"
    manifest = {"documents": [doc("d1", "human"), doc("d2", "human")]}
    for d in manifest["documents"]:
        target = cache / "docs" / f"{d['id']}.txt"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text("text\n")
    engine = runner.Engine("fake", (sys.executable, str(script)))
    out = runner.run_engine(engine, manifest, cache, batch=1)
    assert sorted(out["documents"]) == ["d1", "d2"]
    assert out["documents"]["d1"] == {
        "words": 7,
        "findings": [
            {"rule": "cat.one", "line": 3, "severity": "warning", "text": "t"}
        ],
    }
    assert out["engine"]["version"] == "9.9.9"


def test_missing_text_names_the_remedy(tmp_path):
    with pytest.raises(runner.MissingText, match="evals.fetch"):
        runner.text_path({"id": "gone", "committed": False}, tmp_path)


def test_pending_twins_are_not_linted():
    assert not runner.default_selector({"origin": "model-twin", "status": "pending"})
    assert runner.default_selector({"origin": "model-twin", "status": "generated"})


def test_engine_exit_two_is_an_error(tmp_path):
    script = tmp_path / "bad.py"
    script.write_text("import sys; sys.exit(2)")
    engine = runner.Engine("bad", (sys.executable, str(script)))
    with pytest.raises(runner.EngineError, match="exited 2"):
        engine.lint([tmp_path / "a.md"])


def test_offline_score_command_reads_stored_results(tmp_path, monkeypatch):
    monkeypatch.setattr(cli.lib, "load_manifest", lambda: MANIFEST)
    results = tmp_path / "r.json"
    results.write_text(json.dumps({"schema": 1, **RESULTS}))
    out = tmp_path / "card.json"
    assert (
        cli.main(["score", str(results), "--resamples", "10", "--out", str(out)]) == 0
    )
    assert json.loads(out.read_text())["rules"][R1]["recall"]["n"] == 2
