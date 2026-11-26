"""The Markdown audit refuses a report whose documents carry no usable gate verdict.

`scripts/ci/markdown_audit_reports.py` holds the check, and the audit step in
`.github/workflows/markdown-audit.yml` calls it on every report. A document without
`gate.passed`, or with a non-boolean, must fail the audit. A valid `false` is allowed in
the diagnostic scans, which may fail, and not in the authored scan.
"""

from __future__ import annotations

import importlib.util
import re
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
    assert all("scan" in p and "doc1.md" in p for p in problems)


@pytest.mark.parametrize("report", [None, [], {}, {"documents": None}], ids=str)
def test_a_report_without_a_documents_list_fails(report) -> None:
    assert audit.gate_problems("scan", report, allow_false=True)


def _audit_script() -> str:
    steps = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))["jobs"]["inventory-and-audit"]["steps"]
    (step,) = [s for s in steps if s.get("name") == "Audit tracked Markdown"]
    return step["run"]


def test_the_audit_step_checks_every_report() -> None:
    script = _audit_script()
    assert "from markdown_audit_reports import document_set_problems, gate_problems" in script
    # One call per report, with the authored scan the only one that rejects `false`.
    assert re.search(r"gate_problems\(\s*name,\s*reports\[name\],\s*allow_false=name != \"authored\"\s*\)", script)
    assert "raise SystemExit" in script.split("gate_problems", 1)[1]


def test_the_cargo_cache_is_saved_only_by_a_push_to_main() -> None:
    # The workflow is called from test.yml, where `github.event_name` is the caller's event:
    # `pull_request`, `merge_group` or `push`. Only the push to main may save the cache.
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
