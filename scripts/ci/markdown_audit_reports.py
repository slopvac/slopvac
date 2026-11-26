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

import subprocess


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


def excluded_by_config(files: list[str], patterns: list[str]) -> set[str]:
    """Return the tracked `files` that the config's `exclude` patterns select out.

    The engine reads `exclude` as gitignore patterns, so git applies them here: no second
    glob matcher, and no list of the excluded files in the workflow.
    """
    if not patterns:
        return set()
    command = ["git", "ls-files", "-z", "--cached", "--ignored"]
    for pattern in patterns:
        command += ["--exclude", pattern]
    out = subprocess.run(command, capture_output=True, check=True, text=True).stdout
    return {path for path in out.split("\0") if path} & set(files)
