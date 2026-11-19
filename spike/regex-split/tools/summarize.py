"""SPIKE: merge parity dumps into split counts by pattern source.

Usage: python3 summarize.py <fixtures-dir core-v3> <out.md> <dump.jsonl...>
"""
import collections
import json
import pathlib
import sys

fixtures = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
dumps = sys.argv[3:]

kind_of = {}
for f in sorted(fixtures.glob("*.json")):
    doc = json.loads(f.read_text())
    cat = doc.get("id") or f.stem
    for r in doc.get("rules", []):
        kind_of[f"{cat}.{r['id']}"] = r["match"]["kind"]

recs = {}
summaries = []
for d in dumps:
    fuzz = "fuzz" in pathlib.Path(d).name
    for line in open(d):
        j = json.loads(line)
        if j.get("summary"):
            summaries.append((d, j))
            continue
        j.setdefault("fuzz_diffs", 0)
        j.setdefault("fuzz_first", None)
        if fuzz:
            j["fuzz_diffs"], j["diffs"] = j["diffs"], 0
            j["fuzz_first"], j["first_diff"] = j["first_diff"], None
            j["calls"] = 0
        r = recs.get(j["id"])
        if r is None:
            if fuzz:
                continue
            recs[j["id"]] = j
            continue
        for k in ("calls", "diffs", "edge_diffs", "pcre_limit", "fuzz_diffs"):
            r[k] += j[k]
        if not r["first_diff"] and j["first_diff"]:
            r["first_diff"] = j["first_diff"]
        if not r["fuzz_first"] and j["fuzz_first"]:
            r["fuzz_first"] = j["fuzz_first"]
        if j["label"] and not r["label"]:
            r["label"] = j["label"]


def source(r):
    lab = r["label"]
    if lab.startswith("rule:"):
        rid = lab[5:]
        k = kind_of.get(rid, "?")
        if "slopvac-rules/src/matcher.rs" not in r["caller"]:
            return f"core rules (other: {r['caller'].split('/src/')[0].split('/')[-1]})", rid
        return f"core rules ({k})", rid
    c = r["caller"]
    if "slopvac-context" in c:
        return "slopvac-context", c
    if "slopvac-engine/src/exceptions" in c or "slopvac-engine/src/matching" in c:
        return "engine exceptions/matching", c
    if "slopvac-engine" in c:
        return "engine built-ins", c
    if "slopvac-text" in c:
        return "slopvac-text built-ins", c
    return "other", c


def verdict(r):
    if r["rust_err"]:
        return "no-compile"
    if r["diffs"] or r["edge_diffs"] or r.get("fuzz_diffs"):
        return "differs"
    return "identical"


def reason(err):
    e = err.lower()
    for key, name in [
        ("look-around", "lookaround"),
        ("backreference", "backreference"),
        ("unrecognized flag", "flag"),
        ("repetition quantifier expects", "possessive/odd quantifier"),
        ("unrecognized escape", "escape (\\K, \\G, \\Z, \\h, ...)"),
        ("exceeds size limit", "size limit"),
    ]:
        if key in e:
            return name
    return err.splitlines()[-1][:80] if err else "?"


groups = collections.defaultdict(list)
for r in recs.values():
    s, where = source(r)
    r["_where"] = where
    groups[s].append(r)

lines = ["# Split summary", "", f"Dumps: {', '.join(pathlib.Path(d).name for d in dumps)}", ""]
lines.append("| Source | Patterns | compiles+identical | compiles+differs | no compile | exercised (calls>0) |")
lines.append("|---|---|---|---|---|---|")
tot = collections.Counter()
for s in sorted(groups):
    c = collections.Counter(verdict(r) for r in groups[s])
    ex = sum(1 for r in groups[s] if r["calls"] > 0)
    tot.update(c)
    tot["n"] += len(groups[s])
    tot["ex"] += ex
    lines.append(f"| {s} | {len(groups[s])} | {c['identical']} | {c['differs']} | {c['no-compile']} | {ex} |")
lines.append(f"| total | {tot['n']} | {tot['identical']} | {tot['differs']} | {tot['no-compile']} | {tot['ex']} |")
lines.append("")
rc = collections.Counter(reason(r["rust_err"]) for r in recs.values() if r["rust_err"])
lines.append("No-compile reasons: " + ", ".join(f"{k} {v}" for k, v in rc.most_common()))
lines.append("")
lines.append("## Differs (first example each)")
for s in sorted(groups):
    for r in sorted(groups[s], key=lambda r: r["_where"]):
        if verdict(r) == "differs":
            lines.append(f"- [{s}] `{r['_where']}` diffs={r['diffs']} edge={r['edge_diffs']} fuzz={r.get('fuzz_diffs', 0)} calls={r['calls']}: `{r['source'][:160]}`")
            lines.append(f"  - {(r['first_diff'] or r.get('fuzz_first') or '')[:400]}")
lines.append("")
lines.append("## PCRE2-only (no compile in regex)")
for s in sorted(groups):
    for r in sorted(groups[s], key=lambda r: r["_where"]):
        if verdict(r) == "no-compile":
            lines.append(f"- [{s}] `{r['_where']}` {reason(r['rust_err'])}: `{r['source'][:160]}`")
lines.append("")
lines.append("## Parity allowlist size (identical and exercised)")
allow = [r["id"] for r in recs.values() if verdict(r) == "identical"]
lines.append(str(len(allow)))
for d, j in summaries:
    lines.append(f"- {pathlib.Path(d).name}: limit_hits={j['limit_hits']}")
out.write_text("\n".join(lines) + "\n")
(out.parent / "allow.txt").write_text("\n".join(sorted(allow)) + "\n")
with open(out.parent / "patterns.jsonl", "w") as f:
    for r in sorted(recs.values(), key=lambda r: (source(r)[0], r["_where"], r["id"])):
        f.write(json.dumps({
            "id": r["id"], "source": r["source"], "caseless": r["caseless"], "ucp": r["ucp"],
            "origin": source(r)[0], "where": r["_where"], "verdict": verdict(r),
        }) + "\n")
with open(out.parent / "patterns-identical.jsonl", "w") as f:
    for r in sorted(recs.values(), key=lambda r: (source(r)[0], r["_where"], r["id"])):
        if verdict(r) == "identical":
            f.write(json.dumps({
                "id": r["id"], "source": r["source"], "caseless": r["caseless"], "ucp": r["ucp"],
                "origin": source(r)[0], "where": r["_where"], "verdict": "identical",
            }) + "\n")
print(f"wrote {out}")
