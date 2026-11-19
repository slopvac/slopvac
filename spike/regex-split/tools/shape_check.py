"""SPIKE prototype of the static dangerous-shape check.

A small recursive-descent parser for the PCRE2 subset slopvac patterns use (groups,
lookaround, atomic, named groups, inline flags, classes with \\p{..}, escapes, \\Q..\\E,
backreferences, recursion, \\K, greedy/lazy/possessive quantifiers), then a few rules over
the tree. Character sets are evaluated over a finite alphabet: a fixed list of
representative characters plus every literal in the pattern, so "overlap" is exact for
literals and approximate for classes.

Usage: python3 shape_check.py <patterns.jsonl> [--json]
Prints one line per pattern: id, verdict (ok / flag), rules hit.
"""
import json
import sys
import unicodedata

REPS = list("azksxeAZKSXE09 \t\n\r.,-_'\"()[]{}<>!?:;/*`#|+=@&%$~^\\") + [
    "é", "ß", "ж", "漢", "\u0301", "\u00a0", "\u2014", "\u2013", "’", "“", "”", "\u212a", "\u017f",
    "²", "½", "\u200d", "\u2028", "\u0085", "٣", "👍",
]
OTHER = "\U000F0000"  # stands for any character outside the alphabet


class ParseError(Exception):
    pass


def is_word(c):
    if c == OTHER:
        return False
    cat = unicodedata.category(c)
    return c.isalpha() or cat.startswith("M") or cat == "Nd" or cat == "Pc" or c in "\u200c\u200d"


def prop(name, c):
    if c == OTHER:
        return False
    neg = name.startswith("^")
    name = name.lstrip("^")
    cat = unicodedata.category(c)
    alias = {"Alphabetic": None, "Alpha": None, "L&": "LC", "Letter": "L", "Mark": "M",
             "Number": "N", "Punctuation": "P", "Symbol": "S", "Separator": "Z"}
    name = alias.get(name, name) if name in alias and alias[name] else name
    if name in ("Alphabetic", "Alpha"):
        r = c.isalpha()
    elif name == "LC":
        r = cat in ("Lu", "Ll", "Lt")
    elif len(name) <= 2 and name[:1].isupper():
        r = cat.startswith(name)
    elif name in ("Han", "Latin", "Greek", "Cyrillic"):
        r = name.upper() in unicodedata.name(c, "")
    else:
        r = False
    return r != neg


def esc_pred(e):
    return {
        "w": is_word,
        "W": lambda c: not is_word(c),
        "d": lambda c: c != OTHER and unicodedata.category(c) == "Nd",
        "D": lambda c: not (c != OTHER and unicodedata.category(c) == "Nd"),
        "s": lambda c: c != OTHER and c.isspace(),
        "S": lambda c: not (c != OTHER and c.isspace()),
        "h": lambda c: c in " \t\u00a0",
        "H": lambda c: c not in " \t\u00a0",
        "v": lambda c: c in "\n\r\x0b\x0c\u0085\u2028\u2029",
        "V": lambda c: c not in "\n\r\x0b\x0c\u0085\u2028\u2029",
        "N": lambda c: c != "\n",
        "R": lambda c: c in "\n\r\x0b\x0c\u0085\u2028\u2029",
        "X": lambda c: True,
    }.get(e)


SIMPLE = {"n": "\n", "t": "\t", "r": "\r", "f": "\f", "v": "\v", "e": "\x1b", "a": "\x07", "0": "\0"}


class Parser:
    def __init__(self, src, caseless):
        self.s = src
        self.i = 0
        self.ci = [caseless]
        self.ngroups = 0
        self.lits = set()

    def peek(self, k=0):
        j = self.i + k
        return self.s[j] if j < len(self.s) else None

    def take(self):
        c = self.s[self.i]
        self.i += 1
        return c

    def parse(self):
        n = self.alt()
        if self.i != len(self.s):
            raise ParseError(f"unexpected {self.s[self.i]!r} at {self.i}")
        return n

    def alt(self):
        self.ci.append(self.ci[-1])
        branches = [self.seq()]
        while self.peek() == "|":
            self.take()
            branches.append(self.seq())
        self.ci.pop()
        return branches[0] if len(branches) == 1 else ("alt", branches)

    def seq(self):
        items = []
        while self.peek() is not None and self.peek() not in "|)":
            a = self.atom()
            if a is None:
                continue
            items.append(self.quant(a))
        return ("seq", items)

    def quant(self, a):
        c = self.peek()
        lo = hi = None
        if c == "*":
            lo, hi = 0, None
        elif c == "+":
            lo, hi = 1, None
        elif c == "?":
            lo, hi = 0, 1
        elif c == "{":
            j = self.s.find("}", self.i)
            body = self.s[self.i + 1 : j] if j > 0 else ""
            parts = body.split(",")
            if j > 0 and 1 <= len(parts) <= 2 and all(p.strip().isdigit() or (k == 1 and p.strip() == "") or (k == 0 and p.strip() == "" and len(parts) == 2) for k, p in enumerate(parts)):
                lo = int(parts[0]) if parts[0].strip() else 0
                hi = lo if len(parts) == 1 else (int(parts[1]) if parts[1].strip() else None)
                self.i = j
            else:
                return a
        else:
            return a
        self.take()
        mode = "greedy"
        if self.peek() == "?":
            self.take()
            mode = "lazy"
        elif self.peek() == "+":
            self.take()
            mode = "possessive"
        return self.quant(("rep", a, lo, hi, mode))

    def charnode(self, pred, desc):
        ci = self.ci[-1]
        if ci:
            base = pred
            pred = lambda c, b=base: b(c) or (c != OTHER and (b(c.lower()) or b(c.upper()) or (c.casefold() != c and len(c.casefold()) == 1 and b(c.casefold()))))
        return ("set", pred, desc)

    def lit(self, ch):
        self.lits.add(ch)
        return self.charnode(lambda c, ch=ch: c == ch, ch)

    def atom(self):
        c = self.take()
        if c == "(":
            return self.group()
        if c == "[":
            return self.cls()
        if c == ".":
            return ("set", lambda x: x != "\n", ".")
        if c in "^$":
            return ("assert", c)
        if c == "\\":
            return self.escape(in_class=False)
        if c in "*+?":
            raise ParseError(f"quantifier without operand at {self.i}")
        return self.lit(c)

    def name_until(self, end):
        j = self.s.index(end, self.i)
        n = self.s[self.i : j]
        self.i = j + 1
        return n

    def group(self):
        kind, name = "cap", None
        if self.peek() == "?":
            self.take()
            c = self.take()
            if c == ":":
                kind = "noncap"
            elif c == ">":
                kind = "atomic"
            elif c == "=":
                kind = "la"
            elif c == "!":
                kind = "nla"
            elif c == "|":
                kind = "reset"
            elif c == "#":
                self.name_until(")")
                return None
            elif c == "<" and self.peek() in "=!":
                kind = "lb" if self.take() == "=" else "nlb"
            elif c == "<" or (c == "P" and self.peek() == "<") or c == "'":
                if c == "P":
                    self.take()
                name = self.name_until(">" if c != "'" else "'")
                kind = "cap"
            elif c == "P" and self.peek() in "=>":
                k = self.take()
                self.name_until(")")
                return ("backref",) if k == "=" else ("recurse",)
            elif c == "R" or c.isdigit() or c == "&" or (c in "+-" and (self.peek() or "").isdigit()):
                self.i -= 1
                self.name_until(")")
                return ("recurse",)
            else:
                # inline flags (?i) (?-i) (?i:...) (?x) etc.
                self.i -= 1
                j = self.i
                while self.peek() is not None and self.peek() not in ":)":
                    self.take()
                flags = self.s[j : self.i]
                on, _, off = flags.partition("-")
                if not all(f in "imsxnUJ^" for f in on + off):
                    raise ParseError(f"unsupported group (?{flags}")
                if "x" in on:
                    raise ParseError("(?x) not supported by prototype")
                newci = (self.ci[-1] or "i" in on) and "i" not in off
                if self.take() == ")":
                    self.ci[-1] = newci
                    return None
                self.ci.append(newci)
                n = self.alt()
                self.ci.pop()
                if self.take() != ")":
                    raise ParseError("unclosed group")
                return ("group", n, "noncap", None)
        if kind == "cap":
            self.ngroups += 1
        n = self.alt()
        if self.peek() != ")":
            raise ParseError("unclosed group")
        self.take()
        return ("group", n, kind, name)

    def escape(self, in_class):
        c = self.take()
        p = esc_pred(c)
        if p and not (in_class and c in "NRX"):
            return ("set", p, "\\" + c)
        if c in "pP":
            if self.peek() == "{":
                self.take()
                name = self.name_until("}")
            else:
                name = self.take()
            if c == "P":
                name = "^" + name
            return ("set", lambda x, n=name: prop(n, x), f"\\p{{{name}}}")
        if c == "x":
            if self.peek() == "{":
                self.take()
                ch = chr(int(self.name_until("}"), 16))
            else:
                h = self.s[self.i : self.i + 2]
                self.i += 2
                ch = chr(int(h, 16))
            return self.lit(ch)
        if c == "u" and self.peek() == "{":
            self.take()
            return self.lit(chr(int(self.name_until("}"), 16)))
        if c in SIMPLE and not (c == "0" and False):
            return self.lit(SIMPLE[c])
        if in_class:
            if c == "b":
                return self.lit("\b")
            return self.lit(c)
        if c in "bBAzZG":
            return ("assert", "\\" + c)
        if c == "K":
            return ("keep",)
        if c == "Q":
            j = self.s.find("\\E", self.i)
            j = len(self.s) if j < 0 else j
            lit = self.s[self.i : j]
            self.i = min(len(self.s), j + 2)
            return ("seq", [self.lit(ch) for ch in lit])
        if c == "E":
            return None
        if c.isdigit():
            while self.peek() and self.peek().isdigit():
                self.take()
            return ("backref",)
        if c in "kg":
            o = self.take()
            if o in "<{'":
                self.name_until({"<": ">", "{": "}", "'": "'"}[o])
            else:
                while self.peek() and (self.peek().isdigit() or self.peek() == "-"):
                    self.take()
            return ("recurse",) if (c == "g" and o in "<'") else ("backref",)
        return self.lit(c)

    def cls(self):
        neg = False
        if self.peek() == "^":
            self.take()
            neg = True
        preds = []
        first = True
        while True:
            c = self.peek()
            if c is None:
                raise ParseError("unclosed class")
            if c == "]" and not first:
                self.take()
                break
            first = False
            if c == "[" and self.peek(1) == ":":
                j = self.s.index(":]", self.i)
                name = self.s[self.i + 2 : j]
                self.i = j + 2
                pn = {"alpha": str.isalpha, "digit": str.isdigit, "space": str.isspace, "upper": str.isupper,
                      "lower": str.islower, "alnum": str.isalnum, "punct": lambda x: unicodedata.category(x).startswith("P"),
                      "word": is_word}.get(name.lstrip("^"), lambda x: False)
                preds.append(lambda x, f=pn: x != OTHER and f(x))
                continue
            self.take()
            if c == "\\":
                node = self.escape(in_class=True)
                if node is None:
                    continue
                if node[0] == "seq":
                    preds.extend(n[1] for n in node[1])
                    continue
                lo_pred, lo_desc = node[1], node[2]
                if len(lo_desc) == 1 and self.peek() == "-" and self.peek(1) not in ("]", None):
                    self.take()
                    hi = self.take()
                    if hi == "\\":
                        hi = self.escape(in_class=True)[2]
                    preds.append(lambda x, a=lo_desc, b=hi: x != OTHER and a <= x <= b)
                    self.lits.update([lo_desc, hi])
                    continue
                preds.append(lo_pred)
                continue
            if self.peek() == "-" and self.peek(1) not in ("]", None):
                self.take()
                hi = self.take()
                if hi == "\\":
                    hi = self.escape(in_class=True)[2]
                preds.append(lambda x, a=c, b=hi: x != OTHER and a <= x <= b)
                self.lits.update([c, hi])
                continue
            self.lits.add(c)
            preds.append(lambda x, ch=c: x == ch)
        base = lambda x, ps=tuple(preds): any(p(x) for p in ps)
        node = self.charnode(base, "[]")
        if neg:
            inner = node[1]
            return ("set", lambda x, f=inner: not f(x), "[^]")
        return node


# ---- analysis ----


class Analysis:
    def __init__(self, alphabet):
        self.A = alphabet
        self.hits = []

    def chars(self, n):
        """Characters n can consume (anywhere)."""
        t = n[0]
        if t == "set":
            return frozenset(c for c in self.A if n[1](c))
        if t in ("seq", "alt"):
            s = set()
            for k in n[1]:
                s |= self.chars(k)
            return frozenset(s)
        if t == "rep":
            return self.chars(n[1]) if n[3] != 0 else frozenset()
        if t == "group":
            return frozenset() if n[2] in ("la", "nla", "lb", "nlb") else self.chars(n[1])
        if t in ("backref", "recurse"):
            return frozenset(self.A)
        return frozenset()

    def nullable(self, n):
        t = n[0]
        if t == "set":
            return False
        if t == "seq":
            return all(self.nullable(k) for k in n[1])
        if t == "alt":
            return any(self.nullable(k) for k in n[1])
        if t == "rep":
            return n[2] == 0 or self.nullable(n[1])
        if t == "group":
            return True if n[2] in ("la", "nla", "lb", "nlb") else self.nullable(n[1])
        if t == "backref":
            return True
        return True

    def first(self, n):
        t = n[0]
        if t == "set":
            return self.chars(n)
        if t == "seq":
            s = set()
            for k in n[1]:
                s |= self.first(k)
                if not self.nullable(k):
                    break
            return frozenset(s)
        if t == "alt":
            s = set()
            for k in n[1]:
                s |= self.first(k)
            return frozenset(s)
        if t == "rep":
            return self.first(n[1]) if n[3] != 0 else frozenset()
        if t == "group":
            return frozenset() if n[2] in ("la", "nla", "lb", "nlb") else self.first(n[1])
        if t in ("backref", "recurse"):
            return frozenset(self.A)
        return frozenset()

    def unwrap(self, n):
        while n[0] == "group" and n[2] in ("cap", "noncap", "reset"):
            n = n[1]
        if n[0] == "seq" and len(n[1]) == 1:
            return self.unwrap(n[1][0])
        return n

    def inner_reps(self, n, out):
        """Unbounded reps inside n that can backtrack (not under atomic or possessive)."""
        t = n[0]
        if t == "rep":
            if n[4] == "possessive":
                return
            if n[3] is None and not self.nullable(n[1]) or (n[3] is None and self.chars(n[1])):
                out.append(n)
            self.inner_reps(n[1], out)
        elif t in ("seq", "alt"):
            for k in n[1]:
                self.inner_reps(k, out)
        elif t == "group" and n[2] != "atomic":
            self.inner_reps(n[1], out)

    def walk(self, n, under_rep=None, path=()):
        t = n[0]
        if t == "rep":
            body = n[1]
            atomic = n[4] == "possessive"
            big = n[3] is None or n[3] >= 3
            if not atomic and big:
                self.check_rep(n)
            for k in [body]:
                self.walk(k, n if n[3] is None or (n[3] or 0) > 1 else under_rep)
            return
        if t == "seq":
            self.check_seq(n)
            for k in n[1]:
                self.walk(k, under_rep)
            return
        if t == "alt":
            for k in n[1]:
                self.walk(k, under_rep)
            return
        if t == "group":
            if n[2] == "atomic":
                # Atomic group: inner backtracking cannot multiply with the outside.
                self.walk(n[1], None)
                return
            if n[2] in ("la", "nla", "lb", "nlb") and under_rep is not None:
                reps = []
                self.inner_reps(n[1], reps)
                if reps:
                    self.hit("LOOK-IN-REP", "lookaround with an unbounded quantifier inside a repeated group")
            if n[2] in ("lb", "nlb"):
                mx = self.max_len(n[1])
                if mx is None or mx > 32:
                    self.hit("LONG-LOOKBEHIND", f"lookbehind up to {mx} characters")
            self.walk(n[1], under_rep)
            return
        if t == "backref" and under_rep is not None:
            self.hit("BACKREF-IN-REP", "backreference inside a repeated group")
        if t == "recurse":
            self.hit("RECURSION", "recursion or subroutine call")
        if t == "keep" and under_rep is not None:
            self.hit("K-IN-REP", "\\K inside a repeated group")

    def max_len(self, n):
        t = n[0]
        if t == "set":
            return 1
        if t == "seq":
            s = 0
            for k in n[1]:
                m = self.max_len(k)
                if m is None:
                    return None
                s += m
            return s
        if t == "alt":
            ms = [self.max_len(k) for k in n[1]]
            return None if None in ms else max(ms, default=0)
        if t == "rep":
            m = self.max_len(n[1])
            return None if n[3] is None or m is None else m * n[3]
        if t == "group":
            return 0 if n[2] in ("la", "nla", "lb", "nlb") else self.max_len(n[1])
        if t in ("backref", "recurse"):
            return None
        return 0

    def check_rep(self, rep):
        body = self.unwrap(rep[1])
        unbounded = rep[3] is None
        # NEST: an inner backtracking unbounded quantifier whose characters no mandatory
        # sibling separates (star height 2 with ambiguity).
        items = body[1] if body[0] == "seq" else [body]
        inner = []
        for it in items:
            self.inner_reps(it, inner)
        for x in inner:
            xc = self.chars(x[1])
            separated = False
            for seq in self.seqs_on_path(rep[1], x):
                for it in seq[1]:
                    if self.contains(it, x) or self.nullable(it):
                        continue
                    if not (self.chars(it) & xc):
                        separated = True
                        break
                if separated:
                    break
            if not separated and xc:
                if unbounded:
                    self.hit("NEST", "nested unbounded quantifiers with no separator (exponential)")
                else:
                    self.hit("NEST-COUNTED", f"counted repeat {{,{rep[3]}}} over an unbounded quantifier (polynomial degree {rep[3]})")
                return
        # ALT: alternatives that can start with the same character, repeated.
        if body[0] == "alt" and unbounded:
            bs = body[1]
            for a in range(len(bs)):
                for b in range(a + 1, len(bs)):
                    if self.first(bs[a]) & self.first(bs[b]):
                        if self.chars(bs[a]) <= self.chars(bs[b]) or self.chars(bs[b]) <= self.chars(bs[a]):
                            self.hit("ALT-OVERLAP", "repeated alternation whose branches overlap (exponential)")
                            return
        # Nullable body under an unbounded repeat: (\s*)* style.
        if unbounded and self.nullable(rep[1]) and self.chars(rep[1]):
            self.hit("NULLABLE-REP", "unbounded repeat of a group that can match empty")

    def seqs_on_path(self, n, x):
        """Every seq node on the path from n down to x."""
        if n is x:
            return []
        t = n[0]
        kids = n[1] if t in ("seq", "alt") else ([n[1]] if t in ("rep", "group") else [])
        for k in kids:
            if self.contains(k, x):
                below = self.seqs_on_path(k, x)
                return ([n] if t == "seq" else []) + below
        return []

    def contains(self, n, x):
        if n is x:
            return True
        if n[0] in ("seq", "alt"):
            return any(self.contains(k, x) for k in n[1])
        if n[0] in ("rep", "group"):
            return self.contains(n[1], x)
        return False

    def check_seq(self, seq):
        # POLY: a chain of unbounded quantifiers over overlapping characters, where every
        # element between two links is nullable or overlaps both.
        items = seq[1]
        best = 0
        for i, it in enumerate(items):
            if not (it[0] == "rep" and it[3] is None and it[4] != "possessive"):
                continue
            chain = [self.chars(it[1])]
            for nxt in items[i + 1 :]:
                c = self.chars(nxt)
                if nxt[0] == "rep" and nxt[3] is None and nxt[4] != "possessive":
                    if c & chain[-1]:
                        chain.append(c)
                        continue
                    break
                if self.nullable(nxt) or (c and c <= chain[-1]):
                    continue
                break
            best = max(best, len(chain))
        if best >= 3:
            self.hit("POLY", f"{best} adjacent overlapping unbounded quantifiers (polynomial degree {best})")
        elif best == 2:
            self.hit("POLY2", "2 adjacent overlapping unbounded quantifiers (quadratic)")

    def hit(self, rule, why):
        if rule not in [h[0] for h in self.hits]:
            self.hits.append((rule, why))


REJECT = {"NEST", "NEST-COUNTED", "ALT-OVERLAP", "NULLABLE-REP", "LOOK-IN-REP", "BACKREF-IN-REP", "RECURSION", "K-IN-REP", "POLY"}


def check(source, caseless):
    p = Parser(source, caseless)
    tree = p.parse()
    a = Analysis(REPS + sorted(p.lits) + [OTHER])
    a.walk(tree)
    return a.hits


def main():
    as_json = "--json" in sys.argv
    for line in open(sys.argv[1]):
        r = json.loads(line)
        try:
            hits = check(r["source"], r.get("caseless", False))
            err = None
        except (ParseError, ValueError, IndexError) as e:
            hits, err = [], str(e)
        rules = [h[0] for h in hits]
        verdict = "parse-error" if err else ("flag" if set(rules) & REJECT else ("warn" if rules else "ok"))
        if as_json:
            print(json.dumps({"id": r["id"], "origin": r.get("origin"), "verdict": verdict, "rules": rules, "error": err}))
        else:
            print(f"{r['id']}\t{verdict}\t{','.join(rules)}\t{err or ''}")


if __name__ == "__main__":
    main()
