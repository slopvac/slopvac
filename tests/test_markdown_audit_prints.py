"""Every repository-derived line the Markdown audit prints reaches the log only escaped.

A tracked file name, a symlink alias, a link target or a line of document text can carry a
workflow command (`::error::`, `##[group]`) or a line break. The audit step runs through
the harness with such names in each printed list. For each print key, the printed line holds
no raw command, and its value, once unescaped, equals the expected value.
"""

from __future__ import annotations

import functools
import json
import re
import shutil
import sys
import tempfile
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from markdown_audit_harness import run_audit

CRLF_NAME = "docs/100%\r\n::stale::crlf.md"
EXCLUDED = "docs/::excluded::.md"
STALE = "docs/::stale::.md"
ALIASES = {
    "docs/::alias::link.md": "../README.md",
    "docs/##[group]alias.md": "../README.md",
}
LINKS = "docs/links.md"

# The stub reports a document for each name, where a real report holds one for each file.
_DEDUPE = (
    "seen = set(); "
    'report["documents"] = [d for d in report["documents"] '
    'if not ((r := __import__("os").path.realpath(d["path"])) in seen or seen.add(r))'
)

CASE = {
    "files": {
        EXCLUDED: "# Excluded\n",
        STALE: "three rulesets ::error::text\n",
        CRLF_NAME: "three rulesets\n",
        LINKS: "[a](::error::gone.md)\n\n[b](::add-mask::gone.md)\n",
    },
    "symlinks": ALIASES,
    # The configured scan leaves out the excluded document, as the root config does.
    "configured": _DEDUPE + ' and "excluded" not in d["path"]]',
    "all-files": _DEDUPE + "]",
    "authored": _DEDUPE + "]",
}

_TOKENS = {
    "%25": "%",
    "%0D": "\r",
    "%0A": "\n",
    "%3A%3A": "::",
    "\\\\": "\\",
    "\\u{23}": "#",
}


def _unescape(text: str) -> str:
    return re.sub(
        r"%25|%0D|%0A|%3A%3A|\\\\|\\u\{23\}", lambda m: _TOKENS[m.group()], text
    )


@functools.cache
def _audit_stdout() -> str:
    root = Path(tempfile.mkdtemp(prefix="audit-prints-"))
    try:
        try:
            (root / "a\r\nb").write_text("x")
        except OSError as err:
            pytest.skip(f"the filesystem refuses a name with CR and LF: {err}")
        done = run_audit(root, CASE)
        assert done.returncode == 0, done.stdout
        assert done.stderr == ""
        return done.stdout
    finally:
        shutil.rmtree(root, ignore_errors=True)


def _value(key: str) -> tuple[str, object]:
    (line,) = [ln for ln in _audit_stdout().splitlines() if ln.startswith(key + " ")]
    return line, json.loads(_unescape(line[len(key) + 1 :]))


# key: (expected value, raw payloads that must not appear, payload that must appear escaped)
_PRINTS = {
    "SYMLINK_ALIASES": (
        {"docs/::alias::link.md": "README.md", "docs/##[group]alias.md": "README.md"},
        ["::alias::", "##[group]"],
        "docs/%3A%3Aalias%3A%3Alink.md",
    ),
    "CONFIGURED_EXCLUSIONS": (
        ["CHANGELOG.md", EXCLUDED],
        ["::excluded::"],
        "docs/%3A%3Aexcluded%3A%3A.md",
    ),
    # A document missing from or unexpected in the full scan stops the audit before it
    # reaches these lines with a name in them, so a name cannot land in either list.
    "MISSING_FROM_FULL_AUDIT": ([], [], None),
    "UNEXPECTED_DOCUMENTS": ([], [], None),
    "LOCAL_LINK_AUDIT": (
        {
            "checked": 2,
            "external_not_checked": 0,
            "broken": [
                {"path": LINKS, "link": "::error::gone.md", "reason": "missing target"},
                {
                    "path": LINKS,
                    "link": "::add-mask::gone.md",
                    "reason": "missing target",
                },
            ],
        },
        ["::error::", "::add-mask::"],
        "%3A%3Aerror%3A%3Agone.md",
    ),
    "REVIEW_CANDIDATES": (
        [
            {"path": CRLF_NAME, "line": 1, "text": "three rulesets"},
            {"path": STALE, "line": 1, "text": "three rulesets ::error::text"},
        ],
        ["::stale::", "::error::"],
        "docs/%3A%3Astale%3A%3A.md",
    ),
}


@pytest.mark.parametrize("key", list(_PRINTS))
def test_a_printed_line_starts_no_workflow_command(key) -> None:
    line, _ = _value(key)
    _, raw, escaped = _PRINTS[key]
    assert not line.lstrip().startswith(("::", "##["))
    for payload in raw:
        assert payload not in line
    if escaped:
        assert escaped in line


@pytest.mark.parametrize("key", list(_PRINTS))
def test_a_printed_value_unescapes_to_the_expected_value(key) -> None:
    _, value = _value(key)
    assert value == _PRINTS[key][0]
