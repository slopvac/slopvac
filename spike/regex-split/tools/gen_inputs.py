"""SPIKE: build the input classes and the malicious plugin pattern list.

Usage: python3 gen_inputs.py <repo> <outdir>
Writes <outdir>/inputs/<class>.txt, <outdir>/plugin-patterns.jsonl and, per plugin pattern,
attack strings in <outdir>/attack-<size>/<id>.txt.
"""
import json
import pathlib
import random
import re
import sys

repo = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
inp = out / "inputs"
inp.mkdir(parents=True, exist_ok=True)
MiB = 1 << 20

# Real prose from the eval corpus, newlines folded so the whole text is one block.
prose = []
for p in sorted((repo / "evals/corpus").rglob("*")):
    if p.is_file() and p.suffix in {".md", ".txt", ".rst", ".mdx"}:
        prose.append(p.read_text(errors="replace"))
prose = " ".join(re.sub(r"\s+", " ", t) for t in prose)


def fill(unit, size):
    n = size // max(1, len(unit.encode())) + 1
    s = (unit * n).encode()[:size]
    return s.decode("utf-8", "ignore")


def write(name, text):
    (inp / f"{name}.txt").write_text(text)


for mib in (1, 8, 64):
    write(f"block-{mib}m", fill(prose + " ", mib * MiB))
    # One sentence: prose with every sentence end removed.
    write(f"sentence-{mib}m", fill(re.sub(r"[.!?;:]", ",", prose) + " ", mib * MiB))

for n in (1000, 10000):
    write(f"nest-paren-{n // 1000}k", "(" * n + "x" + ")" * n)
    write(f"nest-mixed-{n // 1000}k", "".join(random.Random(n).choice("([{<`\"'*_") for _ in range(n)) + "word" + "".join(random.Random(n + 1).choice(")]}>`\"'*_") for _ in range(n)))
    write(f"nest-list-{n // 1000}k", "".join("  " * min(i, 200) + "- item\n" for i in range(n)))

for name, unit in [("ws", " \t "), ("dots", "."), ("dash", "-"), ("a", "a"), ("word", "word "), ("caps", "WORD "), ("quote", "\" "), ("apos", "'s ")]:
    write(f"run-{name}-1m", fill(unit, MiB))

# Near-miss runs for lexicon rules: every bad example from the core rules with the last
# letter of each word changed, so alternations get deep before they fail.
bad = []
for f in sorted((repo / "crates/slopvac-rules/tests/fixtures/core-v3").glob("*.json")):
    for r in json.loads(f.read_text()).get("rules", []):
        ex = r.get("examples") or []
        for e in ex if isinstance(ex, list) else [ex]:
            b = e.get("bad") if isinstance(e, dict) else None
            for t in b if isinstance(b, list) else [b]:
                if isinstance(t, str):
                    bad.append(t)


def near(s):
    return re.sub(r"(\w)\b", lambda m: "q" if m.group(1) != "q" else "z", s)


nm = " ".join(near(b) for b in bad if b)
write("nearmiss-1m", fill(nm + " ", MiB))
write("nearmiss-8m", fill(nm + " ", 8 * MiB))

# Malicious plugin patterns: (id, source, caseless, pump unit, suffix, kind).
PLUGIN = [
    ("nested-plus", r"(a+)+$", False, "a", "!", "exp"),
    ("alt-overlap", r"(a|aa)*$", False, "a", "!", "exp"),
    ("alt-class-overlap", r"(\w|\d)+$", False, "1", "!", "exp"),
    ("star-backref", r"(a*)*\1", False, "a", "!", "exp"),
    ("lookahead-repeat", r"(?=(a+))+b", False, "a", "!", "poly"),
    ("lookahead-nested", r"((?=a)a+)+$", False, "a", "!", "exp"),
    ("lookbehind-var", r"(?<=a{0,200})(?<!b{1,200})x", False, "a", "y", "poly"),
    ("lookbehind-chain", r"(?:(?<=a)a)+$", False, "a", "!", "poly"),
    ("k-abuse", r"(?:a+\K)+b", False, "a", "!", "exp"),
    ("k-empty", r"a*\K", False, "a", "!", "poly"),
    ("words-dollar", r"(\w+\s?)+$", False, "word ", "!", "exp"),
    ("dot-star-poly", r"(.*a){12}", False, "a", "!", "poly"),
    ("double-plus", r"(x+x+)+y", False, "x", "!", "exp"),
    ("letters-star", r"([a-zA-Z]+)*$", False, "a", "1", "exp"),
    ("ws-star", r"(\s*)*$", False, " ", "!", "exp"),
    ("same-alt", r"(?:a|a)*$", False, "a", "!", "exp"),
    ("recursion", r"\((?:[^()]|(?R))*\)", False, "(", "", "depth"),
    ("poly3", r"\w*\w*\w*!", False, "a", "", "poly"),
    ("poly2-unanchored", r"\s+\S+\s+$", False, " a", "!", "poly"),
    ("caseless-alt", r"(?i)(k|K|\x{212A})+$", False, "k", "!", "exp"),
    ("atomic-ok", r"(?>a+)+b", False, "a", "!", "safe"),
    ("possessive-ok", r"(a++)+b", False, "a", "!", "safe"),
]
with open(out / "plugin-patterns.jsonl", "w") as f:
    for pid, src, ci, unit, suffix, kind in PLUGIN:
        f.write(json.dumps({"id": f"plugin:{pid}", "source": src, "caseless": ci, "ucp": True, "origin": "plugin", "kind": kind}) + "\n")
for label, reps in [("30", 30), ("64k", 64 * 1024), ("1m", MiB)]:
    d = out / f"attack-{label}"
    d.mkdir(exist_ok=True)
    for pid, src, ci, unit, suffix, kind in PLUGIN:
        n = reps if label != "30" else 30
        body = (unit * (n // len(unit) + 1))[: max(n, len(unit))] if label != "30" else unit * 30
        (d / f"plugin:{pid}.txt").write_text(body + suffix)
print("inputs written")
