"""The golden suite is complete, pinned, replayable and runnable against a binary."""

from __future__ import annotations

import importlib.util
import json
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


def test_format_fixtures_cover_mdx_html_rst_and_two_comment_languages():
    formats = {i["language"]: i for i in INPUTS if i["tier"] == "formats"}
    assert set(formats) == {"mdx", "html", "rst", "python", "javascript"}
    assert {formats[k]["surface"] for k in ("python", "javascript")} == {
        "code-comments"
    }
    assert {formats[k]["surface"] for k in ("mdx", "html", "rst")} == {"prose"}


CORPUS = [i for i in INPUTS if i["tier"] == "corpus"]


def test_corpus_tier_is_every_third_small_human_markdown_document_by_id():
    manifest = json.loads((ROOT / "evals" / "manifest.json").read_text("utf-8"))
    small = sorted(
        d["id"]
        for d in manifest["documents"]
        if d["origin"] == "human"
        and d["format"] == "markdown"
        and d["committed"]
        and d["bytes"] <= 4096
    )
    assert [i["id"] for i in CORPUS] == [f"corpus-{d}" for d in small[::3]]
    assert 10 <= len(CORPUS) <= 20


def test_corpus_inputs_keep_the_sha256_that_the_eval_manifest_pins():
    manifest = json.loads((ROOT / "evals" / "manifest.json").read_text("utf-8"))
    pinned = {f"corpus-{d['id']}": d["sha256"] for d in manifest["documents"]}
    assert {i["id"]: i["sha256"] for i in CORPUS} == {
        i["id"]: pinned[i["id"]] for i in CORPUS
    }


def test_corpus_documents_are_staged_as_markdown(captured):
    staged = {f for c in captured("parity", "3x") for f in c["files"]}
    assert {f"{i['id']}.md" for i in CORPUS} <= staged


@pytest.mark.parametrize("profile", golden.PROFILES)
def test_every_input_has_a_golden_file(profile):
    missing = [i for i in IDS if not golden.golden_path(profile, i).is_file()]
    assert missing == []


@pytest.mark.parametrize("profile", golden.PROFILES)
def test_every_run_group_has_a_run_record(profile):
    groups = {golden.group_key(i) for i in INPUTS}
    assert set(json.loads(golden.runs_path(profile).read_text("utf-8"))) == groups


def test_no_golden_file_lacks_an_input():
    for profile in golden.PROFILES:
        stems = {p.stem for p in (golden.GOLDEN / "parity" / profile).glob("*.json")}
        assert stems == set(IDS)


def test_pin_matches_the_reference_in_use():
    assert golden.verify_pin() == []


def test_pin_records_no_commit():
    recorded = tomllib.loads(golden.PIN_TOML.read_text("utf-8"))
    assert set(recorded) == {"slopvac_version", "python_src_sha256", "vale"}


def test_verify_pin_runs_no_subprocess(monkeypatch):
    def refuse(*args, **kwargs):
        raise AssertionError("a subprocess ran")

    recorded = tomllib.loads(golden.PIN_TOML.read_text("utf-8"))
    monkeypatch.setattr(golden, "vale_version", lambda: recorded["vale"])
    monkeypatch.setattr(golden.subprocess, "run", refuse)
    assert golden.verify_pin() == []


def _tree(root, files):
    for name, text in files.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    return root


SOURCES = {"a.py": "x = 1\n", "rules/r.yml": "k: v\n", "rules/a-b.yml": "k: w\n"}


def test_source_hash_ignores_creation_order(tmp_path):
    forward = _tree(tmp_path / "f", SOURCES)
    backward = _tree(tmp_path / "b", dict(reversed(SOURCES.items())))
    assert golden.reference_tree_hash(forward) == golden.reference_tree_hash(backward)


def test_source_hash_ignores_files_outside_the_reference_set(tmp_path):
    base = golden.reference_tree_hash(_tree(tmp_path / "a", SOURCES))
    extra = _tree(
        tmp_path / "b",
        {**SOURCES, "notes.md": "n", "__pycache__/a.py": "junk", ".git": "gitdir"},
    )
    assert golden.reference_tree_hash(extra) == base


@pytest.mark.parametrize(
    "change",
    [
        {"a.py": "x = 2\n"},
        {"rules/r.yml": "k: other\n"},
        {"new.json": "{}"},
    ],
    ids=["source-edit", "rule-data-edit", "added-file"],
)
def test_source_hash_changes_with_a_reference_file(tmp_path, change):
    base = golden.reference_tree_hash(_tree(tmp_path / "a", SOURCES))
    changed = golden.reference_tree_hash(_tree(tmp_path / "b", {**SOURCES, **change}))
    assert changed != base


def test_source_hash_changes_when_a_file_moves(tmp_path):
    base = golden.reference_tree_hash(_tree(tmp_path / "a", {"a.py": "x"}))
    moved = golden.reference_tree_hash(_tree(tmp_path / "b", {"sub/a.py": "x"}))
    assert moved != base


def test_verify_pin_names_a_source_hash_mismatch(monkeypatch):
    real = golden.current_pin
    monkeypatch.setattr(
        golden, "current_pin", lambda src: {**real(src), "python_src_sha256": "0" * 64}
    )
    assert any(p.startswith("python_src_sha256") for p in golden.verify_pin())


def test_verify_pin_names_a_vale_mismatch(monkeypatch):
    monkeypatch.setattr(golden, "vale_version", lambda: "0.0.1")
    assert any(p.startswith("vale") for p in golden.verify_pin())


def test_record_refuses_a_checkout_that_does_not_match_the_pin(monkeypatch, capsys):
    monkeypatch.setattr(
        golden, "verify_pin", lambda: ["vale: pinned '3.24.0', checkout has '9'"]
    )
    ran = []
    monkeypatch.setattr(golden, "run_binary", lambda *a, **k: ran.append(a))
    assert golden.main(["record", "--reference", REFERENCE]) == 2
    assert ran == []
    assert "does not match pin.toml" in capsys.readouterr().err


# --- how a binary is run ---------------------------------------------------

FAKE = (
    "import json, os, sys\n"
    "json.dump({'argv': sys.argv[1:], 'files': sorted(os.listdir('.')),\n"
    "           'config': open(sys.argv[sys.argv.index('--config') + 1]).read()},\n"
    "          open(os.environ['GOLDEN_CAPTURE'] + '/' + str(len(os.listdir(os.environ['GOLDEN_CAPTURE']))) + '.json', 'w'))\n"
    "print(json.dumps({'schema_version': 3, 'summary': {}, 'documents': []}))\n"
)


@pytest.fixture
def captured(tmp_path, monkeypatch):
    shim = tmp_path / "fake.py"
    shim.write_text(FAKE, encoding="utf-8")
    out = tmp_path / "captures"
    out.mkdir()
    monkeypatch.setenv("GOLDEN_CAPTURE", str(out))

    def run(mode, flavor, profile="normal"):
        for old in out.iterdir():
            old.unlink()
        golden.run_binary(f"{sys.executable} {shim}", mode, profile, INPUTS, flavor)
        return [json.loads(p.read_text("utf-8")) for p in sorted(out.iterdir())]

    return run


@pytest.mark.parametrize("mode", ["parity", "preserve"])
def test_a_3x_candidate_runs_the_legacy_style_in_parity_and_preserve(captured, mode):
    for call in captured(mode, "3x"):
        assert 'styles = ["core:legacy-normal"]' in call["config"]
        assert 'profile = "normal"' in call["config"]
        assert "--profile" not in call["argv"]


def test_a_3x_candidate_runs_the_default_styles_in_policy_mode(captured):
    for call in captured("policy", "3x"):
        assert "styles" not in call["config"]
        assert 'profile = "normal"' in call["config"]
        assert "--profile" not in call["argv"]


def test_a_2x_reference_gets_a_profile_flag_and_an_empty_config(captured):
    for call in captured("parity", "2x", "strict"):
        assert call["config"] == ""
        assert call["argv"][call["argv"].index("--profile") + 1] == "strict"


def test_each_surface_runs_once_and_code_comments_are_selected_explicitly(captured):
    modes = sorted(
        c["argv"][c["argv"].index("--mode") + 1] for c in captured("parity", "3x")
    )
    assert modes == [
        "code-comments",
        "code-comments",
        "prose",
        "prose",
        "prose",
        "prose",
    ]


def test_staged_files_keep_their_real_extension(captured):
    staged = {f for c in captured("parity", "3x") for f in c["files"]}
    for name in (
        "format-mdx.mdx",
        "format-html.html",
        "format-rst.rst",
        "format-python-comments.py",
        "format-javascript-comments.js",
    ):
        assert name in staged


# --- replay and runner -----------------------------------------------------


@pytest.mark.parametrize("mode", ["parity"])
def test_the_python_reference_matches_its_golden(mode):
    assert golden.compare(mode, REFERENCE, flavor="2x") == []


def test_diff_of_a_binary_with_itself_is_empty():
    assert (
        golden.main(["diff", "--a", REFERENCE, "--b", REFERENCE, "--flavor", "2x"]) == 0
    )


def _shim(tmp_path, body):
    shim = tmp_path / "shim.py"
    shim.write_text(
        "import json, subprocess, sys\n"
        f"p = subprocess.run([{REFERENCE!r}, *sys.argv[1:]], capture_output=True, text=True)\n"
        "r = json.loads(p.stdout)\n" + body,
        encoding="utf-8",
    )
    return f"{sys.executable} {shim}"


def test_diff_reports_a_changed_severity(tmp_path, capsys):
    shim = _shim(
        tmp_path,
        "f = r['documents'][0]['findings'][0]\n"
        "f['severity'] = 'suggestion' if f['severity'] == 'error' else 'error'\n"
        "print(json.dumps(r))\n"
        "sys.exit(p.returncode)\n",
    )
    code = golden.main(["diff", "--a", REFERENCE, "--b", shim, "--flavor", "2x"])
    assert code == 1
    assert "differs in" in capsys.readouterr().out


def test_check_fails_for_a_candidate_with_a_different_gate(tmp_path, capsys):
    shim = _shim(
        tmp_path,
        "for d in r['documents']:\n    d['passed'] = not d['passed']\n"
        "print(json.dumps(r))\n",
    )
    assert golden.main(["check", "--candidate", shim, "--flavor", "2x"]) == 1
    assert "differs in" in capsys.readouterr().err


def test_check_fails_when_only_the_run_summary_differs(tmp_path, capsys):
    shim = _shim(
        tmp_path,
        "r['summary']['score'] = r['summary']['score'] - 1\n"
        "print(json.dumps(r))\n"
        "sys.exit(p.returncode)\n",
    )
    assert golden.main(["check", "--candidate", shim, "--flavor", "2x"]) == 1
    err = capsys.readouterr().err
    assert "summary differs in ['score']" in err
    assert "differs in ['" not in err.replace("summary differs in", "")


def test_check_fails_when_only_the_process_exit_status_differs(tmp_path, capsys):
    shim = _shim(
        tmp_path,
        "print(json.dumps(r))\nsys.exit(0 if p.returncode == 1 else p.returncode)\n",
    )
    assert golden.main(["check", "--candidate", shim, "--flavor", "2x"]) == 1
    assert "exit status 0, expected 1" in capsys.readouterr().err


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
    projected = golden.project_document(_doc_2x(), SOURCE, 1)
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


def test_a_document_with_an_unchecked_note_exits_2_whatever_its_gate():
    assert golden.exit_for({"unchecked": ["x"], "gate": {"passed": False}}) == 2
    assert golden.exit_for({"unchecked": [], "gate": {"passed": False}}) == 1
    assert golden.exit_for({"unchecked": [], "gate": {"passed": True}}) == 0


# --- policy deltas ---------------------------------------------------------


def _finding(rule_id, line, column, **kw):
    return {
        "rule_id": rule_id,
        "line": line,
        "column": column,
        "end_line": line,
        "end_column": column + 4,
        "severity": "warning",
        **kw,
    }


def _base(passed=True):
    return {
        "exit_code": 0 if passed else 1,
        "document": {
            "score": 90.0,
            "unchecked": [],
            "gate": {"passed": passed, "reasons": [] if passed else ["x"]},
            "findings": [
                _finding("core:a.b", 3, 1),
                _finding("core:a.c", 5, 1, severity="error"),
                _finding("core:ste-verbs.complex-tense", 5, 10),
                _finding("core:ste-verbs.complex-tense", 5, 30),
            ],
        },
    }


def _delta(**kw):
    return {
        "input": "x",
        "profile": "normal",
        "rule_id": "core:a.b",
        "line": 3,
        "column": 1,
        "kind": "removed",
        **kw,
    }


def _rules(result):
    return [
        (f["rule_id"], f["line"], f["column"]) for f in result["document"]["findings"]
    ]


def test_removed_delta_drops_the_finding():
    out = golden.apply_deltas(_base(), "normal", "x", [_delta()])
    assert ("core:a.b", 3, 1) not in _rules(out)


def test_changed_delta_patches_the_finding_and_the_document():
    out = golden.apply_deltas(
        _base(),
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
    base = _base()
    assert golden.apply_deltas(base, "normal", "y", [_delta()]) == base


def test_delta_that_matches_no_finding_is_an_error():
    with pytest.raises(ValueError, match="one finding"):
        golden.apply_deltas(_base(), "normal", "x", [_delta(line=99)])


def test_delta_without_a_column_is_an_error():
    d = _delta()
    del d["column"]
    with pytest.raises(ValueError, match="needs a column"):
        golden.apply_deltas(_base(), "normal", "x", [d])


def test_delta_picks_one_of_two_matches_of_one_rule_on_one_line_by_column():
    out = golden.apply_deltas(
        _base(),
        "normal",
        "x",
        [_delta(rule_id="core:ste-verbs.complex-tense", line=5, column=30)],
    )
    assert _rules(out).count(("core:ste-verbs.complex-tense", 5, 10)) == 1
    assert ("core:ste-verbs.complex-tense", 5, 30) not in _rules(out)


def test_a_delta_that_fails_the_gate_changes_the_expected_exit_status():
    gate = {"passed": False, "reasons": ["1 error(s), limit 0"]}
    out = golden.apply_deltas(
        _base(passed=True), "normal", "x", [_delta(document={"gate": gate})]
    )
    assert (out["document"]["gate"]["passed"], out["exit_code"]) == (False, 1)


def test_a_delta_that_passes_the_gate_changes_the_expected_exit_status():
    gate = {"passed": True, "reasons": []}
    out = golden.apply_deltas(
        _base(passed=False), "normal", "x", [_delta(document={"gate": gate})]
    )
    assert (out["document"]["gate"]["passed"], out["exit_code"]) == (True, 0)


def test_a_run_delta_patches_the_run_summary_and_exit_status():
    base = {
        "prose:markdown": {"exit_code": 0, "summary": {"score": 90.0, "passed": True}}
    }
    delta = {
        "profile": "normal",
        "run": {
            "group": "prose:markdown",
            "exit_code": 1,
            "summary": {"passed": False},
        },
    }
    out = golden.apply_run_deltas(base, "normal", [delta])
    assert out["prose:markdown"] == {
        "exit_code": 1,
        "summary": {"score": 90.0, "passed": False},
    }


def _mode_deltas(monkeypatch, *deltas):
    monkeypatch.setattr(golden, "load_deltas", lambda: list(deltas))


@pytest.mark.parametrize(
    ("mode", "applies"),
    [("parity", False), ("preserve", False), ("policy", True)],
)
def test_a_delta_without_modes_applies_to_policy_only(monkeypatch, mode, applies):
    _mode_deltas(monkeypatch, _delta())
    assert bool(golden.deltas_for(mode, "3x")) is applies


@pytest.mark.parametrize(
    ("mode", "applies"),
    [("parity", True), ("preserve", False), ("policy", True)],
)
def test_a_delta_applies_to_the_modes_it_lists(monkeypatch, mode, applies):
    _mode_deltas(monkeypatch, _delta(modes=["parity", "policy"]))
    assert bool(golden.deltas_for(mode, "3x")) is applies


@pytest.mark.parametrize("mode", ["parity", "preserve", "policy"])
def test_the_2x_reference_never_gets_a_delta(monkeypatch, mode):
    _mode_deltas(monkeypatch, _delta(modes=["parity", "preserve", "policy"]))
    assert golden.deltas_for(mode, "2x") == []


# --- flavor isolation through compare ---------------------------------------


def _parity_binary(monkeypatch, deltas):
    """A fake binary that reports exactly the parity golden, plus fixed deltas."""
    inputs = golden.load_inputs()
    profile = "normal"
    source = next(
        e["id"]
        for e in inputs
        if json.loads(golden.golden_path(profile, e["id"]).read_text("utf-8"))[
            "document"
        ]["findings"]
    )
    parity = {
        e["id"]: json.loads(golden.golden_path(profile, e["id"]).read_text("utf-8"))
        for e in inputs
    }
    runs = json.loads(golden.runs_path(profile).read_text("utf-8"))
    extra = json.loads(json.dumps(parity[source]["document"]["findings"][0]))
    extra["column"] += 1000
    delta = {
        "input": source,
        "profile": profile,
        "kind": "added",
        "finding": extra,
        "reason": "r",
        "approved_by": "a",
        "run": {"group": next(iter(runs)), "summary": {"score": -1.0}},
        **deltas,
    }
    monkeypatch.setattr(golden, "load_deltas", lambda: [delta])
    monkeypatch.setattr(
        golden,
        "run_binary",
        lambda command, mode, prof, ins, flavor="3x": (parity, runs),
    )


def _compare(mode, flavor):
    return golden.compare(mode, "unused", only_profile="normal", flavor=flavor)


@pytest.mark.parametrize("mode", ["parity", "preserve", "policy"])
def test_the_2x_reference_is_compared_without_deltas_in_every_mode(
    monkeypatch, mode
):
    _parity_binary(monkeypatch, {"modes": ["parity", "preserve", "policy"]})
    assert _compare(mode, "2x") == []


@pytest.mark.parametrize(
    ("modes", "mode", "reported"),
    [
        (None, "policy", True),
        (None, "parity", False),
        (["parity"], "parity", True),
        (["parity"], "policy", False),
        (["parity", "preserve", "policy"], "preserve", True),
    ],
)
def test_a_3x_candidate_is_compared_with_the_deltas_of_its_mode(
    monkeypatch, modes, mode, reported
):
    _parity_binary(monkeypatch, {"modes": modes} if modes else {})
    problems = _compare(mode, "3x")
    # A parity-only candidate differs from the delta-adjusted expectation in the
    # document (the added finding) and in the run summary (the patched score).
    assert bool(problems) is reported
    if reported:
        assert any("differs in" in p and "/normal/" in p for p in problems)
        assert any("summary differs in ['score']" in p for p in problems)


def test_every_recorded_delta_has_a_reason_and_an_approver():
    for delta in golden.load_deltas():
        assert delta["reason"].strip()
        assert delta["approved_by"].strip()
        assert set(delta.get("modes", ["policy"])) <= {"parity", "preserve", "policy"}


def test_vale_is_resolved_on_the_path_a_run_uses(tmp_path, monkeypatch):
    python_dir = tmp_path / "py"
    other_dir = tmp_path / "other"
    for directory, version in ((python_dir, "1.2.3"), (other_dir, "9.9.9")):
        directory.mkdir()
        vale = directory / "vale"
        vale.write_text(f"#!/bin/sh\necho 'vale version {version}'\n", encoding="utf-8")
        vale.chmod(0o755)
    monkeypatch.setattr(sys, "executable", str(python_dir / "python"))
    monkeypatch.setenv("PATH", f"{other_dir}:/usr/bin:/bin")
    assert golden.vale_version() == "1.2.3"
    assert golden.execution_env()["PATH"].split(":")[0] == str(python_dir)


def test_vale_falls_back_to_the_original_path_when_the_python_dir_has_none(
    tmp_path, monkeypatch
):
    python_dir = tmp_path / "py"
    other_dir = tmp_path / "other"
    python_dir.mkdir()
    other_dir.mkdir()
    vale = other_dir / "vale"
    vale.write_text("#!/bin/sh\necho 'vale version 9.9.9'\n", encoding="utf-8")
    vale.chmod(0o755)
    monkeypatch.setattr(sys, "executable", str(python_dir / "python"))
    monkeypatch.setenv("PATH", f"{other_dir}:/usr/bin:/bin")
    assert golden.vale_version() == "9.9.9"
