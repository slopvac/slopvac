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
        "family": "anthropic",
        "model": "claude-test",
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


def test_an_interrupted_judge_run_keeps_finished_judgments_and_resumes_without_repeats(
    tmp_path, monkeypatch
):
    monkeypatch.setattr(cli.lib, "load_manifest", manifest)
    path = tmp_path / "d.txt"
    path.write_text(DOC_TEXT)
    monkeypatch.setattr(lab.runner, "text_path", lambda doc, cache: path)
    items_ = [{**ITEM, "id": f"i{n}", "line": 5} for n in range(3)]
    d = tmp_path / "labels"
    lab.write_jsonl(d / "items.jsonl", items_)
    asked = []

    def run(provider_calls_before_crash):
        def ask(prompt):
            if len(asked) >= provider_calls_before_crash:
                raise lab.runner.EngineError("provider crashed")
            asked.append(prompt)
            return "A"

        monkeypatch.setattr(cli.labelling, "command_asker", lambda command: ask)
        return cli.main(
            [
                "label",
                "--dir",
                str(d),
                "judge",
                "--family",
                "anthropic",
                "--model",
                "claude-test",
                "--provider-cmd",
                "ignored",
                "--rules-dir",
                str(tmp_path),
            ]
        )

    assert run(4) == 2  # 3 items x 2 orders = 6 calls; the 5th call crashes
    saved = lab.read_jsonl(d / "judgments" / "anthropic.jsonl")
    assert len(saved) == 4
    assert run(99) == 0
    assert len(asked) == 6  # the 4 finished calls were not repeated
    assert len(lab.read_jsonl(d / "judgments" / "anthropic.jsonl")) == 6


# --- provenance, resume keys and the spot-check ------------------------------------


@pytest.mark.parametrize(
    ("model", "family"),
    [
        ("claude-sonnet-4", "anthropic"),
        ("us.anthropic.claude-sonnet-4-v1:0", "anthropic"),
        ("gpt-5", "openai"),
        ("o3-mini", "openai"),
        ("gemini-2.5-pro", "google"),
        ("llama-3-70b", "meta"),
        ("mystery-model", None),
        ("claude-vs-gpt", None),  # two families named: no single identity
    ],
)
def test_model_family_is_read_from_the_model_id(model, family):
    assert lab.model_family(model) == family


@pytest.mark.parametrize("model", ["gpt-5", "mystery-model"])
def test_judge_refuses_a_family_the_model_id_does_not_confirm(
    tmp_path, monkeypatch, model
):
    path = tmp_path / "d.txt"
    path.write_text(DOC_TEXT)
    monkeypatch.setattr(lab.runner, "text_path", lambda doc, cache: path)
    asked = []
    with pytest.raises(lab.runner.EngineError, match="anthropic"):
        lab.judge_items(
            [ITEM],
            lambda prompt: asked.append(prompt) or "A",
            family="anthropic",
            model=model,
            manifest=manifest(),
            notes={},
            cache=tmp_path,
        )
    assert asked == []


def judge_cli(
    d,
    tmp_path,
    monkeypatch,
    asked,
    *,
    model="claude-test",
    family="anthropic",
    answer=lambda prompt: "A",
):
    monkeypatch.setattr(
        cli.labelling,
        "command_asker",
        lambda command: lambda prompt: asked.append(prompt) or answer(prompt),
    )
    return cli.main(
        [
            "label",
            "--dir",
            str(d),
            "judge",
            "--family",
            family,
            "--model",
            model,
            "--provider-cmd",
            "ignored",
            "--rules-dir",
            str(tmp_path),
        ]
    )


@pytest.fixture
def judge_env(tmp_path, monkeypatch):
    monkeypatch.setattr(cli.lib, "load_manifest", manifest)
    path = tmp_path / "d.txt"
    path.write_text(DOC_TEXT)
    monkeypatch.setattr(lab.runner, "text_path", lambda doc, cache: path)
    d = tmp_path / "labels"
    lab.write_jsonl(
        d / "items.jsonl", [{**ITEM, "id": "i0", "line": 5, "split": "tuning"}]
    )
    return d


def test_cli_judge_exits_2_and_writes_nothing_on_a_family_mismatch(
    judge_env, tmp_path, monkeypatch
):
    asked = []
    assert judge_cli(judge_env, tmp_path, monkeypatch, asked, model="gpt-5") == 2
    assert asked == []
    assert not (judge_env / "judgments").exists()


def test_a_judgment_by_another_model_is_not_reused_on_resume(
    judge_env, tmp_path, monkeypatch
):
    asked = []
    assert judge_cli(judge_env, tmp_path, monkeypatch, asked) == 0
    assert len(asked) == 2
    assert judge_cli(judge_env, tmp_path, monkeypatch, asked) == 0
    assert len(asked) == 2  # same model: nothing new to ask
    assert judge_cli(judge_env, tmp_path, monkeypatch, asked, model="claude-other") == 0
    assert len(asked) == 4  # other model: both orders asked again


def test_a_judgment_under_another_prompt_version_is_not_reused_on_resume(
    judge_env, tmp_path, monkeypatch
):
    asked = []
    assert judge_cli(judge_env, tmp_path, monkeypatch, asked) == 0
    monkeypatch.setattr(lab, "PROMPT_VERSION", "2")
    assert judge_cli(judge_env, tmp_path, monkeypatch, asked) == 0
    assert len(asked) == 4


def write_judgments(d, family, model, ids, **extra):
    rows = [
        {
            "item": i,
            "family": family,
            "model": model,
            "order": o,
            "prompt_version": lab.PROMPT_VERSION,
            "verdict": "tp",
            **extra,
        }
        for i in ids
        for o in lab.ORDERS
    ]
    lab.write_jsonl(d / "judgments" / f"{family}.jsonl", rows)


def merge_cli(d):
    return cli.main(["label", "--dir", str(d), "merge", "--date", "2026-11-17"])


def test_merge_ignores_checkpoints_without_model_or_prompt_version(tmp_path):
    d = tmp_path / "labels"
    lab.write_jsonl(d / "items.jsonl", items("a"))
    write_judgments(d, "anthropic", "claude-test", ["a"])
    old = [
        {"item": "a", "family": "openai", "order": o, "verdict": "tp"}
        for o in lab.ORDERS
    ]
    lab.write_jsonl(d / "judgments" / "openai.jsonl", old)
    assert merge_cli(d) == 0
    assert lab.read_jsonl(d / "labels.jsonl") == []
    assert [r["item"] for r in lab.read_jsonl(d / "disputes.jsonl")] == ["a"]


def test_merge_ignores_judgments_from_an_older_prompt_version(tmp_path):
    d = tmp_path / "labels"
    lab.write_jsonl(d / "items.jsonl", items("a"))
    write_judgments(d, "anthropic", "claude-test", ["a"])
    write_judgments(d, "openai", "gpt-5", ["a"], prompt_version="0")
    assert merge_cli(d) == 0
    assert lab.read_jsonl(d / "labels.jsonl") == []


def test_merge_counts_only_the_newest_model_of_a_family(tmp_path):
    d = tmp_path / "labels"
    lab.write_jsonl(d / "items.jsonl", items("a"))
    write_judgments(d, "openai", "gpt-5", ["a"])
    write_judgments(d, "anthropic", "claude-old", ["a"])
    path = d / "judgments" / "anthropic.jsonl"
    old = [{**r, "verdict": "fp"} for r in lab.read_jsonl(path)]
    new = [{**r, "model": "claude-new"} for r in lab.read_jsonl(path)]
    lab.write_jsonl(path, [*old, *new])  # claude-new was appended later
    assert merge_cli(d) == 0
    assert [(r["item"], r["label"]) for r in lab.read_jsonl(d / "labels.jsonl")] == [
        ("a", "tp")
    ]


def test_older_model_judgments_never_label_items_the_newest_model_skipped(
    tmp_path, capsys
):
    d = tmp_path / "labels"
    lab.write_jsonl(d / "items.jsonl", items("a", "b", "c"))
    write_judgments(d, "openai", "gpt-5", ["a", "b", "c"])
    write_judgments(d, "anthropic", "claude-old", ["a", "b", "c"])
    path = d / "judgments" / "anthropic.jsonl"
    old = lab.read_jsonl(path)
    write_judgments(d, "anthropic", "claude-new", ["a"])  # newest covers only "a"
    new = lab.read_jsonl(path)
    lab.write_jsonl(path, [*old, *new])
    assert merge_cli(d) == 0
    assert [r["item"] for r in lab.read_jsonl(d / "labels.jsonl")] == ["a"]
    assert sorted(r["item"] for r in lab.read_jsonl(d / "disputes.jsonl")) == [
        "b",
        "c",
    ]
    # 3 items x 2 orders from claude-old are set aside.
    assert "ignored 6 judgments" in capsys.readouterr().err


def test_merge_exits_2_when_a_recorded_family_contradicts_its_model(tmp_path):
    d = tmp_path / "labels"
    lab.write_jsonl(d / "items.jsonl", items("a"))
    write_judgments(d, "openai", "claude-test", ["a"])
    assert merge_cli(d) == 2
    assert not (d / "labels.jsonl").exists()


def auto_labelled(tmp_path, n=20):
    d = tmp_path / "labels"
    ids = [f"i{k}" for k in range(n)]
    lab.write_jsonl(d / "items.jsonl", items(*ids))
    write_judgments(d, "anthropic", "claude-test", ids)
    write_judgments(d, "openai", "gpt-5", ids)
    return d


def resolve_cli(d, item):
    return cli.main(
        ["label", "--dir", str(d), "resolve", item, "tp"]
        + ["--reviewer", "me", "--reason", "checked", "--date", "2026-11-17"]
    )


def test_merge_refuses_automatic_labels_until_the_spot_check_is_recorded(tmp_path):
    d = auto_labelled(tmp_path)  # 20 automatic labels, 10% -> 2 to re-check
    assert merge_cli(d) == 2
    assert not (d / "labels.jsonl").exists()
    spot = json.loads((d / "spot-check.json").read_text())
    assert len(spot) == 2
    assert resolve_cli(d, spot[0]) == 0
    assert merge_cli(d) == 2  # one of two recorded
    assert resolve_cli(d, spot[1]) == 0
    assert merge_cli(d) == 0
    labels = lab.read_jsonl(d / "labels.jsonl")
    assert len(labels) == 20
    assert {r["item"] for r in labels if r["source"] == "human"} == set(spot)


def test_a_fully_human_label_set_needs_no_spot_check(tmp_path):
    from evals import scoring

    d = tmp_path / "labels"
    lab.write_jsonl(d / "items.jsonl", items("a", "b", "c"))
    assert resolve_cli(d, "a") == 0
    assert resolve_cli(d, "b") == 0
    assert merge_cli(d) == 0
    got = scoring.load_verdicts(d / "labels.jsonl")
    assert got == {("d", R, 1): "tp", ("d", R, 2): "tp"}
    assert json.loads((d / "spot-check.json").read_text()) == []


def test_hand_judged_scorer_rows_score_without_a_spot_check(tmp_path):
    from evals import scoring

    path = tmp_path / "labels.jsonl"
    lab.write_jsonl(
        path,
        [
            {"doc": "h1", "rule": R, "line": 2, "label": "tp"}
            | {"reviewer": "me", "reason": "clear", "date": "2026-11-17"},
            {"doc": "h1", "rule": R, "line": 3, "label": "fp"}
            | {"reviewer": "me", "reason": "fine here", "date": "2026-11-17"},
        ],
    )
    card = scoring.score(
        manifest(), results(), labels=scoring.load_verdicts(path), resamples=10
    )
    assert card["rules"][R]["precision"]["n"] == 2


def test_exact_model_ids_reach_judgments_labels_and_the_export(
    judge_env, tmp_path, monkeypatch
):
    from evals import scoring

    def says_tp(prompt):  # picks the "rule is right" option in either order
        return "A" if "A) The flagged text is a genuine" in prompt else "B"

    ids = {
        "anthropic": "amazon-bedrock/global.anthropic.claude-sonnet-5-5",
        "openai": "openai/gpt-5.1-codex",
    }
    for family, model in ids.items():
        assert (
            judge_cli(
                judge_env,
                tmp_path,
                monkeypatch,
                [],
                model=model,
                family=family,
                answer=says_tp,
            )
            == 0
        )
        rows = lab.read_jsonl(judge_env / "judgments" / f"{family}.jsonl")
        assert {r["model"] for r in rows} == {model}
    assert merge_cli(judge_env) == 0
    (label,) = lab.read_jsonl(judge_env / "labels.jsonl")
    assert label["source"] == "judges"
    assert label["models"] == ids
    # The export stays readable by the scorer.
    assert scoring.load_verdicts(judge_env / "labels.jsonl") == {("h1", R, 5): "tp"}


def test_sample_dry_run_counts_per_rule_and_writes_nothing(
    tmp_path, monkeypatch, capsys
):
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
    d = tmp_path / "labels"
    args = ["label", "--dir", str(d), "sample", str(res), "--per-rule", "2"]
    assert cli.main([*args, "--dry-run"]) == 0
    assert capsys.readouterr().out == f"{R}\t2\ntotal\t2\n"
    assert not d.exists()
