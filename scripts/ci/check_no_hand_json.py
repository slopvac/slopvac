"""Keep the no-hand-json allow list closed, honest and shrink-only.

The allow list lives in the ``ignores`` of ast-grep/rules/no-hand-json.yml. Every entry there
carries an annotation, and this check rejects a line without one:

* ``# exempt glob``: a test, bench or example path from a fixed set;
* ``# exempt test-module``: an out-of-line file that its parent declares as
  ``#[cfg(test)] mod name;`` (the check reads the parent to confirm it);
* ``# allow <category> <count>``: an allow-list entry for one production file.

File checks (always run). The check strips the allow-list entries, runs the rule, and
compares what it finds with the list:

* every allow-listed file still has a finding, so an entry whose sites are gone must go;
* every pinned count equals the number of findings, so removing some sites needs a lower count;
* every category has exactly one ``# total <category> <n>`` line, and it equals the sum of
  the entries;
* a TEMPORARY category names a bead as ``TEMPORARY(slopvac-...)``;
* the ``ignores`` list that a YAML parser loads from the file equals the annotated entries, in
  order, so a sequence written in another style cannot add an entry;
* no finding sits outside the allow list. The rule runs on a copy of the sources in which every
  `ast-grep-ignore` marker is neutralised, so a suppression comment, in any form or position,
  cannot hide a finding from this check. (ast-grep 0.51.3 has no switch to turn suppression
  off, and a text classifier of comments misses forms such as a vertical tab or a bare CR inside
  a line comment, and nested block comments.) Another rule's suppression stays legal in the
  checkout, because only the copy is neutralised.

Growth check (``--base <ref>``). The file checks alone compare the file with itself, so raising
a pin along with a new ``json!`` would pass. With a base, the check reads the rule file at that
ref and fails on a new entry, a raised count or a raised total. A permanent growth passes only
when the pull request carries the label ``no-hand-json-growth`` (``--labels`` takes the JSON list
of label names), which a maintainer sets after contract sign-off. TEMPORARY growth never
passes. A base without the rule file has nothing to compare, so the growth check passes.

A maintainer applies the label ``no-hand-json-growth`` to approve a permanent growth, then
re-runs the checker job. Nothing in this repository creates, applies or writes labels. CI reads
the pull request's current labels while the job runs, so a label added after a run changes
nothing until the job runs again. TEMPORARY growth never passes, label or not.

CI runs the growth check on pull_request only. A merge_group event carries no labels, so a
label-approved growth would fail in the queue. The queue requires the pull request run, which
checked the growth at the reviewed head, so merge_group and push run the file checks alone.

Usage: python3 scripts/ci/check_no_hand_json.py [ROOT] [--base REF] [--labels JSON]
(needs ast-grep on PATH)
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path
from typing import NamedTuple

import yaml

RULE = Path("ast-grep") / "rules" / "no-hand-json.yml"
LABEL = "no-hand-json-growth"
PERMANENT = {"schema-generation", "config-leaf", "digest-input", "validator-fragment"}
TEMPORARY = re.compile(r"TEMPORARY\(slopvac-[a-z0-9]+(?:\.[a-z0-9]+)*\)")
EXEMPT_GLOBS = {"**/tests/**", "**/benches/**", "**/examples/**", "**/tests.rs"}
# An ignore entry: a path (bare or double quoted) and an optional trailing comment.
ENTRY_LINE = re.compile(r'^  - (?P<path>"[^"]+"|[^\s#"]\S*)(?:\s+#\s*(?P<note>.*?))?\s*$')
ALLOW = re.compile(r"allow (?P<category>\S+) (?P<count>\d+)")
EXEMPT = re.compile(r"exempt (?P<kind>glob|test-module)")
TOTAL = re.compile(r"^# total (?P<category>\S+) (?P<count>\d+)\s*$")


# The cfg predicates that put an item under test. The rule exempts an item only when its
# `#[cfg(...)]` predicate, read as boolean logic, is false whenever `test` is false: `test`,
# `all(...)` with any argument that requires `test`, and `any(...)` whose every argument does.
# `not(...)` never counts, and neither does `cfg_attr`. A regex cannot recurse, so this builds
# the pattern to a fixed nesting (REQUIRE_DEPTH levels of all/any, ARGUMENT_DEPTH levels inside
# an argument that is no `test`). A deeper predicate is not exempt, which is the safe side.
REQUIRE_DEPTH = 2
ARGUMENT_DEPTH = 2


def _argument(depth: int) -> str:
    if depth == 0:
        return r"[^(),]+"
    inner = _argument(depth - 1)
    return rf"[^(),]+(?:\(\s*{inner}(?:,{inner})*\s*\))?"


def _requires_test(depth: int) -> str:
    if depth == 0:
        return r"\s*test\s*"
    inner = _requires_test(depth - 1)
    other = _argument(ARGUMENT_DEPTH)
    conj = rf"\s*all\(\s*(?:{other},)*{inner}(?:,{other})*\s*\)\s*"
    disj = rf"\s*any\(\s*{inner}(?:,{inner})*\s*\)\s*"
    return rf"(?:\s*test\s*|{conj}|{disj})"


def cfg_test_regex() -> str:
    """The regex the rule uses on an attribute to tell a test-only `#[cfg(...)]`."""
    return rf"^#\[cfg\({_requires_test(REQUIRE_DEPTH)}\)\s*\]$"


class Entry(NamedTuple):
    path: str
    category: str
    count: int


class Rule(NamedTuple):
    """The parsed ignore list of the rule file."""

    entries: list[Entry]
    totals: dict[str, int]
    globs: list[str]
    test_modules: list[str]
    errors: list[str]


def is_temporary(category: str) -> bool:
    return TEMPORARY.fullmatch(category) is not None


class _UniqueKeyLoader(yaml.SafeLoader):
    """A safe loader that refuses a repeated mapping key, which PyYAML would resolve silently."""

    def construct_mapping(self, node: yaml.MappingNode, deep: bool = False) -> dict:
        seen: set[object] = set()
        for key_node, _ in node.value:
            key = self.construct_object(key_node, deep=deep)
            if key in seen:
                raise yaml.constructor.ConstructorError(
                    None, None, f"duplicate key {key!r}", key_node.start_mark
                )
            seen.add(key)
        return super().construct_mapping(node, deep)


def effective_ignores_problems(text: str, annotated: list[str]) -> list[str]:
    """Compare the ignore list ast-grep reads with the entries the annotation parser saw.

    The line parser only sees ``  - path`` lines under ``ignores:``. A YAML sequence can be
    written other ways (unindented, in flow style), and ast-grep reads all of them. The loaded
    list must equal the annotated entries, in order, so no entry escapes its annotation.
    """
    try:
        data = yaml.load(text, Loader=_UniqueKeyLoader)  # noqa: S506 - a safe loader subclass
    except yaml.YAMLError as error:
        return [f"the rule file is not valid YAML: {error}"]
    effective = data.get("ignores") if isinstance(data, dict) else None
    if effective == annotated:
        return []
    if not isinstance(effective, list):
        return ["'ignores' is not a list of paths"]
    extra = [path for path in effective if path not in annotated]
    missing = [path for path in annotated if path not in effective]
    detail = (
        f"extra {extra}, missing {missing}"
        if extra or missing
        else "same entries in a different order or count"
    )
    return [f"the ignores ast-grep reads differ from the annotated entries: {detail}"]


def parse(text: str) -> Rule:
    """Parse the ignore list strictly: an unannotated or odd line is an error."""
    entries: list[Entry] = []
    totals: dict[str, int] = {}
    globs: list[str] = []
    modules: list[str] = []
    errors: list[str] = []
    order: list[str] = []
    in_ignores = False
    ignores_seen = 0
    for number, line in enumerate(text.splitlines(), start=1):
        if re.match(r"^#\s*total\b", line):
            match = TOTAL.match(line)
            if match is None:
                errors.append(f"line {number}: malformed total line {line!r}")
            elif match["category"] in totals:
                errors.append(f"line {number}: duplicate total for {match['category']}")
            else:
                totals[match["category"]] = int(match["count"])
            continue
        if re.match(r"^ignores:\s*$", line):
            in_ignores = True
            ignores_seen += 1
            continue
        if line and not line[0].isspace() and not line.startswith("#"):
            in_ignores = False
        if not in_ignores or not line.strip() or line.lstrip().startswith("#"):
            continue
        match = ENTRY_LINE.match(line)
        if match is None:
            errors.append(f"line {number}: ignore entry is not '  - <path> # <annotation>'")
            continue
        path = match["path"].strip('"')
        order.append(path)
        note = match["note"] or ""
        if allow := ALLOW.fullmatch(note):
            category = allow["category"]
            if category not in PERMANENT and not is_temporary(category):
                errors.append(
                    f"{path}: unknown category {category!r}; use one of {sorted(PERMANENT)} "
                    "or TEMPORARY(slopvac-<bead>)"
                )
            entries.append(Entry(path, category, int(allow["count"])))
        elif exempt := EXEMPT.fullmatch(note):
            if exempt["kind"] == "glob":
                globs.append(path)
            else:
                modules.append(path)
        else:
            errors.append(
                f"{path}: ignore entry needs '# allow <category> <count>' or '# exempt "
                "glob|test-module'"
            )
    if ignores_seen != 1:
        errors.append(f"expected one 'ignores:' key, found {ignores_seen}")
    errors.extend(effective_ignores_problems(text, order))
    return Rule(entries, totals, globs, modules, errors)


def without_allow_list(text: str) -> str:
    """Return the rule text with every allow-list entry removed."""
    kept: list[str] = []
    in_ignores = False
    for line in text.splitlines(keepends=True):
        bare = line.rstrip("\n")
        if re.match(r"^ignores:\s*$", bare):
            in_ignores = True
        elif bare and not bare[0].isspace() and not bare.startswith("#"):
            in_ignores = False
        match = ENTRY_LINE.match(bare) if in_ignores else None
        if match and ALLOW.fullmatch(match["note"] or ""):
            continue
        kept.append(line)
    return "".join(kept)


def file_problems(rule: Rule, found: Counter[str]) -> list[str]:
    """Compare the allow list with the findings of the rule that has no allow list."""
    errors = list(rule.errors)
    sums: Counter[str] = Counter()
    seen: set[str] = set()
    for entry in rule.entries:
        if entry.path in seen:
            errors.append(f"{entry.path}: listed twice")
        seen.add(entry.path)
        sums[entry.category] += entry.count
        actual = found.get(entry.path, 0)
        if actual == 0:
            errors.append(f"{entry.path}: no finding left, delete the entry")
        elif actual != entry.count:
            errors.append(
                f"{entry.path}: pinned {entry.count} but the rule finds {actual}; "
                "update the count and the category total"
            )
    for category in sorted(set(sums) | set(rule.totals)):
        if sums[category] != rule.totals.get(category):
            errors.append(
                f"total {category}: declared {rule.totals.get(category)}, entries sum to "
                f"{sums[category]}"
            )
    for path in sorted(found):
        if path not in seen:
            errors.append(
                f"{path}: {found[path]} finding(s) outside the allow list; build the JSON from a "
                "typed model, or get contract sign-off for an allow-list entry"
            )
    for glob in rule.globs:
        if glob not in EXEMPT_GLOBS:
            errors.append(f"{glob}: not one of the fixed test globs {sorted(EXEMPT_GLOBS)}")
    return errors


def declares_test_module(root: Path, path: str) -> bool:
    """Whether the parent of ``path`` declares it as ``#[cfg(test)] mod name;``.

    The parent is parsed with ast-grep, so a commented-out attribute does not count: the
    declaration must be a `mod_item` with no body that follows a test-only `cfg` attribute,
    with only attributes and comments between them.
    """
    file = Path(path)
    if file.name in {"mod.rs", "lib.rs", "main.rs"}:
        return False
    folder = file.parent
    parents = [folder.with_suffix(".rs"), folder / "mod.rs", folder / "lib.rs", folder / "main.rs"]
    rule = {
        "id": "test-module-declaration",
        "language": "rust",
        "rule": {
            "kind": "mod_item",
            "has": {
                "field": "name",
                "kind": "identifier",
                "regex": f"^{re.escape(file.stem)}$",
            },
            "not": {"has": {"field": "body", "kind": "declaration_list"}},
            "follows": {
                "kind": "attribute_item",
                "regex": cfg_test_regex(),
                "stopBy": {
                    "not": {
                        "any": [
                            {"kind": "attribute_item"},
                            {"kind": "line_comment"},
                            {"kind": "block_comment"},
                        ]
                    }
                },
            },
        },
    }
    for parent in parents:
        if not (root / parent).is_file():
            continue
        done = subprocess.run(
            [
                "ast-grep",
                "scan",
                "--inline-rules",
                json.dumps(rule),
                "--json=compact",
                parent.as_posix(),
            ],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
        if done.returncode not in (0, 1):
            raise RuntimeError(f"ast-grep exited {done.returncode}: {done.stderr.strip()}")
        if done.stdout.strip() and json.loads(done.stdout):
            return True
    return False


def exempt_problems(root: Path, rule: Rule) -> list[str]:
    errors = [
        f"{path}: the parent does not declare it as '#[cfg(test)] mod {Path(path).stem};'"
        for path in rule.test_modules
        if not declares_test_module(root, path)
    ]
    for tests in sorted(root.glob("crates/*/src/**/tests.rs")):
        relative = tests.relative_to(root).as_posix()
        if not declares_test_module(root, relative):
            errors.append(
                f"{relative}: matches '**/tests.rs' but its parent does not declare it under "
                "cfg(test)"
            )
    return errors


class Growth(NamedTuple):
    permanent: list[str]
    temporary: list[str]


def growth(base: Rule, head: Rule) -> Growth:
    """List what ``head`` adds to ``base``: new entries, raised counts, raised totals."""
    permanent: list[str] = []
    temporary: list[str] = []
    old = {entry.path: entry for entry in base.entries}
    for entry in head.entries:
        was = old.get(entry.path)
        bucket = temporary if is_temporary(entry.category) else permanent
        if was is None:
            bucket.append(f"{entry.path}: new {entry.category} entry")
        elif was.category != entry.category:
            bucket.append(f"{entry.path}: category {was.category} -> {entry.category}")
        elif entry.count > was.count:
            bucket.append(f"{entry.path}: count raised {was.count} -> {entry.count}")
    for category, total in sorted(head.totals.items()):
        if total > base.totals.get(category, 0):
            bucket = temporary if is_temporary(category) else permanent
            bucket.append(f"total {category}: raised {base.totals.get(category, 0)} -> {total}")
    for kind, now, before in (
        ("glob", head.globs, base.globs),
        ("test module", head.test_modules, base.test_modules),
    ):
        permanent.extend(f"{path}: new exempt {kind}" for path in now if path not in before)
    return Growth(permanent, temporary)


def growth_problems(base: Rule, head: Rule, allow_growth: bool) -> list[str]:
    found = growth(base, head)
    errors = [f"TEMPORARY growth is never allowed: {item}" for item in found.temporary]
    if not allow_growth:
        errors += [
            f"allow list grows without the '{LABEL}' label: {item}" for item in found.permanent
        ]
    return errors


def has_label(labels_json: str | None) -> bool:
    try:
        names = json.loads(labels_json or "[]")
    except json.JSONDecodeError:
        return False
    return isinstance(names, list) and LABEL in names


# ast-grep honours a comment that holds `ast-grep-ignore` (above a line, at its end, or at the
# top of a file) and drops the findings it covers, so a scan that wants every finding must not
# see the marker. ast-grep 0.51.3 matches the lowercase spelling only, and has no switch to turn
# suppression off. The scan therefore runs on a copy in which every occurrence of the marker, in
# any casing, is replaced by text of the same length that ast-grep does not know. Neutralising
# more than ast-grep reads is safe, because the copy is used for this scan alone. The bytes are
# replaced in place, so everything else (CR, VT, nested comments, non-UTF-8 bytes) stays as
# written, and the file names stay the same.
MARKER = re.compile(rb"ast-grep-ignore", re.IGNORECASE)
INERT = b"ast-grep-inert_"
assert len(INERT) == len(b"ast-grep-ignore")  # noqa: S101 - a constant check at import


def neutralise(data: bytes) -> bytes:
    """Replace every occurrence of the suppression marker, keeping all other bytes."""
    return MARKER.sub(INERT, data)


def scanned_sources(root: Path) -> list[Path]:
    """The Rust files the rule scans, relative to ``root`` (its ``files`` glob, rust only)."""
    return sorted(
        path.relative_to(root) for path in root.glob("crates/*/src/**/*.rs") if path.is_file()
    )


def copy_neutralised(root: Path, dest: Path, rule_id: str, rule_text: str) -> None:
    """Copy what a scan of ``rule_id`` reads into ``dest``, with the marker neutralised.

    That is the sources, sgconfig.yml and the one rule file, at the same relative paths, so the
    rule's own ``ignores`` and ``files`` globs apply as they do in the checkout. Nothing is
    written under ``root``.
    """
    for relative in scanned_sources(root):
        target = dest / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(neutralise((root / relative).read_bytes()))
    config = root / "sgconfig.yml"
    if config.is_file():
        (dest / "sgconfig.yml").write_bytes(config.read_bytes())
    rule = dest / "ast-grep" / "rules" / f"{rule_id}.yml"
    rule.parent.mkdir(parents=True, exist_ok=True)
    rule.write_text(rule_text, encoding="utf-8")


def neutralised_scan(root: Path, rule_id: str, rule_text: str | None = None) -> Counter[str]:
    """Run one rule over a neutralised copy of the sources and count its findings by file.

    Built for any rule id, so the same scan can later guard `user-text-literal` against
    suppression comments (slopvac-8l4.21.25). Only no-hand-json is wired today. The copy lives
    in a temporary directory that is removed afterwards, also on an error. Paths in the result
    are relative to ``root``, as in the checkout.
    """
    if rule_text is None:
        rule_text = (root / "ast-grep" / "rules" / f"{rule_id}.yml").read_text(encoding="utf-8")
    with tempfile.TemporaryDirectory(prefix=f"{rule_id}-") as tmp:
        copy = Path(tmp)
        copy_neutralised(root, copy, rule_id, rule_text)
        done = subprocess.run(
            [
                "ast-grep",
                "scan",
                "--rule",
                f"ast-grep/rules/{rule_id}.yml",
                "--json=compact",
            ],
            cwd=copy,
            capture_output=True,
            text=True,
            check=False,
        )
    if done.returncode not in (0, 1):
        raise RuntimeError(f"ast-grep exited {done.returncode}: {done.stderr.strip()}")
    if not done.stdout.strip():
        return Counter()
    return Counter(item["file"] for item in json.loads(done.stdout))


def base_text(root: Path, ref: str) -> str | None:
    """Return the rule file at ``ref``, or None when the ref has no such file."""
    spec = f"{ref}:{RULE.as_posix()}"
    exists = subprocess.run(
        ["git", "cat-file", "-e", spec], cwd=root, capture_output=True, check=False
    )
    if exists.returncode != 0:
        probe = subprocess.run(
            ["git", "cat-file", "-e", f"{ref}^{{commit}}"],
            cwd=root,
            capture_output=True,
            check=False,
        )
        if probe.returncode != 0:
            raise RuntimeError(f"base {ref!r} is not a commit in this checkout; fetch it first")
        return None
    shown = subprocess.run(
        ["git", "show", spec], cwd=root, capture_output=True, text=True, check=True
    )
    return shown.stdout


def check(root: Path, base: str | None = None, labels: str | None = None) -> list[str]:
    text = (root / RULE).read_text()
    rule = parse(text)
    # A malformed list cannot be compared, so report its own errors first.
    errors = (
        list(rule.errors)
        if rule.errors
        else file_problems(rule, neutralised_scan(root, "no-hand-json", without_allow_list(text)))
    )
    errors += exempt_problems(root, rule)
    if base is not None:
        before = base_text(root, base)
        if before is not None:
            old = parse(before)
            errors += growth_problems(old, rule, has_label(labels))
    return errors


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("root", nargs="?", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--base", help="compare the allow list with the rule file at this ref")
    parser.add_argument("--labels", help="JSON list of the pull request's label names")
    parser.add_argument(
        "--print-cfg-regex", action="store_true", help="print the cfg(test) regex of the rule"
    )
    args = parser.parse_args(argv[1:])
    if args.print_cfg_regex:
        print(cfg_test_regex())
        return 0
    try:
        errors = check(args.root, args.base, args.labels)
    except RuntimeError as error:
        print(f"no-hand-json allow list: {error}", file=sys.stderr)
        return 2
    for error in errors:
        print(f"no-hand-json allow list: {error}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
