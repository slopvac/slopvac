"""CLA.md is the repository copy of the text that contributors sign through CLA Assistant.

CLA Assistant reads the agreement from a Gist. The Gist is the text that counts. CLA.md must stay
the same text, with the two lint markers as the only difference. These constants pin the Gist
revision that CLA.md was checked against.

Any change to the agreement text must update the Gist in the same pull request, and these two
constants. CLA Assistant then asks every contributor to sign again.
"""

from __future__ import annotations

import hashlib
from pathlib import Path

CLA = Path(__file__).resolve().parents[1] / "CLA.md"
GIST_URL = "https://gist.github.com/srobroek/5374c748726c6ec555de263ba96c15b4"
GIST_REVISION = "17e6bf102d8c79e62b1534e5e65f03af09504cdb"
GIST_SHA256 = "d97e3c15f29b3a69c912780eaeea07f8369cb1e3c7fd8941712cd49190a8f83a"
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
