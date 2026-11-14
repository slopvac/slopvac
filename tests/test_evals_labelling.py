"""Offline tests for the labelling workflow. No model is called."""

from __future__ import annotations

import json

import pytest

from evals import __main__ as cli
from evals import labelling as lab

R = "cat.rule"
DOC_TEXT = "\n".join(f"line {n}" for n in range(1, 11))


def manifest():
    return {
        "documents": [
            {
                "id": "h1",
                "origin": "human",
                "genre": "readmes",
                "source": {},
                "committed": False,
            },
            {
                "id": "h2",
                "origin": "human",
                "genre": "adrs",
                "source": {},
                "committed": False,
            },
            {
                "id": "s1",
                "origin": "seeded-defect",
                "genre": "adrs",
                "committed": False,
                "source": {
                    "parents": ["h2"],
                    "defects": [{"rule": R, "line": 1, "text": "x"}],
                },
            },
        ]
    }


def results():
    def f(line):
        return {"rule": R, "line": line, "text": "t"}

    return {
        "documents": {
            "h1": {"words": 10, "findings": [f(2), f(3), f(4)]},
            "h2": {"words": 10, "findings": [f(5)]},
            "s1": {"words": 10, "findings": [f(1)]},
        }
    }


def test_sampling_skips_seeded_documents_and_balances_documents():
    items = lab.sample_items(manifest(), results(), per_rule=2)
    assert {i["doc"] for i in items} == {
        "h1",
        "h2",
    }  # one from each before a second from h1
    assert len(items) == 2


def test_sampling_is_deterministic_and_extends_without_repeats():
    first = lab.sample_items(manifest(), results(), per_rule=2)
    assert first == lab.sample_items(manifest(), results(), per_rule=2)
    more = lab.sample_items(
        manifest(), results(), per_rule=2, existing=[i["id"] for i in first]
    )
    assert not {i["id"] for i in first} & {i["id"] for i in more}
    assert len(more) == 2  # the two findings left


ITEM = {"id": "x", "doc": "h1", "genre": "readmes", "rule": R, "line": 5, "text": "t"}


def test_prompt_lists_options_in_swapped_positions():
    p1, l1 = lab.build_prompt(ITEM, "tp-first", DOC_TEXT, {R: "about"})
    _, l2 = lab.build_prompt(ITEM, "fp-first", DOC_TEXT, {R: "about"})
    assert (l1["A"], l1["B"]) == ("tp", "fp")
    assert (l2["A"], l2["B"]) == ("fp", "tp")
    assert ">> 5: line 5" in p1
    assert "   2: line 2" in p1 and " 1: line 1" not in p1 and " 9: line 9" not in p1


@pytest.mark.parametrize(
    ("answer", "expected"),
    [
        ("A because it matches", "tp"),
        ("  (B) no", "fp"),
        ("C", "unsure"),
        ("maybe", "unsure"),
        ("D", "unsure"),
    ],
)
def test_answer_parsing(answer, expected):
    assert lab.parse_answer(answer, {"A": "tp", "B": "fp", "C": "unsure"}) == expected


def test_judge_asks_both_orders_and_resumes(tmp_path, monkeypatch):
    path = tmp_path / "d.txt"
    path.write_text(DOC_TEXT)
    monkeypatch.setattr(lab.runner, "text_path", lambda doc, cache: path)
    kw = {
        "family": "f",
        "model": "m",
        "manifest": manifest(),
        "notes": {},
        "cache": tmp_path,
    }
    rows = lab.judge_items([ITEM], lambda prompt: "A", **kw)
    assert [(r["order"], r["verdict"]) for r in rows] == [
        ("tp-first", "tp"),
        ("fp-first", "fp"),
    ]
    again = lab.judge_items([ITEM], lambda prompt: "A", done={("x", "tp-first")}, **kw)
    assert [r["order"] for r in again] == ["fp-first"]


def j(item, family, order, verdict):
    return {"item": item, "family": family, "order": order, "verdict": verdict}


def both(item, family, first, second):
    return [j(item, family, "tp-first", first), j(item, family, "fp-first", second)]


@pytest.mark.parametrize(
    ("first", "second", "expected"),
    [
        ("tp", "tp", "tp"),
        ("fp", "fp", "fp"),
        ("tp", "fp", "unstable"),
        ("unsure", "unsure", "unstable"),
    ],
)
def test_family_verdict_needs_both_orders_to_agree(first, second, expected):
    assert lab.family_verdict(both("i", "f", first, second)) == expected


def test_family_verdict_with_one_order_is_missing():
    assert lab.family_verdict([j("i", "f", "tp-first", "tp")]) == "missing"


def test_kappa_matches_a_hand_computed_value():
    # 10 pairs: 4 tp/tp, 3 fp/fp, 2 tp/fp, 1 fp/tp.
    # observed 0.7; left tp 6 fp 4; right tp 5 fp 5; expected (6*5+4*5)/100 = 0.5.
    pairs = (
        [("tp", "tp")] * 4 + [("fp", "fp")] * 3 + [("tp", "fp")] * 2 + [("fp", "tp")]
    )
    assert lab.cohen_kappa(pairs) == pytest.approx(0.4)


def test_kappa_of_a_constant_rater_is_undefined():
    assert lab.cohen_kappa([("tp", "tp")] * 3) is None


def items(*ids):
    return [
        {"id": i, "doc": "d", "rule": R, "line": n, "split": "tuning"}
        for n, i in enumerate(ids, 1)
    ]


def test_merge_labels_only_stable_agreement_and_queues_the_rest():
    judgments = [
        *both("a", "x", "tp", "tp"),
        *both("a", "y", "tp", "tp"),  # agree
        *both("b", "x", "tp", "tp"),
        *both("b", "y", "fp", "fp"),  # families disagree
        *both("c", "x", "tp", "tp"),
        *both("c", "y", "tp", "fp"),  # one family unstable
    ]
    out = lab.merge(items("a", "b", "c"), judgments, [])
    assert [(r["item"], r["label"]) for r in out["labels"]] == [("a", "tp")]
    assert sorted(d["item"] for d in out["disputes"]) == ["b", "c"]
    assert out["stats"]["unstable"] == 1


def test_a_human_verdict_beats_the_judges_and_leaves_the_dispute_queue():
    judgments = [*both("b", "x", "tp", "tp"), *both("b", "y", "fp", "fp")]
    human = [
        {"item": "b", "label": "fp", "reviewer": "r", "reason": "why", "date": "d"}
    ]
    out = lab.merge(items("b"), judgments, human)
    assert out["labels"][0]["label"] == "fp" and out["labels"][0]["source"] == "human"
    assert out["disputes"] == []


def test_spot_check_takes_a_fixed_share_of_automatic_labels():
    ids = [f"i{n}" for n in range(20)]
    judgments = [r for i in ids for fam in "xy" for r in both(i, fam, "tp", "tp")]
    out = lab.merge(items(*ids), judgments, [], spot_rate=0.1)
    assert len(out["spot_check"]) == 2
    assert (
        out["spot_check"]
        == lab.merge(items(*ids), judgments, [], spot_rate=0.1)["spot_check"]
    )


def test_revisions_record_each_change_once_and_keep_the_reason():
    labels = [
        {"item": "a", "label": "tp", "source": "judges"},
        {"item": "b", "label": "fp", "source": "human"},
        {"item": "c", "label": "tp", "source": "judges"},
    ]
    human = [
        {"item": "b", "label": "fp", "reviewer": "r", "reason": "quote", "date": "d"}
    ]
    revs = lab.revisions_for_merge({"b": "tp", "c": "tp"}, labels, "2026-11-14", human)
    assert [(r["item"], r["from"], r["to"]) for r in revs] == [
        ("a", None, "tp"),
        ("b", "tp", "fp"),
    ]
    assert revs[1]["reason"] == "quote"


def test_underlabelled_lists_rules_below_the_minimum():
    labels = [{"rule": "a", "label": "tp"}] * 3 + [{"rule": "b", "label": "fp"}]
    assert lab.underlabelled(labels, ["a", "b", "c"], minimum=3) == {"b": 0, "c": 0}


def test_human_verdict_needs_a_reason_and_a_real_label():
    with pytest.raises(ValueError, match="reason"):
        lab.record_human(item="a", label="tp", reviewer="r", reason=" ", date="d")
    with pytest.raises(ValueError, match="tp or fp"):
        lab.record_human(item="a", label="unsure", reviewer="r", reason="x", date="d")


def test_precision_uses_labels_and_ignores_unlabelled_findings(tmp_path):
    from evals import scoring

    labels = {("h1", R, 2): "tp", ("h1", R, 3): "fp", ("h1", R, 4): "tp"}
    card = scoring.score(manifest(), results(), labels=labels, resamples=10)
    assert card["rules"][R]["precision"]["rate"] == pytest.approx(2 / 3)
    assert card["rules"][R]["precision"]["n"] == 3


def test_cli_roundtrip_sample_merge_resolve(tmp_path, monkeypatch):
    monkeypatch.setattr(cli.lib, "load_manifest", manifest)
    res = tmp_path / "r.json"
    res.write_text(
        json.dumps(
            {
                "schema": 1,
                "corpus": {"manifest_sha256": cli.runner.manifest_digest(manifest())},
                **results(),
            }
        )
    )
    d = str(tmp_path / "labels")
    assert cli.main(["label", "--dir", d, "sample", str(res), "--per-rule", "2"]) == 0
    items_ = lab.read_jsonl(tmp_path / "labels" / "items.jsonl")
    assert len(items_) == 2
    target = items_[0]["id"]
    assert (
        cli.main(
            [
                "label",
                "--dir",
                d,
                "resolve",
                target,
                "tp",
                "--reviewer",
                "me",
                "--reason",
                "clear case",
                "--date",
                "2026-11-14",
            ]
        )
        == 0
    )
    assert cli.main(["label", "--dir", d, "merge", "--date", "2026-11-14"]) == 0
    labels = lab.read_jsonl(tmp_path / "labels" / "labels.jsonl")
    assert [(r["item"], r["label"], r["source"]) for r in labels] == [
        (target, "tp", "human")
    ]
    revs = lab.read_jsonl(tmp_path / "labels" / "revisions.jsonl")
    assert revs[0]["reason"] == "clear case"
    assert len(lab.read_jsonl(tmp_path / "labels" / "disputes.jsonl")) == 1


def test_resolve_rejects_unknown_items(tmp_path):
    assert (
        cli.main(
            [
                "label",
                "--dir",
                str(tmp_path),
                "resolve",
                "nope",
                "tp",
                "--reviewer",
                "r",
                "--reason",
                "x",
            ]
        )
        == 2
    )
