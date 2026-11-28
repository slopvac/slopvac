"""Every row of the policies.md section 4 conformance table has a marked Rust test.

The release gate's `conformance` item runs `test(/^conformance::/)`. A row counts only when a
test inside a `mod conformance { ... }` block carries the doc line `/// Conformance: <ID>`. The
row IDs come from the section 4 table itself, so a new row needs a marked test at once, and a
marker that names an ID the table does not have is stale.

The checks that read the repository and the checks of the checker itself (on trees built in
`tmp_path`) share one function, `check`.
"""

from __future__ import annotations

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


@dataclass(frozen=True)
class Marked:
    path: Path
    line: int
    test: str
    ids: tuple[str, ...]
    inside: bool


def _module_span(lines: list[str]) -> tuple[int, int] | None:
    """Zero-based line numbers of `mod conformance {` and of its closing brace, both in column 0."""
    for i, line in enumerate(lines):
        if line == _MODULE:
            for j in range(i + 1, len(lines)):
                if lines[j] == "}":
                    return i, j
            return i, len(lines)
    return None


def markers(root: Path) -> list[Marked]:
    """Every `/// Conformance:` doc line under `root/crates/*/tests`, with the test it sits on."""
    found: list[Marked] = []
    for path in sorted(root.glob("crates/*/tests/**/*.rs")):
        lines = path.read_text(encoding="utf-8").split("\n")
        span = _module_span(lines)
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
                    found.append(Marked(path, j + 1, fn["name"], ids, inside))
                    break
            else:
                found.append(Marked(path, i + 1, "<no test follows>", ids, False))
    return found


@dataclass
class Problems:
    rows_without_a_test: list[str] = field(default_factory=list)
    markers_naming_an_unknown_row: list[str] = field(default_factory=list)
    tests_outside_the_module: list[str] = field(default_factory=list)

    def any(self) -> bool:
        return bool(
            self.rows_without_a_test
            or self.markers_naming_an_unknown_row
            or self.tests_outside_the_module
        )


def check(root: Path, families: frozenset[str] = GATED_FAMILIES) -> Problems:
    """Compare the table in `root/docs/contract/policies.md` with the markers under `root`."""
    rows = table_rows(root / "docs/contract/policies.md")
    found = markers(root)
    problems = Problems()
    covered = {i for m in found if m.inside for i in m.ids}
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

_REAL_ROWS = [r for r in table_rows(REPO / "docs/contract/policies.md") if _family(r) in GATED_FAMILIES]


@pytest.mark.parametrize("row", _REAL_ROWS)
def test_the_row_has_a_test_marked_under_the_conformance_module(row: str) -> None:
    assert row not in check(REPO).rows_without_a_test, (
        f"no test under `mod conformance` carries `/// Conformance: {row}`"
    )


def test_every_marker_names_a_row_of_the_table() -> None:
    assert check(REPO).markers_naming_an_unknown_row == []


def test_every_marked_test_sits_in_the_conformance_module() -> None:
    assert check(REPO).tests_outside_the_module == []


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
