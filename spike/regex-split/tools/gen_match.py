"""SPIKE: pattern-directed differential inputs.

For each pattern, parse it with the shape_check parser and random-walk the tree to build
strings the pattern is likely to match, then wrap each in variants that probe the known
PCRE2 / Rust `regex` gaps: trailing `\\n` (`$` and `\\Z`), CRLF, a leading newline
(`(?m)^` at the end of the subject), surrounding words (`\\b`), upper case (case folding).
Writes one JSON line per pattern: {"id", "strings": [...]}.

Usage: python3 gen_match.py <patterns.jsonl> <out.jsonl> [samples=150]
"""
import json
import random
import sys

import shape_check as sc

SAMPLES = int(sys.argv[3]) if len(sys.argv) > 3 else 150
LOOK = ("la", "nla", "lb", "nlb")


def gen(n, rnd, alphabet, depth=0):
    t = n[0]
    if depth > 60:
        return ""
    if t == "set":
        cs = [c for c in alphabet if c != sc.OTHER and n[1](c)]
        return rnd.choice(cs) if cs else ""
    if t == "seq":
        return "".join(gen(k, rnd, alphabet, depth + 1) for k in n[1])
    if t == "alt":
        return gen(rnd.choice(n[1]), rnd, alphabet, depth + 1)
    if t == "rep":
        lo, hi = n[2], n[3]
        top = lo + 3 if hi is None else min(hi, lo + 3)
        k = rnd.randint(lo, max(lo, top))
        return "".join(gen(n[1], rnd, alphabet, depth + 1) for _ in range(k))
    if t == "group":
        return "" if n[2] in LOOK else gen(n[1], rnd, alphabet, depth + 1)
    return ""


def variants(s):
    return [s, s + "\n", s + "\n\n", s + "\r\n", "\n" + s, "x\n" + s + "\n", "word " + s + " word",
            s.upper(), s.title(), "(" + s + ")", s + ".", "\u00a0" + s + "\u2028"]


def main():
    out = open(sys.argv[2], "w")
    for line in open(sys.argv[1]):
        r = json.loads(line)
        try:
            p = sc.Parser(r["source"], r.get("caseless", False))
            tree = p.parse()
        except (sc.ParseError, ValueError, IndexError):
            continue
        # Prefer literals: they make walks hit the words the pattern names.
        alphabet = sorted(p.lits) * 4 + sc.REPS
        rnd = random.Random(r["id"])
        strings = []
        seen = set()
        for _ in range(SAMPLES):
            s = gen(tree, rnd, alphabet)
            if s in seen:
                continue
            seen.add(s)
            strings.extend(variants(s))
        strings.extend(["", "\n", "\n\n", " \n", "a\n"])
        out.write(json.dumps({"id": r["id"], "strings": strings}) + "\n")


if __name__ == "__main__":
    main()
