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
* a TEMPORARY category names a bead as ``TEMPORARY(slopvac-...)``.

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


def parse(text: str) -> Rule:
    """Parse the ignore list strictly: an unannotated or odd line is an error."""
    entries: list[Entry] = []
    totals: dict[str, int] = {}
    globs: list[str] = []
    modules: list[str] = []
    errors: list[str] = []
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
    for glob in rule.globs:
        if glob not in EXEMPT_GLOBS:
            errors.append(f"{glob}: not one of the fixed test globs {sorted(EXEMPT_GLOBS)}")
    return errors


def declares_test_module(root: Path, path: str) -> bool:
    """Whether the parent of ``path`` declares it as ``#[cfg(test)] mod name;``."""
    file = Path(path)
    name = file.stem
    folder = file.parent
    parents = [folder.with_suffix(".rs"), folder / "mod.rs", folder / "lib.rs", folder / "main.rs"]
    if file.name in {"mod.rs", "lib.rs", "main.rs"}:
        return False
    declaration = re.compile(
        rf"#\[cfg\((all\()?([^()]*,\s*)?test\b[,)][^\n]*\n(\s*(#\[[^\n]*\]|//[^\n]*)\n)*"
        rf"\s*(pub(\([^)]*\))? )?mod {re.escape(name)};"
    )
    for parent in parents:
        candidate = root / parent
        if candidate.is_file() and declaration.search(candidate.read_text()):
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


def scan(root: Path, rule_text: str) -> Counter[str]:
    """Run the rule from ``root`` and count its findings by file."""
    with tempfile.TemporaryDirectory() as tmp:
        rule = Path(tmp) / "no-hand-json.yml"
        rule.write_text(rule_text)
        done = subprocess.run(
            ["ast-grep", "scan", "--rule", str(rule), "--json=compact"],
            cwd=root,
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
        else file_problems(rule, scan(root, without_allow_list(text)))
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
    args = parser.parse_args(argv[1:])
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
