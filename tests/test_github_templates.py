"""Repository checks for the GitHub pull request template and issue form config."""

from __future__ import annotations

import re
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
TEMPLATE = ROOT / ".github" / "PULL_REQUEST_TEMPLATE.md"
CONFIG = ROOT / ".github" / "ISSUE_TEMPLATE" / "config.yml"
CONTRIBUTING = ROOT / "CONTRIBUTING.md"

TRACKER_KEY = "Bea" + "d"
ENGINEERING_DOC_URL = "https://github.com/slopvac/slopvac/blob/main/docs/engineering.md"
DEPARTURE_ANCHOR = "#how-to-request-a-departure"
# CONTRIBUTING.md, "Rust workspace" and "Rust checks".
CARGO_CHECKS = [
    "cargo fmt --all --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --workspace",
    "cargo test --doc --workspace",
    "prek run --all-files",
]
LAUNCHERS_2X = [
    "uvx --from ./packages/slopvac-lint",
    "uvx --from slopvac@latest",
    "slopvac@latest",
]
HEADING = re.compile(r"^(#{1,6}) ", re.MULTILINE)


def _text(path: Path) -> str:
    assert path.is_file(), f"missing {path.relative_to(ROOT)}"
    return path.read_text(encoding="utf-8")


def _section(text: str, title: str) -> str:
    """Return the body of the `## title` section, up to the next heading of the same or higher level."""
    heading = re.search(rf"^(#{{1,6}}) {re.escape(title)}$", text, re.MULTILINE)
    assert heading, f"CONTRIBUTING.md has no {title} section"
    level = len(heading.group(1))
    body = text[heading.end() :]
    for later in HEADING.finditer(body):
        if len(later.group(1)) <= level:
            return body[: later.start()]
    return body


def _sh_blocks(section: str) -> list[str]:
    return [m.strip() for m in re.findall(r"```sh\n(.+?)\n```", section, re.DOTALL)]


def _contributing_command(title: str, needle: str) -> str:
    blocks = [
        b for b in _sh_blocks(_section(_text(CONTRIBUTING), title)) if needle in b
    ]
    assert len(blocks) == 1, f"expected one sh block with {needle!r} under {title}"
    return blocks[0]


def _checks_rows() -> list[list[str]]:
    rows = []
    for line in _text(TEMPLATE).splitlines():
        if line.startswith("|"):
            rows.append([cell.strip() for cell in line.strip().strip("|").split("|")])
    return rows


def _row_starting(prefix: str) -> list[str]:
    rows = [row for row in _checks_rows() if row[0].startswith(prefix)]
    assert len(rows) == 1, (
        f"expected one checks row starting {prefix!r}, got {len(rows)}"
    )
    return rows[0]


def test_template_has_tracker_placeholder_line():
    assert re.search(rf"^{TRACKER_KEY}: <id>$", _text(TEMPLATE), re.MULTILINE)


def test_tracker_lines_are_optional_and_an_issue_link_works():
    text = _text(TEMPLATE)
    assert "optional" in text.lower()
    assert "Fixes #<n>" in text
    assert "Issue: <url>" in text


def test_template_reminds_title_is_conventional_commit():
    text = _text(TEMPLATE)
    assert "Conventional Commit" in text
    assert "type(scope): summary" in text


@pytest.mark.parametrize("check", CARGO_CHECKS, ids=lambda c: c.split()[1])
def test_template_names_each_cargo_check(check):
    assert _row_starting(f"`{check}`")[0] == f"`{check}`"


def test_template_pytest_row_uses_the_contributing_invocation():
    command = _contributing_command("Development setup", "pytest")
    assert f"`{command}`" in _row_starting("Tests")[0]


def test_template_gate_row_names_the_contributing_command():
    command = _contributing_command("Prose checks", "lint")
    assert f"`{command}`" in _row_starting("Prose gate")[0]


def test_engineering_doc_links_are_absolute():
    targets = [m[1] for m in re.findall(r"\[([^\]]*)\]\(([^)\s]+)\)", _text(TEMPLATE))]
    assert ENGINEERING_DOC_URL in targets
    assert ENGINEERING_DOC_URL + DEPARTURE_ANCHOR in targets
    assert all(t.startswith("https://") for t in targets)


@pytest.mark.parametrize("path", [TEMPLATE, CONFIG], ids=lambda p: p.name)
@pytest.mark.parametrize(
    "launcher", LAUNCHERS_2X, ids=lambda s: s.split()[0] + ":" + s.split()[-1]
)
def test_no_2x_launcher(path, launcher):
    """Guard on new content: neither file may send contributors to a 2.x launcher.

    The template is new, so this does not check that old code is gone.
    """
    assert launcher not in _text(path)


def test_config_contact_links_are_complete():
    config = yaml.safe_load(_text(CONFIG))
    assert config["blank_issues_enabled"] is False
    links = config["contact_links"]
    assert links
    for link in links:
        assert set(link) == {"name", "url", "about"}
        assert all(isinstance(v, str) and v for v in link.values())
        assert link["url"].startswith("https://")
