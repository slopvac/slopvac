"""SPIKE: self-test of shape_check on hand-picked shapes (expected verdict in the list)."""
import shape_check as s

CASES = [
    (r"(a+)+$", True), (r"(a|aa)*$", True), (r"(\w|\d)+$", True), (r"(a*)*\1", True),
    (r"(?=(a+))+b", True), (r"((?=a)a+)+$", True), (r"(?<=a{0,200})(?<!b{1,200})x", True),
    (r"(?:a+\K)+b", True), (r"(\w+\s?)+$", True), (r"(.*a){12}", True), (r"(x+x+)+y", True),
    (r"([a-zA-Z]+)*$", True), (r"(\s*)*$", True), (r"(?:a|a)*$", True),
    (r"\((?:[^()]|(?R))*\)", True), (r"\w*\w*\w*!", True), (r"(?i)(k|K|\x{212A})+$", True),
    (r"(?>a+)+b", False), (r"(a++)+b", False),
    (r"(?<![\w-])(?:very|really)\s+\w+(?![\w-])", False), (r"(\w+\s)+x", False),
    (r"[^()]+(?:\([^()]*\))*", False), (r"\b(?:leverage|utilize)s?\b", False),
]
bad = 0
for p, want in CASES:
    hits = [h[0] for h in s.check(p, False)]
    got = bool(set(hits) & s.REJECT)
    mark = "ok " if got == want else "BAD"
    bad += got != want
    print(mark, p, hits)
print("mismatches", bad)
