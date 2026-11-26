#!/usr/bin/env python3
"""Check the gate verdict of every document in a 3.x JSON report, for the Markdown audit.

The audit in `.github/workflows/markdown-audit.yml` reads `documents[].gate.passed` to
tell a passing document from a failing one. A report whose document lacks that key, or
holds anything but a boolean, says nothing about the document, so it is a failure of the
audit and never a pass.

A document that carries `gate.passed: false` is valid in a diagnostic scan (the scan may
fail). It is a problem only where `allow_false` is off, as in the authored scan.
"""

from __future__ import annotations


def gate_problems(name: str, report: object, *, allow_false: bool) -> list[str]:
    """Return one message per document of `report` whose gate verdict is unusable."""
    documents = report.get("documents") if isinstance(report, dict) else None
    if not isinstance(documents, list):
        return [f"{name}: the report has no `documents` list"]
    problems = []
    for index, document in enumerate(documents):
        path = document.get("path", f"#{index}") if isinstance(document, dict) else f"#{index}"
        gate = document.get("gate") if isinstance(document, dict) else None
        passed = gate.get("passed") if isinstance(gate, dict) else None
        if type(passed) is not bool:
            problems.append(f"{name}: {path}: `gate.passed` is {passed!r}, not a boolean")
        elif not passed and not allow_false:
            problems.append(f"{name}: {path}: the document failed its gate")
    return problems
