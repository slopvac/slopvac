"""Fail when the slopvac-engine fix fixture drifts from the Python reference.

`crates/slopvac-engine/tests/fixtures/fix-parity.json` holds the text that 2.x `--fix` writes for
a frozen corpus. If a 2.x rule or the fix code changes, this test fails until the fixture is
regenerated, and the Rust fix test then tells whether `slopvac-engine` still agrees.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

SCRIPT = (
    Path(__file__).resolve().parents[2]
    / "crates/slopvac-engine/tests/fixtures/generate_fix.py"
)


def test_engine_fix_fixture_is_current() -> None:
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--check"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr or result.stdout
