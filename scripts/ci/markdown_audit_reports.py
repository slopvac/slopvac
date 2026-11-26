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
    """Return one message per document of `report` whose gate verdict is unusable.

    The messages are fixed text: they name the scan by its fixed name and a document by its
    index, and never print a path or a value from the report, which can carry text that the
    Actions log would act on as a workflow command.
    """
    documents = report.get("documents") if isinstance(report, dict) else None
    if not isinstance(documents, list):
        return [f"{name}: the report has no `documents` list"]
    problems = []
    for index, document in enumerate(documents):
        gate = document.get("gate") if isinstance(document, dict) else None
        passed = gate.get("passed") if isinstance(gate, dict) else None
        if type(passed) is not bool:
            problems.append(f"{name}: document {index} has no boolean gate.passed")
        elif not passed and not allow_false:
            problems.append(f"{name}: document {index} failed its gate")
    return problems


def document_set_problems(name: str, reported: list[str], expected: set[str]) -> list[str]:
    """Return one message per way the report's documents differ from the expected set.

    `reported` holds the path of each report document, as the audit normalises it, and
    `expected` the paths the audit classed for the scan. The messages hold counts only:
    a path comes from the repository and can carry text the Actions log would act on.
    """
    problems = []
    found = set(reported)
    if missing := expected - found:
        problems.append(f"{name}: the report omits {len(missing)} expected document(s)")
    if extra := found - expected:
        problems.append(f"{name}: the report holds {len(extra)} unexpected document(s)")
    if duplicates := len(reported) - len(found):
        problems.append(f"{name}: the report holds {duplicates} duplicate document(s)")
    return problems


def configured_problems(reported: list[str], all_files: set[str], authored_count: int) -> list[str]:
    """Return one message per fault of the configured report.

    The CLI's own exclusion rules pick the configured files, and this audit does not repeat
    them. It checks only what must hold whatever the config selects: every document is one
    the all-files scan also holds, none repeats, and the report is not empty when authored
    documents exist. The messages hold counts only.
    """
    problems = []
    if extra := set(reported) - all_files:
        problems.append(f"configured: the report holds {len(extra)} document(s) the all-files report lacks")
    if duplicates := len(reported) - len(set(reported)):
        problems.append(f"configured: the report holds {duplicates} duplicate document(s)")
    if not reported and authored_count:
        problems.append("configured: the report is empty but authored documents exist")
    return problems


def path_problems(name: str, report: object) -> list[str]:
    """Return one message per document of `report` that has no usable `path`.

    A path is a non-empty string. The messages name the scan and the document index only.
    """
    documents = report.get("documents") if isinstance(report, dict) else None
    if not isinstance(documents, list):
        return []
    problems = []
    for index, document in enumerate(documents):
        path = document.get("path") if isinstance(document, dict) else None
        if not isinstance(path, str) or not path:
            problems.append(f"{name}: document {index} has no path")
    return problems
