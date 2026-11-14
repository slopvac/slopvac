#!/usr/bin/env python3
# STUB(slopvac-8l4.13.12): stands in for the 3.0 binary, whose lint command and
# renderers are not on main. It writes contract-shaped reports for one finding:
# lint-report JSON (schemas/v3/lint-report.schema.json), SARIF 2.1.0, reviewdog
# rdjsonl and a Code Climate issues array.
#
# Behaviour switches, all optional:
#   FAKE_LOG              append one JSON array of argv per call to this file
#   FAKE_EXIT_<FORMAT>    exit status for that format (default 1, "findings")
#   FAKE_SIGNAL_<FORMAT>  die from this signal number instead of exiting
#   FAKE_EXIT_SYNC        exit status of `plugins sync` (default 0)
import json
import os
import signal
import sys

args = sys.argv[1:]
log = os.environ.get("FAKE_LOG")
if log:
    with open(log, "a", encoding="utf-8") as handle:
        handle.write(json.dumps(args) + "\n")

if "plugins" in args and "sync" in args:
    sys.exit(int(os.environ.get("FAKE_EXIT_SYNC", "0")))

fmt = args[args.index("--format") + 1]
out = args[args.index("--out") + 1]
targets = args[args.index("--") + 1 :] if "--" in args else []
path = targets[0] if targets else "README.md"
rule_id = "core:prose-inflation.slop-lexicon"
message = "Replace 'delve' with a plain verb."

CATEGORY = {
    "category": "prose-inflation",
    "findings": 1,
    "errors": 0,
    "warnings": 1,
    "suggestions": 0,
    "per_100_words": 5.0,
    "score": 96.0,
}
AI_REGISTER = {
    "strong": {"findings": 1, "errors": 0, "warnings": 1, "per_100_words": 5.0},
    "weak": {"findings": 0, "errors": 0, "warnings": 0, "per_100_words": 0.0},
}
PROSE = {"findings": 1, "errors": 0, "warnings": 1, "per_100_words": 5.0}

reports = {
    "json": {
        "schema_version": 3,
        "slopvac_version": "3.0.0",
        "summary": {
            "documents": 1,
            "words": 20,
            "findings": 1,
            "errors": 0,
            "warnings": 1,
            "suggestions": 0,
            "per_100_words": 5.0,
            "score": 96.0,
            "passed": False,
            "categories": [CATEGORY],
            "ai_register": AI_REGISTER,
            "prose": PROSE,
        },
        "documents": [
            {
                "path": path,
                "profile": "normal",
                "genre": {"id": "reference", "source": "default", "reasons": []},
                "contexts_seen": ["prose"],
                "config_sources": [{"layer": "core"}],
                "words": 20,
                "sentences": 2,
                "paragraphs": 1,
                "findings": [
                    {
                        "rule_id": rule_id,
                        "pack": "core",
                        "category": "prose-inflation",
                        "severity": "warning",
                        "message": message,
                        "span": {"start": 141, "end": 146},
                        "line": 4,
                        "column": 12,
                        "end_line": 4,
                        "end_column": 17,
                        "contexts": ["prose"],
                        "ai_signal": "strong",
                        "matched_text": "delve",
                    }
                ],
                "categories": [
                    {
                        **CATEGORY,
                        "gating_per_100_words": 5.0,
                        "budget": 1.0,
                        "over_budget": True,
                    }
                ],
                "total_findings": 1,
                "errors": 0,
                "warnings": 1,
                "suggestions": 0,
                "per_100_words": 5.0,
                "score": 96.0,
                "gate": {
                    "passed": False,
                    "reasons": ["prose-inflation is over budget"],
                },
                "unchecked": [],
                "ai_register": AI_REGISTER,
                "prose": PROSE,
            }
        ],
        "notices": [],
    },
    "rdjsonl": {
        "message": message,
        "location": {
            "path": path,
            "range": {
                "start": {"line": 4, "column": 12},
                "end": {"line": 4, "column": 17},
            },
        },
        "severity": "WARNING",
        "source": {"name": "slopvac"},
        "code": {"value": rule_id},
    },
    "sarif": {
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [
            {
                "tool": {
                    "driver": {
                        "name": "slopvac",
                        "version": "3.0.0",
                        "rules": [{"id": rule_id}],
                    }
                },
                "results": [
                    {
                        "ruleId": rule_id,
                        "level": "warning",
                        "message": {"text": message},
                        "locations": [
                            {
                                "physicalLocation": {
                                    "artifactLocation": {"uri": path},
                                    "region": {
                                        "startLine": 4,
                                        "startColumn": 12,
                                        "endLine": 4,
                                        "endColumn": 17,
                                    },
                                }
                            }
                        ],
                    }
                ],
            }
        ],
    },
    "codeclimate": [
        {
            "type": "issue",
            "check_name": rule_id,
            "description": message,
            "categories": ["Style"],
            "severity": "minor",
            "fingerprint": "0123456789abcdef0123456789abcdef",
            "location": {"path": path, "lines": {"begin": 4, "end": 4}},
        }
    ],
}

with open(out, "w", encoding="utf-8") as handle:
    handle.write(json.dumps(reports[fmt]) + "\n")

signum = os.environ.get("FAKE_SIGNAL_" + fmt.upper())
if signum:
    os.kill(os.getpid(), signal.Signals(int(signum)))
sys.exit(int(os.environ.get("FAKE_EXIT_" + fmt.upper(), "1")))
