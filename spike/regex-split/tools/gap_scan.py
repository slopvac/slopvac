"""SPIKE: static scan for constructs whose meaning differs between PCRE2 (UTF+UCP) and Rust
`regex`, over the patterns whose spans matched in every parity run.

Usage: python3 gap_scan.py <patterns.jsonl>  -> one JSON line per pattern with a gap.
Gaps checked (outside character classes unless noted):
  dollar-eol   `$` without (?m): PCRE2 also matches before a final \\n, Rust only at the end
  caret-m-end  `(?m)^`: Rust matches after a final \\n at the end of the subject, PCRE2 not
  bigZ         `\\Z` (Rust has no \\Z; would not compile)
  posix-class  `[[:alpha:]]` etc: PCRE2 UCP makes them Unicode, Rust keeps them ASCII
  dot-crlf     none: both stop `.` at \\n only (PCRE2 newline is LF in this build)
"""
import json
import re
import sys


def strip_classes(src):
    out, i, n = [], 0, len(src)
    while i < n:
        c = src[i]
        if c == "\\":
            out.append(src[i : i + 2])
            i += 2
            continue
        if c == "[":
            j = i + 1
            if j < n and src[j] == "^":
                j += 1
            if j < n and src[j] == "]":
                j += 1
            while j < n and src[j] != "]":
                if src[j] == "\\":
                    j += 1
                elif src.startswith("[:", j):
                    k = src.find(":]", j)
                    j = k + 1 if k > 0 else j
                j += 1
            out.append("[]")
            i = j + 1
            continue
        out.append(c)
        i += 1
    return "".join(out)


for line in open(sys.argv[1]):
    r = json.loads(line)
    s = r["source"]
    flat = strip_classes(s)
    gaps = []
    multiline = bool(re.search(r"\(\?[a-z]*m", flat))
    if re.search(r"(?<!\\)\$", flat.replace("\\\\", "")) and not multiline:
        gaps.append("dollar-eol")
    if multiline and "^" in flat:
        gaps.append("caret-m-end")
    if "\\Z" in flat:
        gaps.append("bigZ")
    if re.search(r"\[:\^?[a-z]+:\]", s) and r.get("ucp"):
        gaps.append("posix-class")
    if gaps:
        print(json.dumps({"id": r["id"], "origin": r["origin"], "verdict": r["verdict"], "gaps": gaps, "source": s[:200]}))
