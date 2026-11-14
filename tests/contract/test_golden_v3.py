"""The golden suite is complete, pinned, replayable and runnable against a binary."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tomllib
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
_spec = importlib.util.spec_from_file_location(
    "golden", ROOT / "scripts" / "contract" / "golden.py"
)
golden = importlib.util.module_from_spec(_spec)
sys.modules["golden"] = golden
_spec.loader.exec_module(golden)

REFERENCE = str(Path(sys.executable).parent / "slopvac")
SRC = ROOT / "packages" / "slopvac-lint" / "src" / "slopvac"
INPUTS = golden.load_inputs()
IDS = [i["id"] for i in INPUTS]


# --- inputs, golden files and pin ------------------------------------------


def test_input_ids_are_unique():
    assert len(IDS) == len(set(IDS))


@pytest.mark.parametrize("entry", INPUTS, ids=IDS)
def test_input_matches_its_recorded_hash(entry):
    assert golden.sha256(ROOT / entry["path"]) == entry["sha256"]


def test_inputs_manifest_equals_regenerated_output():
    assert golden.INPUTS_TOML.read_text("utf-8") == golden.render_inputs(
        golden.input_entries()
    )


def test_rule_example_inputs_equal_regenerated_output():
    for doc_id, text in golden.rule_example_documents().items():
        path = golden.RULE_EXAMPLES / f"{doc_id.removeprefix('rule-examples-')}.md"
        assert path.read_text("utf-8") == text


@pytest.mark.parametrize("profile", golden.PROFILES)
def test_every_input_has_a_golden_file(profile):
    missing = [i for i in IDS if not golden.golden_path(profile, i).is_file()]
    assert missing == []


def test_no_golden_file_lacks_an_input():
    for profile in golden.PROFILES:
        stems = {p.stem for p in (golden.GOLDEN / "parity" / profile).glob("*.json")}
        assert stems == set(IDS)


def test_pin_matches_the_reference_in_use():
    recorded = tomllib.loads(golden.PIN_TOML.read_text("utf-8"))
    assert recorded == golden.current_pin(SRC)


# --- replay and runner -----------------------------------------------------


@pytest.mark.parametrize("mode", ["parity", "policy"])
def test_the_python_reference_matches_its_golden(mode):
    assert golden.compare(mode, REFERENCE) == []


def test_diff_of_a_binary_with_itself_is_empty():
    assert golden.main(["diff", "--a", REFERENCE, "--b", REFERENCE]) == 0


def test_diff_reports_a_changed_severity(tmp_path, capsys):
    shim = tmp_path / "shim.py"
    shim.write_text(
        "import json, subprocess, sys\n"
        f"p = subprocess.run([{REFERENCE!r}, *sys.argv[1:]], capture_output=True, text=True)\n"
        "r = json.loads(p.stdout)\n"
        "f = r['documents'][0]['findings'][0]\n"
        "f['severity'] = 'suggestion' if f['severity'] == 'error' else 'error'\n"
        "print(json.dumps(r))\n"
        "sys.exit(p.returncode)\n",
        encoding="utf-8",
    )
    code = golden.main(
        [
            "diff",
            "--a",
            REFERENCE,
            "--b",
            f"{sys.executable} {shim}",
            "--mode",
            "parity",
        ]
    )
    assert code == 1
    assert "differs in" in capsys.readouterr().out


def test_check_fails_for_a_candidate_with_a_different_exit_status(tmp_path, capsys):
    shim = tmp_path / "shim.py"
    shim.write_text(
        "import json, subprocess, sys\n"
        f"p = subprocess.run([{REFERENCE!r}, *sys.argv[1:]], capture_output=True, text=True)\n"
        "r = json.loads(p.stdout)\n"
        "for d in r['documents']:\n"
        "    d['passed'] = not d['passed']\n"
        "print(json.dumps(r))\n",
        encoding="utf-8",
    )
    assert golden.main(["check", "--candidate", f"{sys.executable} {shim}"]) == 1
    assert "differs in" in capsys.readouterr().err


# --- projection ------------------------------------------------------------

SOURCE = "Café ok. The cache is bad.\n".encode()


def _doc_2x(**finding):
    base = {
        "path": "a.md",
        "line": 1,
        "column": 10,
        "end_column": 19,
        "rule_id": "ste-sentences.x",
        "category": "ste-sentences",
        "severity": "warning",
        "message": "m",
        "ai_signal": "none",
        "matched_text": "The cache",
        "replacement": None,
    }
    return {
        "path": "a.md",
        "profile": "normal",
        "words": 6,
        "sentences": 2,
        "paragraphs": 1,
        "total_findings": 1,
        "errors": 0,
        "warnings": 1,
        "suggestions": 0,
        "per_100_words": 1.0,
        "score": 90.0,
        "passed": True,
        "failure_reasons": [],
        "categories": [],
        "unchecked": ["note"],
        "ai_register": {},
        "prose": {},
        "findings": [{**base, **finding}],
    }


def test_2x_finding_gets_a_byte_span_for_code_point_columns():
    doc = _doc_2x()
    projected = golden.project_document(doc, SOURCE, 1)
    # "Café ok. " is 9 code points and 10 bytes, so the match starts at byte 10.
    assert projected["findings"][0]["span"] == {"start": 10, "end": 19}


def test_2x_finding_with_a_masked_match_keeps_line_and_column_only():
    doc = _doc_2x(matched_text="The ***** x", end_column=20)
    finding = golden.project_document(doc, SOURCE, 1)["findings"][0]
    assert (finding["line"], finding["column"]) == (1, 10)
    assert (finding["span"], finding["end_line"], finding["end_column"]) == (
        None,
        None,
        None,
    )


def test_2x_and_v3_reports_project_to_the_same_document():
    two = golden.project_document(_doc_2x(), SOURCE, 1)
    v3 = _doc_2x()
    v3["gate"] = {"passed": True, "reasons": []}
    v3["unchecked"] = [{"code": "note", "level": "info", "message": "note"}]
    f = v3["findings"][0]
    f.update(rule_id="core:ste-sentences.x", end_line=1, span={"start": 10, "end": 19})
    f.pop("replacement")
    assert golden.project_document(v3, SOURCE, 3) == two


def test_multi_edit_fix_has_no_2x_replacement():
    doc = _doc_2x()
    f = doc["findings"][0]
    f.update(rule_id="core:a.b", end_line=1, span={"start": 10, "end": 19})
    f["fix"] = {
        "edits": [
            {"start": 10, "end": 12, "text": "a"},
            {"start": 13, "end": 15, "text": "b"},
        ]
    }
    doc["gate"] = {"passed": True, "reasons": []}
    with pytest.raises(ValueError, match="multi-edit"):
        golden.project_document(doc, SOURCE, 3)


# --- policy deltas ---------------------------------------------------------

BASE = {
    "exit_code": 0,
    "document": {
        "score": 90.0,
        "findings": [
            {
                "rule_id": "core:a.b",
                "line": 3,
                "column": 1,
                "end_line": 3,
                "severity": "warning",
                "end_column": 5,
            },
            {
                "rule_id": "core:a.c",
                "line": 5,
                "column": 1,
                "end_line": 5,
                "severity": "error",
                "end_column": 5,
            },
        ],
    },
}


def _delta(**kw):
    return {
        "input": "x",
        "profile": "normal",
        "rule_id": "core:a.b",
        "line": 3,
        "kind": "removed",
        **kw,
    }


def test_removed_delta_drops_the_finding():
    out = golden.apply_deltas(BASE, "normal", "x", [_delta()])
    assert [f["rule_id"] for f in out["document"]["findings"]] == ["core:a.c"]


def test_changed_delta_patches_the_finding_and_the_document():
    out = golden.apply_deltas(
        BASE,
        "normal",
        "x",
        [
            _delta(
                kind="changed", set={"severity": "suggestion"}, document={"score": 95.0}
            )
        ],
    )
    assert out["document"]["findings"][0]["severity"] == "suggestion"
    assert out["document"]["score"] == 95.0


def test_delta_for_another_input_is_ignored():
    assert golden.apply_deltas(BASE, "normal", "y", [_delta()]) == BASE


def test_delta_that_matches_no_finding_is_an_error():
    with pytest.raises(ValueError, match="one finding"):
        golden.apply_deltas(BASE, "normal", "x", [_delta(line=99)])


def test_every_recorded_delta_has_a_reason_and_an_approver():
    for delta in golden.load_deltas():
        assert delta["reason"].strip()
        assert delta["approved_by"].strip()
