"""The Markdown audit refuses a report whose documents carry no usable gate verdict.

`scripts/ci/markdown_audit_reports.py` holds the check, and the audit step in
`.github/workflows/markdown-audit.yml` calls it on every report. A document without
`gate.passed`, or with a non-boolean, must fail the audit. A valid `false` is allowed in
the diagnostic scans, which may fail, and not in the authored scan.
"""

from __future__ import annotations

import importlib.util
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "markdown-audit.yml"
_spec = importlib.util.spec_from_file_location(
    "markdown_audit_reports", ROOT / "scripts" / "ci" / "markdown_audit_reports.py"
)
assert _spec and _spec.loader
audit = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(audit)


def _report(*gates: object) -> dict:
    return {
        "documents": [
            {"path": f"doc{i}.md", **({} if gate is audit_missing else {"gate": gate})}
            for i, gate in enumerate(gates)
        ]
    }


audit_missing = object()
_PASS = {"passed": True, "reasons": []}


@pytest.mark.parametrize(
    ("gates", "allow_false", "failing"),
    [
        pytest.param([_PASS, _PASS], False, 0, id="valid"),
        pytest.param([_PASS, audit_missing], True, 1, id="gate-key-missing"),
        pytest.param([_PASS, {"reasons": []}], True, 1, id="passed-key-missing"),
        pytest.param([_PASS, {"passed": None}], True, 1, id="null"),
        pytest.param([_PASS, {"passed": "true"}], True, 1, id="string-true"),
        pytest.param([_PASS, {"passed": 1}], True, 1, id="integer-one"),
        pytest.param([_PASS, None], True, 1, id="gate-null"),
        pytest.param([_PASS, {"passed": False}], True, 0, id="false-in-a-diagnostic-scan"),
        pytest.param([_PASS, {"passed": False}], False, 1, id="false-in-the-authored-scan"),
    ],
)
def test_gate_verdicts(gates, allow_false, failing) -> None:
    problems = audit.gate_problems("scan", _report(*gates), allow_false=allow_false)
    assert len(problems) == failing
    assert all(p.startswith("scan: document 1 ") for p in problems)


@pytest.mark.parametrize("report", [None, [], {}, {"documents": None}], ids=str)
def test_a_report_without_a_documents_list_fails(report) -> None:
    assert audit.gate_problems("scan", report, allow_false=True)


def test_the_cargo_cache_is_saved_only_by_a_push_to_main() -> None:
    # The workflow is called from test.yml, where `github.event_name` is the caller's event.
    # Only the push to main may save the cache.
    steps = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))["jobs"]["inventory-and-audit"]["steps"]
    (cache,) = [s for s in steps if str(s.get("uses", "")).startswith("Swatinem/rust-cache@")]
    assert cache["with"]["save-if"] == (
        "${{ github.event_name == 'push' && github.ref == 'refs/heads/main' }}"
    )


_EXPECTED = {"README.md", "docs/a.md", "docs/b.md"}


@pytest.mark.parametrize(
    ("reported", "problems"),
    [
        pytest.param(["README.md", "docs/a.md", "docs/b.md"], 0, id="exact-set"),
        pytest.param(["docs/b.md", "README.md", "docs/a.md"], 0, id="order-is-free"),
        pytest.param([], 1, id="empty-report"),
        pytest.param(["README.md", "docs/a.md"], 1, id="one-omitted"),
        pytest.param(["README.md", "docs/a.md", "docs/b.md", "extra.md"], 1, id="one-extra"),
        pytest.param(["README.md", "docs/a.md", "docs/a.md"], 2, id="duplicate-and-omitted"),
        pytest.param(["README.md", "docs/a.md", "docs/b.md", "docs/b.md"], 1, id="duplicate"),
    ],
)
def test_the_authored_report_holds_exactly_the_authored_documents(reported, problems) -> None:
    found = audit.document_set_problems("authored", reported, _EXPECTED)
    assert len(found) == problems


# ---- the audit step's real script, against a stub linter ---------------------------

from markdown_audit_harness import run_audit  # noqa: E402

HOSTILE_PATH = "::error::pwned\n::add-mask::x"


def _no_workflow_command(text: str) -> None:
    assert "::" not in text
    assert "##[" not in text


def test_a_correct_audit_passes_and_prints_no_workflow_command(tmp_path) -> None:
    done = run_audit(tmp_path, {})
    assert done.returncode == 0, done.stderr
    _no_workflow_command(done.stdout + done.stderr)
    # The hostile file name still reaches the log, in escaped form.
    assert "%3A%3Aerror%3A%3Apwned" in done.stdout


# Each case makes one in-scope print carry the hostile text. The scan whose report holds it
# and the print it reaches are named in the id.
_PRINTS = {
    "summary": {"configured": 'report["summary"]["errors"] = "::error::pwned"'},
    "unchecked": {
        "all-files": (
            'report["documents"][0]["unchecked"] = '
            '[{"level": "warning", "message": "::error::pwned\\n::add-mask::x"}]'
        )
    },
    "configured-failure": {
        "configured": (
            'report["documents"][0]["gate"] = {"passed": False};'
            'report["documents"][0]["findings"] = [{"message": "::error::pwned"}]'
        )
    },
    "linter-output": {
        "configured": "pass",
        "configured-stdout": "::error::pwned\n##[group]x\n",
        "configured-exit": 2,
    },
}


@pytest.mark.parametrize("case", list(_PRINTS.values()), ids=list(_PRINTS))
def test_untrusted_report_text_reaches_the_log_only_escaped(tmp_path, case) -> None:
    done = run_audit(tmp_path, case)
    _no_workflow_command(done.stdout + done.stderr)


@pytest.mark.parametrize(
    "report",
    [
        pytest.param({"documents": [{"path": HOSTILE_PATH}]}, id="gate-missing"),
        pytest.param({"documents": [{"path": HOSTILE_PATH, "gate": {"passed": "x::y"}}]}, id="value"),
        pytest.param({"documents": [{"path": HOSTILE_PATH, "gate": {"passed": False}}]}, id="failed"),
    ],
)
def test_the_helper_messages_carry_no_report_text(report) -> None:
    messages = audit.gate_problems("authored", report, allow_false=False)
    assert messages
    for message in messages:
        _no_workflow_command(message)
        assert "pwned" not in message and "x::y" not in message


def test_the_set_messages_carry_no_path() -> None:
    messages = audit.document_set_problems("authored", [HOSTILE_PATH], {"README.md"})
    assert messages
    for message in messages:
        _no_workflow_command(message)
        assert "pwned" not in message


_MALFORMED = {
    "gate-key-missing": 'del report["documents"][0]["gate"]',
    "passed-null": 'report["documents"][0]["gate"]["passed"] = None',
    "passed-string": 'report["documents"][0]["gate"]["passed"] = "true"',
}


@pytest.mark.parametrize("fault", list(_MALFORMED))
@pytest.mark.parametrize("scan", ["authored", "configured", "all-files"])
def test_a_malformed_report_fails_the_audit_step(tmp_path, scan, fault) -> None:
    done = run_audit(tmp_path, {scan: _MALFORMED[fault]})
    assert done.returncode != 0
    assert f"{scan}: " in done.stderr
    _no_workflow_command(done.stderr)


def test_an_empty_authored_report_fails_the_audit_step(tmp_path) -> None:
    done = run_audit(tmp_path, {"authored": 'report["documents"] = []'})
    assert done.returncode != 0
    assert "authored: the report omits" in done.stderr


_CONFIGURED_FAULTS = {
    "empty": 'report["documents"] = []',
    "one-extra": 'report["documents"].append(dict(report["documents"][0], path="extra.md"))',
    "duplicate": 'report["documents"].append(dict(report["documents"][0]))',
}


@pytest.mark.parametrize("fault", list(_CONFIGURED_FAULTS))
def test_a_bad_configured_report_fails_the_audit_step(tmp_path, fault) -> None:
    done = run_audit(tmp_path, {"configured": _CONFIGURED_FAULTS[fault]})
    assert done.returncode != 0
    assert "configured: the report " in done.stderr
    _no_workflow_command(done.stderr)


def test_a_proper_subset_is_a_valid_configured_report(tmp_path) -> None:
    # The config's exclusions are the CLI's business, so a smaller report is not a fault.
    done = run_audit(tmp_path, {"configured": 'del report["documents"][0]'})
    assert done.returncode == 0, done.stderr


@pytest.mark.parametrize(
    ("configured", "all_files", "authored_count", "problems"),
    [
        pytest.param(["a.md"], ["a.md", "b.md"], 2, 0, id="proper-subset"),
        pytest.param(["a.md", "b.md"], ["a.md", "b.md"], 2, 0, id="same-set"),
        pytest.param([], ["a.md"], 0, 0, id="empty-with-no-authored"),
        pytest.param([], ["a.md"], 1, 1, id="empty-with-authored"),
        pytest.param(["a.md", "x.md"], ["a.md"], 1, 1, id="extra"),
        pytest.param(["a.md", "a.md"], ["a.md"], 1, 1, id="duplicate"),
    ],
)
def test_configured_coverage(configured, all_files, authored_count, problems) -> None:
    found = audit.configured_problems(configured, set(all_files), authored_count)
    assert len(found) == problems
    for message in found:
        assert ".md" not in message


_ALL_FILES_FAULTS = {"empty": 'report["documents"] = []', "one-omitted": 'del report["documents"][0]'}


@pytest.mark.parametrize("fault", list(_ALL_FILES_FAULTS))
def test_a_partial_all_files_report_fails_the_audit_step(tmp_path, fault) -> None:
    done = run_audit(tmp_path, {"all-files": _ALL_FILES_FAULTS[fault]})
    assert done.returncode != 0
    assert "all-files: the report " in done.stderr


_PAYLOAD = "::error::pwned\n##[group]x\n"


@pytest.mark.parametrize("scan", ["authored", "configured", "all-files"])
@pytest.mark.parametrize("code", [1, 2])
def test_a_failed_linter_with_no_report_fails_with_fixed_text(tmp_path, scan, code) -> None:
    case = {
        scan + "-noreport": True,
        scan + "-exit": code,
        scan + "-stdout": _PAYLOAD,
        scan + "-stderr": _PAYLOAD,
    }
    done = run_audit(tmp_path, case)
    assert done.returncode != 0
    assert f"{scan}: the linter exited {code} and wrote no report" in done.stderr
    assert "Traceback" not in done.stderr
    _no_workflow_command(done.stdout + done.stderr)
    assert "pwned" not in done.stdout + done.stderr


@pytest.mark.parametrize("scan", ["authored", "configured", "all-files"])
def test_a_report_path_outside_the_root_fails_with_fixed_text(tmp_path, scan) -> None:
    case = {scan: 'report["documents"][0]["path"] = "/outside/::error::pwned.md"'}
    done = run_audit(tmp_path, case)
    assert done.returncode != 0
    assert f"{scan}: document 0 lies outside the repository" in done.stderr
    assert "Traceback" not in done.stderr
    _no_workflow_command(done.stdout + done.stderr)
    assert "pwned" not in done.stdout + done.stderr
