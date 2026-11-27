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


_CORE = {"layer": "core"}
_PROJECT = {"layer": "project", "file": "slopvac.toml"}


def routing_shape_problems(document: object) -> list[str]:
    """Return one message per fault in the shape of an `--explain-config --format json` document.

    The audit reads `inputs[].path` before it can compare the paths, so it checks the shape
    first. The messages are fixed text and carry nothing from the document.
    """
    if not isinstance(document, dict):
        return ["routing: the output is not a JSON object"]
    problems = []
    if document.get("schema_version") != 3:
        problems.append("routing: the output has no schema_version 3")
    inputs = document.get("inputs")
    if not isinstance(inputs, list):
        return [*problems, "routing: the output has no `inputs` list"]
    for index, record in enumerate(inputs):
        path = record.get("path") if isinstance(record, dict) else None
        if not isinstance(path, str) or not path:
            problems.append(f"routing: input {index} has no path")
    return problems


def _int(value: object) -> bool:
    return type(value) is int


def routing_problems(inputs: list[dict], paths: list[str], styles: object = None) -> list[str]:
    """Return one message per way the resolved config of an authored document is wrong.

    `inputs` holds the records of the document, and `paths` the path of each, as the audit
    normalises it. The repository config must resolve for every document: the config root is
    the working directory, the config sources are the built-in defaults and `slopvac.toml`,
    and the profile is `normal`. A `README.md` takes `thresholds.min_score` from an override
    block and no other document does. The resolved `styles` setting equals `styles`, the
    value the repository's `slopvac.toml` holds (`None` when it sets none). The catalogue is
    not empty and not every rule is off.
    The messages hold counts only: a path or a value comes from the repository and can carry
    text the Actions log would act on.
    """
    wrong_config = wrong_profile = wrong_override = wrong_rules = wrong_styles = wrong_settings = 0
    for record, path in zip(inputs, paths, strict=True):
        if record.get("config_root") != "." or record.get("config_sources") != [_CORE, _PROJECT]:
            wrong_config += 1
        if record.get("profile") != "normal":
            wrong_profile += 1
        rules = record.get("rules")
        counts = rules.get("configured") if isinstance(rules, dict) else None
        total = rules.get("total") if isinstance(rules, dict) else None
        off = counts.get("off") if isinstance(counts, dict) else None
        if not (_int(total) and _int(off) and total > 0 and total > off):
            wrong_rules += 1
        settings = record.get("settings")
        if not isinstance(settings, list) or not all(isinstance(s, dict) for s in settings):
            # A record with no usable settings can pass no per-setting check.
            wrong_settings += 1
            continue
        layers = [
            s.get("source", {}).get("layer") if isinstance(s.get("source"), dict) else None
            for s in settings
            if s.get("key") == "thresholds.min_score"
        ]
        overridden = layers == ["override"]
        if overridden != (path.rsplit("/", 1)[-1] == "README.md"):
            wrong_override += 1
        resolved_styles = [
            s.get("value") for s in settings if s.get("key") == "styles"
        ]
        if resolved_styles != ([] if styles is None else [styles]):
            wrong_styles += 1
    problems = []
    if wrong_config:
        problems.append(f"routing: {wrong_config} document(s) do not resolve to the repository config")
    if wrong_profile:
        problems.append(f"routing: {wrong_profile} document(s) do not resolve to the normal profile")
    if wrong_settings:
        problems.append(f"routing: {wrong_settings} document(s) have no resolved settings")
    if wrong_styles:
        problems.append(f"routing: {wrong_styles} document(s) do not resolve to the repository styles")
    if wrong_override:
        problems.append(f"routing: {wrong_override} document(s) have the wrong README override")
    if wrong_rules:
        problems.append(f"routing: {wrong_rules} document(s) have no rule that runs")
    return problems


def routing_counts(document: dict) -> dict:
    """The counts the audit prints for a document that passed the routing checks."""
    inputs = document["inputs"]
    return {
        "documents": len(inputs),
        "rules_total": inputs[0]["rules"]["total"] if inputs else 0,
        "front_matter_resolved": any(r.get("front_matter_resolved") is True for r in inputs),
    }
