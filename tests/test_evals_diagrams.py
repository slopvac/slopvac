"""Tests for the LLM-written diagram eval set and its scorer."""

from __future__ import annotations

import json
import shlex
import shutil
import subprocess
import sys
import textwrap
from pathlib import Path

import pytest

from evals import diagrams

REPO = Path(__file__).resolve().parent.parent
FIXTURE = REPO / "tests/fixtures/diagram_eval_output.json"


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


def _finding(rule, matched, line=1):
    return {"rule": rule, "line": line, "matched": matched}


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
            _finding(RUN_ON, "a long one here", 3),
            _finding(RUN_ON, "unlabelled", 4),
        ],
        "b": [_finding(RESTATES, "any paragraph")],
        "c": [_finding(RESTATES, "noise", 2)],
    }
    counts = diagrams.score(labels, findings)
    run_on = counts[RUN_ON]
    # Worked by hand: 2 labels, 1 matched; 2 findings, 1 of them unpaired.
    assert (run_on["labels"], run_on["findings"], run_on["matched"]) == (2, 2, 1)
    assert run_on["recall"] == 0.5
    assert run_on["precision"] == 0.5
    assert run_on["misses"] == [{"sample": "a", "text": "long two"}]
    restates = counts[RESTATES]
    assert (restates["labels"], restates["findings"], restates["matched"]) == (1, 2, 1)
    assert restates["false_positives"] == [
        {"sample": "c", "line": 2, "matched": "noise"}
    ]


def test_one_finding_credits_only_one_of_two_labels():
    gloss = "diagram.label-title-plus-gloss"
    labels = {
        "a": {
            "labels": [
                {"rule": gloss, "text": "Clean"},
                {"rule": gloss, "text": "Validate"},
            ]
        }
    }
    findings = {"a": [_finding(gloss, "Clean and Validate in one label")]}
    row = diagrams.score(labels, findings)[gloss]
    # One finding can satisfy one label: 1 of 2 found, and the finding is correct.
    assert (row["matched"], row["recall"], row["precision"]) == (1, 0.5, 1.0)
    assert len(row["misses"]) == 1


def test_repeated_findings_for_one_label_leave_the_extras_as_false_positives():
    labels = {"a": {"labels": [{"rule": RUN_ON, "text": "long"}]}}
    findings = {"a": [_finding(RUN_ON, "long x", 1), _finding(RUN_ON, "long y", 2)]}
    row = diagrams.score(labels, findings)[RUN_ON]
    assert (row["matched"], row["recall"], row["precision"]) == (1, 1.0, 0.5)
    assert [f["line"] for f in row["false_positives"]] == [2]


def test_empty_restatement_label_accepts_one_finding_not_many():
    labels = {"a": {"labels": [{"rule": RESTATES, "text": ""}]}}
    findings = {"a": [_finding(RESTATES, "p1", 1), _finding(RESTATES, "p2", 5)]}
    row = diagrams.score(labels, findings)[RESTATES]
    assert (row["matched"], row["precision"]) == (1, 0.5)


def test_assignment_finds_the_full_pairing_whatever_the_label_order():
    # The first label could take either finding; only one choice leaves room for the second.
    labels = {
        "a": {
            "labels": [{"rule": RUN_ON, "text": "x"}, {"rule": RUN_ON, "text": "x y"}]
        }
    }
    findings = {"a": [_finding(RUN_ON, "x y", 1), _finding(RUN_ON, "x", 2)]}
    assert diagrams.score(labels, findings)[RUN_ON]["matched"] == 2


def test_score_reports_none_when_a_rule_has_no_labels_or_findings():
    counts = diagrams.score({"a": {"labels": []}}, {})
    assert counts["diagram.note-long"]["recall"] is None
    assert counts["diagram.note-long"]["precision"] is None


# ---- the command line: main() with an engine, and with stored findings ----

ECHO_LABELS_ENGINE = textwrap.dedent(
    """\
    import json, pathlib, sys
    labels = json.load(open(sys.argv[1]))
    out = []
    for path in sys.argv[2:]:
        sid = pathlib.Path(path).stem
        found = [
            {"rule": l["rule"], "line": 1, "matched": l["text"] or "paragraph"}
            for l in labels[sid]["labels"]
        ]
        out.append({"file": path, "findings": found})
    print(json.dumps(out))
    """
)


def _write_engine(tmp_path: Path, body: str) -> str:
    # The directory name has a space, so the command only works if it is quoted.
    folder = tmp_path / "fake engine"
    folder.mkdir()
    script = folder / "engine.py"
    script.write_text(body)
    return shlex.join([sys.executable, str(script), str(diagrams.LABELS)])


def test_main_runs_an_engine_on_the_staged_files_and_scores_it(tmp_path, capsys):
    out = tmp_path / "result.json"
    # An engine that reports exactly the labels scores 1.0 everywhere it has labels.
    code = diagrams.main(
        ["--engine", _write_engine(tmp_path, ECHO_LABELS_ENGINE), "--out", str(out)]
    )
    assert code == 0
    assert "53 samples" in capsys.readouterr().out
    rules = json.loads(out.read_text())["rules"]
    for rule in diagrams.RULES:
        assert rules[rule]["recall"] == 1.0
        assert rules[rule]["precision"] == 1.0
        assert (
            rules[rule]["labels"]
            == rules[rule]["findings"]
            == rules[rule]["matched"]
            > 0
        )


def test_main_exits_2_when_the_engine_fails(tmp_path, capsys):
    engine = _write_engine(tmp_path, "import sys\nsys.exit(3)\n")
    assert diagrams.main(["--engine", engine]) == 2
    assert "engine exited 3" in capsys.readouterr().err


def test_main_scores_stored_findings_without_an_engine(tmp_path, capsys):
    stored = tmp_path / "findings.json"
    stored.write_text(
        json.dumps(
            [
                {
                    "file": "/anywhere/mermaid-24-runon-seven-words.md",
                    "findings": [
                        _finding(RUN_ON, "Send the request to the remote service")
                    ],
                }
            ]
        )
    )
    out = tmp_path / "result.json"
    assert diagrams.main(["--findings", str(stored), "--out", str(out)]) == 0
    run_on = json.loads(out.read_text())["rules"][RUN_ON]
    # One of the ten run-on labels is found, and the single finding is correct.
    assert (run_on["labels"], run_on["matched"], run_on["findings"]) == (10, 1, 1)
    assert run_on["precision"] == 1.0


# ---- the Rust JSON adapter ----

# Output of crates/slopvac-engine/examples/diagram_eval.rs on one sample of each format.
RECORDED = {
    "mermaid-20-kitchen-sink": ".md",
    "plantuml-06-meta-restates": ".md",
    "drawio-08-mixed": ".drawio",
}


def test_recorded_adapter_output_matches_the_labels_of_each_format():
    labels = json.loads(diagrams.LABELS.read_text())
    recorded = {
        Path(r["file"]).stem: r["findings"] for r in json.loads(FIXTURE.read_text())
    }
    assert set(recorded) == set(RECORDED)
    counts = diagrams.score({k: labels[k] for k in RECORDED}, recorded)
    for row in counts.values():
        assert row["misses"] == [] and row["false_positives"] == []
    assert sum(r["matched"] for r in counts.values()) == 7


@pytest.mark.skipif(
    not shutil.which("cargo")
    or not __import__("os").environ.get("SLOPVAC_EVALS_LIVE_ENGINE"),
    reason="set SLOPVAC_EVALS_LIVE_ENGINE=1 to build and run the Rust adapter",
)
def test_live_adapter_reproduces_the_recorded_output(tmp_path):
    paths = []
    for sid, ext in RECORDED.items():
        dst = tmp_path / f"{sid}{ext}"
        shutil.copy(diagrams.SAMPLES / f"{sid}.corpus", dst)
        paths.append(str(dst))
    live = diagrams.run_engine(diagrams.DEFAULT_ENGINE, paths)
    for r in live:
        r["file"] = Path(r["file"]).name
    assert live == json.loads(FIXTURE.read_text())


# ---- the justfile recipe ----


@pytest.mark.skipif(
    not shutil.which("just") or not shutil.which("uv"), reason="needs just and uv"
)
def test_just_recipe_keeps_a_spaced_argument_whole(tmp_path):
    stored = tmp_path / "stored findings.json"
    stored.write_text("[]")
    proc = subprocess.run(
        ["just", "eval-diagrams", "--findings", str(stored)],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=False,
    )
    assert proc.returncode == 0, proc.stderr
    assert "53 samples" in proc.stdout
