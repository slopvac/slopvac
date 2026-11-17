"""The ratchet blocks a seeded regression and ignores noise."""

from __future__ import annotations

import copy
import json

import pytest

from evals import __main__ as cli
from evals import gate, runner, scoring, splits

R = "cat.rule"
N = 40  # seeded documents, each with one defect and one control


def build():
    docs, found = [], {}
    for i in range(N):
        sid, cid, hid = f"s{i}", f"c{i}", f"h{i}"
        docs.append({"id": hid, "origin": "human", "genre": "readmes", "source": {}})
        docs.append(
            {
                "id": sid,
                "origin": "seeded-defect",
                "genre": "readmes",
                "source": {
                    "parents": [hid],
                    "defects": [{"rule": R, "line": 5, "text": "x"}],
                },
            }
        )
        docs.append(
            {
                "id": cid,
                "origin": "clean-control",
                "genre": "readmes",
                "source": {
                    "parents": [hid],
                    "controls": [{"rule": R, "line": 5, "text": "x"}],
                },
            }
        )
        found[hid] = {"words": 100, "findings": []}
        found[sid] = {"words": 100, "findings": [{"rule": R, "line": 5}]}
        found[cid] = {"words": 100, "findings": []}
    manifest = {"documents": docs}
    return manifest, {
        "schema": 1,
        "engine": {"name": "t"},
        "corpus": {"manifest_sha256": runner.manifest_digest(manifest)},
        "documents": found,
    }


def cards(manifest, results):
    return {s: scoring.score(manifest, results, split=s) for s in gate.SPLITS}


@pytest.fixture
def base():
    manifest, results = build()
    return manifest, results, gate.make_baseline(cards(manifest, results))


def regress(manifest, results, mutate):
    changed = copy.deepcopy(results)
    mutate(changed["documents"])
    return gate.compare(
        gate.make_baseline(cards(manifest, results)), cards(manifest, changed)
    )


def test_identical_results_do_not_regress(base):
    manifest, results, baseline = base
    assert gate.compare(baseline, cards(manifest, results))["regressions"] == []


def test_a_large_recall_drop_is_a_regression(base):
    manifest, results, _ = base
    out = regress(
        manifest,
        results,
        lambda d: [d[k].update(findings=[]) for k in d if k.startswith("s")],
    )
    assert {r["metric"] for r in out["regressions"]} >= {"recall"}


def test_one_lost_detection_is_inside_the_noise(base):
    manifest, results, _ = base
    tuning = sum(
        splits.split_of(d) == "tuning"
        for d in manifest["documents"]
        if d["id"].startswith("s")
    )
    assert tuning >= gate.MIN_N  # the baseline rate is gated, not merely listed
    out = regress(manifest, results, lambda d: d["s0"].update(findings=[]))
    assert [r for r in out["regressions"] if r["metric"] == "recall"] == []


def test_new_false_positives_on_controls_are_a_regression(base):
    manifest, results, _ = base

    def flag_controls(d):
        for k in d:
            if k.startswith("c"):
                d[k]["findings"] = [{"rule": R, "line": 5}]

    out = regress(manifest, results, flag_controls)
    assert {r["metric"] for r in out["regressions"]} >= {"control_fpr"}


def test_extra_findings_on_human_text_are_a_regression(base):
    manifest, results, _ = base

    def noisy(d):
        for k in d:
            if k.startswith("h"):
                d[k]["findings"] = [{"rule": R, "line": 1}] * 3

    out = regress(manifest, results, noisy)
    assert "human_findings" in {r["metric"] for r in out["regressions"]}


def test_a_few_extra_human_findings_are_inside_the_noise(base):
    manifest, results, _ = base
    out = regress(
        manifest, results, lambda d: d["h0"].update(findings=[{"rule": R, "line": 1}])
    )
    assert [r for r in out["regressions"] if r["metric"] == "human_findings"] == []


def test_rates_on_few_cases_are_listed_not_gated():
    manifest, results = build()
    baseline = gate.make_baseline(cards(manifest, results))
    for split in baseline["splits"].values():
        for entry in split["rules"].values():
            if "recall" in entry:
                entry["recall"] = {"hits": 3, "n": 3}
    changed = copy.deepcopy(results)
    for k in changed["documents"]:
        if k.startswith("s"):
            changed["documents"][k]["findings"] = []
    out = gate.compare(baseline, cards(manifest, changed))
    assert [r for r in out["regressions"] if r["metric"] == "recall"] == []
    assert any(u["metric"] == "recall" for u in out["uncertain"])


def test_precision_with_too_few_adjudicated_positives_is_listed_not_gated():
    # 10 true positives among 40 labelled cases: enough cases for MIN_N, but
    # below the 20 positives the labelling README requires before trusting precision.
    baseline = {
        "splits": {
            "tuning": {
                "human_words": 0,
                "rules": {R: {"human_findings": 0, "precision": {"hits": 10, "n": 40}}},
            }
        }
    }
    card = {
        "origins": {},
        "rules": {R: {"human_findings": 0, "precision": {"tp": 2, "n": 40}}},
    }
    out = gate.compare(baseline, {"tuning": card})
    assert out["regressions"] == []
    assert [(u["rule"], u["metric"]) for u in out["uncertain"]] == [(R, "precision")]


def test_a_rule_that_vanished_is_a_regression(base):
    baseline = base[2]
    empty = {s: {"rules": {}, "origins": {}} for s in gate.SPLITS}
    reasons = {
        r["reason"]
        for r in gate.compare(baseline, empty)["regressions"]
        if "reason" in r
    }
    assert reasons == {"no longer measured"}


def test_baseline_round_trips_through_a_file(base, tmp_path):
    _, _, baseline = base
    path = tmp_path / "b.json"
    gate.dump_baseline(baseline, path)
    assert gate.load_baseline(path) == baseline


def test_unknown_baseline_schema_is_rejected(tmp_path):
    path = tmp_path / "b.json"
    path.write_text(json.dumps({"schema": 9}))
    with pytest.raises(ValueError, match="schema"):
        gate.load_baseline(path)


def run_cli(tmp_path, monkeypatch, manifest, results, changed, *extra):
    monkeypatch.setattr(cli.lib, "load_manifest", lambda: manifest)
    good, bad, baseline = (
        tmp_path / "good.json",
        tmp_path / "bad.json",
        tmp_path / "base.json",
    )
    good.write_text(json.dumps(results))
    bad.write_text(json.dumps(changed))
    assert cli.main(["baseline", str(good), "--out", str(baseline)]) == 0
    return cli.main(["gate", str(bad), "--baseline", str(baseline), *extra])


def broken(results):
    changed = copy.deepcopy(results)
    for k in changed["documents"]:
        if k.startswith("s"):
            changed["documents"][k]["findings"] = []
    return changed


def test_enforce_mode_exits_one_on_a_seeded_regression(tmp_path, monkeypatch, capsys):
    manifest, results = build()
    code = run_cli(
        tmp_path, monkeypatch, manifest, results, broken(results), "--mode", "enforce"
    )
    assert code == 1
    assert f"`{R}`" in capsys.readouterr().out


def test_report_mode_lists_the_regression_and_exits_zero(tmp_path, monkeypatch, capsys):
    manifest, results = build()
    assert run_cli(tmp_path, monkeypatch, manifest, results, broken(results)) == 0
    assert "regression(s)" in capsys.readouterr().out


def test_mode_comes_from_the_environment(tmp_path, monkeypatch):
    manifest, results = build()
    monkeypatch.setenv("SLOPVAC_EVALS_GATE", "enforce")
    assert run_cli(tmp_path, monkeypatch, manifest, results, broken(results)) == 1


def test_clean_results_pass_in_enforce_mode(tmp_path, monkeypatch):
    manifest, results = build()
    assert (
        run_cli(tmp_path, monkeypatch, manifest, results, results, "--mode", "enforce")
        == 0
    )


def test_missing_baseline_is_an_incomplete_run(tmp_path, monkeypatch):
    manifest, results = build()
    monkeypatch.setattr(cli.lib, "load_manifest", lambda: manifest)
    path = tmp_path / "r.json"
    path.write_text(json.dumps(results))
    assert cli.main(["gate", str(path), "--baseline", str(tmp_path / "none.json")]) == 2


def test_baseline_from_another_manifest_is_an_incomplete_run(tmp_path, monkeypatch):
    manifest, results = build()
    monkeypatch.setattr(cli.lib, "load_manifest", lambda: manifest)
    path = tmp_path / "r.json"
    path.write_text(json.dumps(results))
    baseline = tmp_path / "base.json"
    assert cli.main(["baseline", str(path), "--out", str(baseline)]) == 0
    data = json.loads(baseline.read_text())
    data["corpus"] = {"manifest_sha256": "0" * 64}
    baseline.write_text(json.dumps(data))
    assert cli.main(["gate", str(path), "--baseline", str(baseline)]) == 2


def test_committed_baseline_belongs_to_the_current_manifest():
    baseline = gate.load_baseline(gate.BASELINE_DIR / "python.json")
    manifest = cli.lib.load_manifest()
    assert baseline["corpus"]["manifest_sha256"] == runner.manifest_digest(manifest), (
        "the corpus changed: rebuild evals/baselines/python.json with `just eval-baseline`"
    )


OTHER = "cat.other"


def test_a_new_rule_with_false_positives_on_controls_is_a_regression(base):
    manifest, results, baseline = base
    changed = copy.deepcopy(results)
    new_manifest = copy.deepcopy(manifest)
    for d in new_manifest["documents"]:
        if d["id"].startswith("c"):
            d["source"]["controls"].append({"rule": OTHER, "line": 9, "text": "x"})
            changed["documents"][d["id"]]["findings"].append({"rule": OTHER, "line": 9})
    changed["corpus"] = {"manifest_sha256": runner.manifest_digest(new_manifest)}
    out = gate.compare(baseline, cards(new_manifest, changed))
    assert {(r["rule"], r["metric"]) for r in out["regressions"]} == {
        (OTHER, "control_fpr")
    }


def test_a_new_rule_that_floods_human_text_is_a_regression(base):
    manifest, results, baseline = base
    changed = copy.deepcopy(results)
    for key in changed["documents"]:
        if key.startswith("h"):
            changed["documents"][key]["findings"] = [{"rule": OTHER, "line": 2}] * 3
    out = gate.compare(baseline, cards(manifest, changed))
    assert (OTHER, "human_findings") in {
        (r["rule"], r["metric"]) for r in out["regressions"]
    }


def labelled_run(base_data, tmp_path, monkeypatch, bad_share):
    """Baseline: every finding on human text labelled true. Now: `bad_share` of them false."""
    manifest, results = build()
    for d in manifest["documents"]:
        d["source"].pop("defects", None)  # keep recall out of the way
    for key in results["documents"]:
        if key.startswith("h"):
            results["documents"][key]["findings"] = [{"rule": R, "line": 2}]
    results["corpus"] = {"manifest_sha256": runner.manifest_digest(manifest)}
    monkeypatch.setattr(cli.lib, "load_manifest", lambda: manifest)
    hum = [k for k in results["documents"] if k.startswith("h")]
    good = tmp_path / "good.jsonl"
    good.write_text(
        "\n".join(
            json.dumps({"doc": k, "rule": R, "line": 2, "label": "tp"}) for k in hum
        )
    )
    cut = int(len(hum) * bad_share)
    worse = tmp_path / "worse.jsonl"
    worse.write_text(
        "\n".join(
            json.dumps(
                {"doc": k, "rule": R, "line": 2, "label": "fp" if i < cut else "tp"}
            )
            for i, k in enumerate(hum)
        )
    )
    res = tmp_path / "r.json"
    res.write_text(json.dumps(results))
    return res, good, worse


def test_precision_is_gated_through_the_cli(tmp_path, monkeypatch, capsys):
    res, good, worse = labelled_run(None, tmp_path, monkeypatch, bad_share=0.75)
    baseline = tmp_path / "base.json"
    assert (
        cli.main(["baseline", str(res), "--labels", str(good), "--out", str(baseline)])
        == 0
    )
    assert (
        json.loads(baseline.read_text())["splits"]["tuning"]["rules"][R]["precision"][
            "n"
        ]
        >= gate.MIN_N
    )
    code = cli.main(
        [
            "gate",
            str(res),
            "--baseline",
            str(baseline),
            "--labels",
            str(worse),
            "--mode",
            "enforce",
        ]
    )
    assert code == 1
    assert "precision" in capsys.readouterr().out


def test_unchanged_precision_passes_through_the_cli(tmp_path, monkeypatch):
    res, good, _ = labelled_run(None, tmp_path, monkeypatch, bad_share=0.0)
    baseline = tmp_path / "base.json"
    assert (
        cli.main(["baseline", str(res), "--labels", str(good), "--out", str(baseline)])
        == 0
    )
    assert (
        cli.main(
            [
                "gate",
                str(res),
                "--baseline",
                str(baseline),
                "--labels",
                str(good),
                "--mode",
                "enforce",
            ]
        )
        == 0
    )
