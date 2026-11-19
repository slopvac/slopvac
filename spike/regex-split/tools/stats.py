"""SPIKE: summary numbers for recheck and the shape check (prints Markdown).

Usage: python3 stats.py <patterns.jsonl> <plugin-patterns.jsonl> <shape.jsonl> <recheck.jsonl>
"""
import collections
import json
import sys

pats = {json.loads(l)["id"]: json.loads(l) for l in open(sys.argv[1])}
plug = {json.loads(l)["id"]: json.loads(l) for l in open(sys.argv[2])}
shape = {json.loads(l)["id"]: json.loads(l) for l in open(sys.argv[3])}
rech = {json.loads(l)["id"]: json.loads(l) for l in open(sys.argv[4])}

print("## Shape check by source")
print("| Source | Patterns | flag (reject) | warn | ok | parse error |")
print("|---|---|---|---|---|---|")
by = collections.defaultdict(collections.Counter)
for i, s in shape.items():
    by[s["origin"]][s["verdict"]] += 1
for o in sorted(by):
    c = by[o]
    print(f"| {o} | {sum(c.values())} | {c['flag']} | {c['warn']} | {c['ok']} | {c['parse-error']} |")
print()
print("Flagged non-plugin patterns:")
for i, s in shape.items():
    if s["origin"] != "plugin" and s["verdict"] in ("flag", "parse-error"):
        p = pats.get(i, {})
        print(f"- {s['verdict']} {s['rules']} `{p.get('where', i)}` {s.get('error') or ''}")
print()
print("Plugin patterns:")
for i, p in plug.items():
    s = shape.get(i, {})
    print(f"- `{p['source']}` ({p['kind']}): {s.get('verdict')} {s.get('rules')}")
print()
print("## recheck")
c = collections.Counter()
for i, r in rech.items():
    cx = (r.get("complexity") or {})
    c[(r.get("status"), cx.get("type"))] += 1
for k, v in c.most_common():
    print(f"- {k}: {v}")
print()
print("recheck vulnerable vs shape check (non-plugin):")
vul = {i for i, r in rech.items() if r.get("status") == "vulnerable"}
flag = {i for i, s in shape.items() if s["verdict"] == "flag" and s["origin"] != "plugin"}
print(f"- vulnerable {len(vul)}, shape-flagged {len(flag)}, both {len(vul & flag)}")
for i in sorted(vul):
    r = rech[i]
    p = pats.get(i, {})
    cx = r.get("complexity") or {}
    print(f"- `{p.get('where', i)}` {cx.get('type')} deg={cx.get('degree')} attack_len={r.get('attack_native_len')} shape={shape.get(i, {}).get('verdict')}")
errs = collections.Counter((r.get("error") or "")[:60] for r in rech.values() if r.get("status") not in ("safe", "vulnerable"))
print()
print("recheck unknown/error reasons:", dict(errs.most_common(8)))
