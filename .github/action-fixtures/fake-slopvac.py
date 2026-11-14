#!/usr/bin/env python3
# STUB(slopvac-8l4.13.12): stands in for the 3.0 binary, whose lint command and
# renderers are not on main. It writes canned reports and exits 1 (findings).
import json
import sys

args = sys.argv[1:]
fmt = args[args.index("--format") + 1]
out = args[args.index("--out") + 1]
summary = {
    "score": 91.5,
    "findings": 3,
    "errors": 1,
    "warnings": 2,
    "suggestions": 0,
    "passed": False,
}
body = (
    json.dumps({"summary": summary}) if fmt == "json" else json.dumps({"format": fmt})
)
with open(out, "w", encoding="utf-8") as handle:
    handle.write(body)
sys.exit(1)
