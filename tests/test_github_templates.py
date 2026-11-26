"""Repository checks for the GitHub pull request template and issue form config."""

from __future__ import annotations

import re
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
TEMPLATE = ROOT / ".github" / "PULL_REQUEST_TEMPLATE.md"
CONFIG = ROOT / ".github" / "ISSUE_TEMPLATE" / "config.yml"

TRACKER_KEY = "Bea" + "d"
# CONTRIBUTING.md, "Rust workspace" and "Rust checks", plus the test suite.
CHECKS = [
    "cargo fmt --all --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --workspace",
    "cargo test --doc --workspace",
    "prek run --all-files",
    "pytest",
]
LAUNCHERS_2X = [
    "uvx --from ./packages/slopvac-lint",
    "uvx --from slopvac@latest",
    "slopvac@latest",
]


def _text(path: Path) -> str:
    assert path.is_file(), f"missing {path.relative_to(ROOT)}"
    return path.read_text(encoding="utf-8")


def test_template_has_tracker_line():
    assert re.search(rf"^{TRACKER_KEY}: \S", _text(TEMPLATE), re.MULTILINE)


def test_template_reminds_title_is_conventional_commit():
    text = _text(TEMPLATE)
    assert "Conventional Commit" in text
    assert "type(scope): summary" in text


@pytest.mark.parametrize("check", CHECKS)
def test_template_names_each_contributing_check(check):
    assert f"`{check}`" in _text(TEMPLATE)


def test_template_names_prose_gate_and_engineering_doc():
    text = _text(TEMPLATE)
    assert "Prose gate" in text
    assert "docs/engineering.md" in text


@pytest.mark.parametrize("path", [TEMPLATE, CONFIG], ids=lambda p: p.name)
@pytest.mark.parametrize("launcher", LAUNCHERS_2X)
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
