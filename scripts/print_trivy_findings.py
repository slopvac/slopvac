"""Print the findings in a Trivy SARIF file, one escaped line each, for the job log.

A scanned file name or a finding text is untrusted content. The runner reads a workflow
command from a log line that starts with `::` or `##[`, so every field goes through
`escape_workflow_data`, the repo's one escaper, before it is printed.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

from workflow_escape import escape_workflow_data


def lines(sarif: dict) -> list[str]:
    out = []
    for run in sarif.get("runs") or []:
        for result in run.get("results") or []:
            first = (result.get("message") or {}).get("text", "").split("\n")[0]
            places = result.get("locations") or [{}]
            uri = (
                (places[0].get("physicalLocation") or {})
                .get("artifactLocation", {})
                .get("uri")
            ) or "?"
            rule = result.get("ruleId", "?")
            level = result.get("level", "?")
            out.append(escape_workflow_data(f"{rule} [{level}] {first} @ {uri}"))
    return out


def main(argv: list[str]) -> int:
    (path,) = argv
    try:
        found = lines(json.loads(Path(path).read_text()))
    except (OSError, ValueError, AttributeError, TypeError, IndexError):
        # One fixed line: no path, no parser text, no traceback.
        sys.stdout.write("Trivy report is missing or unreadable\n")
        return 1
    for line in found:
        sys.stdout.write(line + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
