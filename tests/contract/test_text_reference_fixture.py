"""Fail when the Rust text fixture no longer matches the Python reference.

`crates/slopvac-text/tests/fixtures/reference.json` stores inputs and the values the
Python implementation produced for them. If `slopvac.analyze` changes behaviour, this
test fails until the fixture is regenerated and the Rust port is brought in line.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

SCRIPT = (
    Path(__file__).resolve().parents[2]
    / "crates/slopvac-text/tests/fixtures/generate.py"
)


def test_text_reference_fixture_is_current() -> None:
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--check"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr or result.stdout
