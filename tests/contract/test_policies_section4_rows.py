"""Every row of the policies.md section 4 conformance table has a marked Rust test.

The release gate's `conformance` item runs `test(/^conformance::/)`. A row counts only when a
test inside a `mod conformance { ... }` block carries the doc line `/// Conformance: <ID>`. The
row IDs come from the section 4 table itself, so a new row needs a marked test at once, and a
marker that names an ID the table does not have is stale.

A marker counts only on a runnable test: a `#[test]` or `#[rstest]` function with no `#[ignore]`.
A test limited with `#[cfg(...)]` counts only when the cfg admits Linux, the OS of the gate: a
`target_os = "linux"` or `unix` cfg does, and any other `target_os = "..."`, `windows` or `macos`
cfg does not. Other cfg conditions, such as features, are not read. So a row whose only marked
tests are limited to another OS has no covered test, and the check fails.

The checks that read the repository and the checks of the checker itself (on trees built in
`tmp_path`) share one function, `check`.
"""

from __future__ import annotations

import os
import re
from dataclasses import dataclass, field
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]

# Row families whose tests are Rust tests that run under the gate. For now only the N rows are
# gated. The R, C, H and V rows have no marked Rust test yet, so they need no marker until a
# later change maps them.
GATED_FAMILIES = frozenset("N")

_SECTION = re.compile(r"^## 4\. Conformance tests$", re.MULTILINE)
_ROW = re.compile(r"^\|\s*([A-Z]+[0-9]+)\s*\|", re.MULTILINE)
_MARKER = re.compile(r"^\s*///\s*Conformance:(?P<rest>.*)$")
_ID = re.compile(r"[A-Z]+[0-9]+")
_FN = re.compile(r"^(?P<indent>\s*)(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(?P<name>\w+)")
_MODULE = "mod conformance {"
_RUNNABLE = re.compile(r"#\[(?:rstest(?:::rstest)?|test)\b")
_IGNORED = re.compile(r"#\[ignore\b")
_CFG_OS = re.compile(r'#\[cfg\(\s*target_os\s*=\s*"(?P<os>\w+)"\s*\)\]')
_CFG_FAMILY = re.compile(r"#\[cfg\(\s*(?P<family>unix|windows)\s*\)\]")


def _family(row: str) -> str:
    return row.rstrip("0123456789")


def table_rows(policies: Path) -> list[str]:
    """The row IDs of the section 4 table, in table order."""
    text = policies.read_text(encoding="utf-8")
    start = _SECTION.search(text)
    assert start, f"{policies} has no section 4 heading"
    rest = text[start.end() :]
    end = re.search(r"^## ", rest, re.MULTILINE)
    return _ROW.findall(rest[: end.start()] if end else rest)


def _admits_linux(attrs: str) -> bool:
    """False when a `cfg` on the test limits it to an OS other than Linux."""
    for found in _CFG_OS.finditer(attrs):
        if found["os"] != "linux":
            return False
    return all(f["family"] == "unix" for f in _CFG_FAMILY.finditer(attrs))


@dataclass(frozen=True)
class Marked:
    path: Path
    line: int
    test: str
    ids: tuple[str, ...]
    inside: bool
    runnable: bool = True
    on_linux: bool = True

    @property
    def counts(self) -> bool:
        return self.inside and self.runnable and self.on_linux


def _module_span(lines: list[str]) -> tuple[int, int] | None:
    """Zero-based line numbers of `mod conformance {` and of its closing brace, both in column 0."""
    for i, line in enumerate(lines):
        if line == _MODULE:
            for j in range(i + 1, len(lines)):
                if lines[j] == "}":
                    return i, j
            return i, len(lines)
    return None


def _scope_attrs(lines: list[str], span: tuple[int, int] | None) -> str:
    """The `#![cfg(..)]` lines of the file and the `#[cfg(..)]` lines on `mod conformance`."""
    found = [line for line in lines if line.startswith("#![cfg(")]
    if span is not None:
        k = span[0]
        while k > 0 and lines[k - 1].startswith("#["):
            k -= 1
            found.append(lines[k])
    return "\n".join(found).replace("#![", "#[")


def markers(root: Path) -> list[Marked]:
    """Every `/// Conformance:` doc line under `root/crates/*/tests`, with the test it sits on."""
    found: list[Marked] = []
    for path in sorted(root.glob("crates/*/tests/**/*.rs")):
        lines = path.read_text(encoding="utf-8").split("\n")
        span = _module_span(lines)
        scope_attrs = _scope_attrs(lines, span)
        for i, line in enumerate(lines):
            marker = _MARKER.match(line)
            if not marker:
                continue
            ids = tuple(_ID.findall(marker["rest"]))
            for j in range(i + 1, len(lines)):
                fn = _FN.match(lines[j])
                if fn:
                    inside = (
                        span is not None
                        and span[0] < j < span[1]
                        and fn["indent"] == "    "
                    )
                    attrs = "\n".join(lines[i + 1 : j])
                    cfg_attrs = scope_attrs + "\n" + attrs
                    runnable = bool(_RUNNABLE.search(attrs)) and not _IGNORED.search(attrs)
                    found.append(
                        Marked(path, j + 1, fn["name"], ids, inside, runnable, _admits_linux(cfg_attrs))
                    )
                    break
            else:
                found.append(Marked(path, i + 1, "<no test follows>", ids, False))
    return found


@dataclass
class Problems:
    rows_without_a_test: list[str] = field(default_factory=list)
    markers_naming_an_unknown_row: list[str] = field(default_factory=list)
    tests_outside_the_module: list[str] = field(default_factory=list)
    table_without_gated_rows: bool = False

    def any(self) -> bool:
        return bool(
            self.rows_without_a_test
            or self.markers_naming_an_unknown_row
            or self.tests_outside_the_module
            or self.table_without_gated_rows
        )


def check(root: Path, families: frozenset[str] = GATED_FAMILIES) -> Problems:
    """Compare the table in `root/docs/contract/policies.md` with the markers under `root`."""
    rows = table_rows(root / "docs/contract/policies.md")
    found = markers(root)
    problems = Problems()
    covered = {i for m in found if m.counts for i in m.ids}
    problems.table_without_gated_rows = not any(_family(r) in families for r in rows)
    for row in rows:
        if _family(row) in families and row not in covered:
            problems.rows_without_a_test.append(row)
    for m in found:
        where = f"{m.path.relative_to(root)}:{m.line} {m.test}"
        unknown = [i for i in m.ids if i not in rows] or ([] if m.ids else ["(no ID)"])
        if unknown:
            problems.markers_naming_an_unknown_row.append(f"{where}: {', '.join(unknown)}")
        if not m.inside:
            problems.tests_outside_the_module.append(where)
    return problems


# --- the repository ---------------------------------------------------------------------------

_REAL_ROWS = [
    r for r in table_rows(REPO / "docs/contract/policies.md") if _family(r) in GATED_FAMILIES
]


def test_the_table_parsed_into_gated_rows() -> None:
    assert _REAL_ROWS, "the section 4 table gave no N rows"
    assert not check(REPO).table_without_gated_rows


@pytest.mark.parametrize("row", _REAL_ROWS)
def test_the_row_has_a_test_marked_under_the_conformance_module(row: str) -> None:
    assert row not in check(REPO).rows_without_a_test, (
        f"no test under `mod conformance` carries `/// Conformance: {row}`"
    )


def test_every_marker_names_a_row_of_the_table() -> None:
    assert check(REPO).markers_naming_an_unknown_row == []


def test_every_marked_test_sits_in_the_conformance_module() -> None:
    assert check(REPO).tests_outside_the_module == []


def unlisted(found: list[Marked], listing: str, root: Path) -> list[str]:
    return []  # STUB(8l4.13.93): replaced in the next commit


@pytest.mark.skipif(
    not os.environ.get("SLOPVAC_CONFORMANCE_LIST"),
    reason="set SLOPVAC_CONFORMANCE_LIST to the output of `cargo nextest list` for the gate filter",
)
def test_every_counted_test_is_in_the_nextest_list_of_the_gate() -> None:
    """The pytest reads the source; nextest knows what the gate runs. They must agree."""
    listed = Path(os.environ["SLOPVAC_CONFORMANCE_LIST"]).read_text(encoding="utf-8")
    missing = [
        f"{m.path.relative_to(REPO)}:{m.line} {m.test}"
        for m in markers(REPO)
        if m.counts and f"conformance::{m.test}" not in listed
    ]
    assert missing == []


# --- the checker, on trees built here ---------------------------------------------------------


def _tree(tmp_path: Path, rows: list[str], source: str) -> Path:
    policies = tmp_path / "docs/contract/policies.md"
    policies.parent.mkdir(parents=True)
    table = "\n".join(f"| {row} | A row. | `bead` |" for row in rows)
    policies.write_text(
        f"## 3. Other\n\n| X9 | not a section 4 row |\n\n## 4. Conformance tests\n\n"
        f"| ID | Test | Owner |\n|---|---|---|\n{table}\n\n## 5. Follow-up\n\n| N77 | no |\n",
        encoding="utf-8",
    )
    tests = tmp_path / "crates/demo/tests"
    tests.mkdir(parents=True)
    (tests / "demo.rs").write_text(source, encoding="utf-8")
    return tmp_path


_MARKED = """\
mod conformance {
    /// Conformance: N1
    #[test]
    fn first() {}

    /// Conformance: N2, N3
    #[rstest]
    #[case::a(
        1
    )]
    fn second_and_third() {}
}
"""


def test_a_clean_tree_has_no_problems(tmp_path: Path) -> None:
    root = _tree(tmp_path, ["N1", "N2", "N3"], _MARKED)
    assert not check(root).any()


def test_a_row_with_no_marked_test_is_reported(tmp_path: Path) -> None:
    root = _tree(tmp_path, ["N1", "N2", "N3", "N4"], _MARKED)
    assert check(root).rows_without_a_test == ["N4"]


def test_a_marker_naming_an_unknown_row_is_reported(tmp_path: Path) -> None:
    source = _MARKED.replace("N2, N3", "N2, N99")
    root = _tree(tmp_path, ["N1", "N2", "N3"], source)
    problems = check(root)
    assert problems.markers_naming_an_unknown_row == [
        "crates/demo/tests/demo.rs:11 second_and_third: N99"
    ]
    assert problems.rows_without_a_test == ["N3"]


def test_a_marker_with_no_row_id_is_reported(tmp_path: Path) -> None:
    source = _MARKED.replace("N2, N3", "soon")
    root = _tree(tmp_path, ["N1", "N2", "N3"], source)
    assert check(root).markers_naming_an_unknown_row == [
        "crates/demo/tests/demo.rs:11 second_and_third: (no ID)"
    ]


def test_a_marked_test_outside_the_conformance_module_is_reported(tmp_path: Path) -> None:
    source = _MARKED + "\n/// Conformance: N4\n#[test]\nfn outside() {}\n"
    root = _tree(tmp_path, ["N1", "N2", "N3", "N4"], source)
    problems = check(root)
    assert problems.tests_outside_the_module == ["crates/demo/tests/demo.rs:16 outside"]
    assert problems.rows_without_a_test == ["N4"]


def test_a_row_outside_the_gated_families_needs_no_test(tmp_path: Path) -> None:
    root = _tree(tmp_path, ["N1", "N2", "N3", "R1"], _MARKED)
    assert not check(root).any()


def test_only_the_section_4_table_names_rows(tmp_path: Path) -> None:
    root = _tree(tmp_path, ["N1"], "")
    assert table_rows(root / "docs/contract/policies.md") == ["N1"]


# --- what counts as a runnable test -----------------------------------------------------------


def _one_test(attrs: str, table: list[str] | None = None, tmp: Path | None = None) -> Problems:
    source = f"mod conformance {{\n    /// Conformance: N1\n{attrs}    fn only() {{}}\n}}\n"
    assert tmp is not None
    return check(_tree(tmp, table or ["N1"], source))


def test_a_marked_helper_that_is_not_a_test_does_not_count(tmp_path: Path) -> None:
    assert _one_test("", tmp=tmp_path).rows_without_a_test == ["N1"]


def test_a_marked_ignored_test_does_not_count(tmp_path: Path) -> None:
    attrs = "    #[test]\n    #[ignore]\n"
    assert _one_test(attrs, tmp=tmp_path).rows_without_a_test == ["N1"]


@pytest.mark.parametrize("attrs", ["    #[test]\n", "    #[rstest]\n    #[case::a(1)]\n"])
def test_a_marked_test_or_rstest_counts(tmp_path: Path, attrs: str) -> None:
    assert not _one_test(attrs, tmp=tmp_path).any()


@pytest.mark.parametrize("os_name", ["linux", "unix"])
def test_a_test_limited_to_linux_counts(tmp_path: Path, os_name: str) -> None:
    cfg = f'target_os = "{os_name}"' if os_name == "linux" else "unix"
    attrs = f"    #[cfg({cfg})]\n    #[test]\n"
    assert not _one_test(attrs, tmp=tmp_path).any()


@pytest.mark.parametrize("cfg", ['target_os = "macos"', "windows"])
def test_a_test_limited_to_another_os_does_not_count(tmp_path: Path, cfg: str) -> None:
    attrs = f"    #[cfg({cfg})]\n    #[test]\n"
    assert _one_test(attrs, tmp=tmp_path).rows_without_a_test == ["N1"]


def _tree_with(tmp: Path, source: str) -> Problems:
    return check(_tree(tmp, ["N1"], source))


@pytest.mark.parametrize(
    "source",
    [
        '#![cfg(target_os = "macos")]\n\nmod conformance {\n    /// Conformance: N1\n    #[test]\n    fn t() {}\n}\n',
        '#[cfg(target_os = "macos")]\nmod conformance {\n    /// Conformance: N1\n    #[test]\n    fn t() {}\n}\n',
    ],
    ids=["file-level", "on-the-module"],
)
def test_a_module_limited_to_another_os_does_not_count(tmp_path: Path, source: str) -> None:
    assert _tree_with(tmp_path, source).rows_without_a_test == ["N1"]


def test_a_module_limited_to_linux_counts(tmp_path: Path) -> None:
    source = '#![cfg(target_os = "linux")]\n\nmod conformance {\n    /// Conformance: N1\n    #[test]\n    fn t() {}\n}\n'
    assert not _tree_with(tmp_path, source).any()


@pytest.mark.parametrize(
    "cfg",
    [
        'not(target_os = "linux")',
        'all(unix, not(target_os = "linux"))',
        'any(target_os = "macos", windows)',
        'all(target_os = "linux", unix)',
        'feature = "extra"',
    ],
    ids=["not-linux", "all-with-not", "any-of-others", "all-of-linux-and-unix", "feature"],
)
def test_a_cfg_the_checker_cannot_read_exactly_does_not_count(tmp_path: Path, cfg: str) -> None:
    attrs = f"    #[cfg({cfg})]\n    #[test]\n"
    assert _one_test(attrs, tmp=tmp_path).rows_without_a_test == ["N1"]


def test_a_module_that_is_not_linux_does_not_count(tmp_path: Path) -> None:
    source = (
        '#[cfg(not(target_os = "linux"))]\n'
        "mod conformance {\n    /// Conformance: N1\n    #[test]\n    fn t() {}\n}\n"
    )
    assert _tree_with(tmp_path, source).rows_without_a_test == ["N1"]


# --- the nextest inventory --------------------------------------------------------------------


def _unlisted(tmp: Path, listing: str) -> list[str]:
    root = _tree(tmp, ["N1", "N2", "N3"], _MARKED)
    return unlisted(markers(root), listing, root)


def test_a_marked_test_listed_under_its_own_binary_is_found(tmp_path: Path) -> None:
    listing = (
        "demo::demo conformance::first\n"
        "demo::demo conformance::second_and_third::case_1_a\n"
    )
    assert _unlisted(tmp_path, listing) == []


def test_a_marked_test_listed_only_under_another_binary_is_missing(tmp_path: Path) -> None:
    listing = (
        "demo::other conformance::first\n"
        "demo::demo conformance::second_and_third::case_1_a\n"
    )
    assert _unlisted(tmp_path, listing) == ["crates/demo/tests/demo.rs:4 first"]


def test_a_marked_test_missing_from_the_list_is_reported(tmp_path: Path) -> None:
    assert _unlisted(tmp_path, "demo::demo conformance::first\n") == [
        "crates/demo/tests/demo.rs:11 second_and_third"
    ]


# --- a table that did not parse ---------------------------------------------------------------


def test_a_table_with_no_rows_is_a_problem(tmp_path: Path) -> None:
    assert check(_tree(tmp_path, [], "")).any()


def test_a_table_with_no_gated_rows_is_a_problem(tmp_path: Path) -> None:
    assert check(_tree(tmp_path, ["R1"], "")).any()
