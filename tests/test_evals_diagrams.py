"""Tests for the LLM-written diagram eval set and its scorer."""

from __future__ import annotations

from evals import diagrams


def test_set_is_intact_and_covers_every_rule_and_format():
    manifest, labels = diagrams.load()
    assert diagrams.verify(manifest, labels) == []
    assert 30 <= len(labels) <= 60
    assert {s["diagram"] for s in manifest["samples"]} == {
        "mermaid",
        "plantuml",
        "drawio",
    }
    labelled = {(rule["rule"]) for e in labels.values() for rule in e["labels"]}
    assert labelled == set(diagrams.RULES)
    assert any(not e["labels"] for e in labels.values()), "needs clean negatives"
    assert manifest["provenance"]["author"]["model"]
    assert manifest["provenance"]["date"]


def test_verify_reports_a_changed_sample(monkeypatch, tmp_path):
    manifest, labels = diagrams.load()
    sid = manifest["samples"][0]["id"]
    (tmp_path / f"{sid}.corpus").write_text("changed")
    monkeypatch.setattr(diagrams, "SAMPLES", tmp_path)
    problems = diagrams.verify({"samples": manifest["samples"][:1]}, {sid: labels[sid]})
    assert problems == [f"{sid}: sha256 differs from the manifest"]


RUN_ON = "diagram.label-run-on"
RESTATES = "diagram.prose-restates-labels"


def test_score_counts_hits_misses_and_false_positives_per_rule():
    labels = {
        "a": {
            "labels": [
                {"rule": RUN_ON, "text": "long one"},
                {"rule": RUN_ON, "text": "long two"},
            ]
        },
        "b": {"labels": [{"rule": RESTATES, "text": ""}]},
        "c": {"labels": []},
    }
    findings = {
        "a": [
            {"rule": RUN_ON, "line": 3, "matched": "a long one here"},
            {"rule": RUN_ON, "line": 4, "matched": "unlabelled"},
        ],
        "b": [{"rule": RESTATES, "line": 1, "matched": "any paragraph"}],
        "c": [{"rule": RESTATES, "line": 2, "matched": "noise"}],
    }
    counts = diagrams.score(labels, findings)
    run_on = counts[RUN_ON]
    # Worked by hand: 2 labels, 1 found; 2 findings, 1 correct.
    assert (
        run_on["labels"],
        run_on["found"],
        run_on["findings"],
        run_on["correct"],
    ) == (2, 1, 2, 1)
    assert run_on["recall"] == 0.5
    assert run_on["precision"] == 0.5
    assert run_on["misses"] == [{"sample": "a", "text": "long two"}]
    restates = counts[RESTATES]
    # An empty label text accepts any finding of the rule in that document.
    assert (restates["found"], restates["findings"], restates["correct"]) == (1, 2, 1)
    assert restates["false_positives"] == [
        {"sample": "c", "line": 2, "matched": "noise"}
    ]


def test_score_reports_none_when_a_rule_has_no_labels_or_findings():
    counts = diagrams.score({"a": {"labels": []}}, {})
    assert counts["diagram.note-long"]["recall"] is None
    assert counts["diagram.note-long"]["precision"] is None
