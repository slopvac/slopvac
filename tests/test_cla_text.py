"""CLA.md is the repository copy of the text that contributors sign through CLA Assistant.

CLA Assistant reads the agreement from a Gist. The Gist is the text that counts. CLA.md must stay
the same text, with the two lint markers as the only difference. These constants pin the Gist
revision that CLA.md was checked against, and the whole committed file, markers included.

Any change to the agreement text must update the Gist in the same pull request, plus GIST_REVISION,
GIST_SHA256 and CLA_FILE_SHA256. CLA Assistant then asks every contributor to sign again.
"""

from __future__ import annotations

import hashlib
from pathlib import Path

CLA = Path(__file__).resolve().parents[1] / "CLA.md"
GIST_URL = "https://gist.github.com/srobroek/5374c748726c6ec555de263ba96c15b4"
GIST_REVISION = "0b69776de600ba03bca2924f455107bd7a66f701"
GIST_SHA256 = "1f1a82331bcf185153658889db0c02504ad976172a343bd41eff250f7fbaa63b"
CLA_FILE_SHA256 = "ff859dde16bc21c88f1e1f2b72fd606de5f010006f1778f2b879421aaa30fe65"
DISABLE = "<!-- slopvac-disable -->"
ENABLE = "<!-- slopvac-enable -->"


def lines() -> list[str]:
    return CLA.read_bytes().decode("utf-8").splitlines(keepends=True)


def test_cla_md_without_its_two_marker_lines_is_the_gist_text():
    kept = [line for line in lines() if line.rstrip("\r\n") not in {DISABLE, ENABLE}]
    assert hashlib.sha256("".join(kept).encode("utf-8")).hexdigest() == GIST_SHA256


def test_cla_md_keeps_both_lint_markers_once():
    stripped = [line.rstrip("\r\n") for line in lines()]
    assert stripped.count(DISABLE) == 1
    assert stripped.count(ENABLE) == 1
    assert stripped.index(DISABLE) < stripped.index(ENABLE)


def test_the_attribution_line_comes_after_the_enable_marker():
    # The marked span is the legal text only. The attribution is ordinary prose that the gate lints.
    stripped = [line.rstrip("\r\n") for line in lines()]
    after = stripped[stripped.index(ENABLE) + 1 :]
    assert any(
        line.startswith("Adapted from the [Contributor Agreements 1.2]")
        for line in after
    )
    assert not any(
        line.startswith("Adapted from the")
        for line in stripped[: stripped.index(ENABLE)]
    )


def test_the_committed_file_is_the_pinned_one():
    # The Gist hash ignores the markers, so this one pins them too.
    assert hashlib.sha256(CLA.read_bytes()).hexdigest() == CLA_FILE_SHA256


def test_the_markers_wrap_exactly_the_legal_text():
    stripped = [line.rstrip("\r\n") for line in lines()]
    rule = stripped.index("---")
    assert stripped[0] == DISABLE
    assert stripped[rule - 2] == ENABLE
    assert stripped[rule - 1] == ""
    assert stripped.count("---") == 1
