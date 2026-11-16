"""Fail when the slopvac-engine parity fixture drifts from the Python reference.

STUB(slopvac-8l4.3.29): `crates/slopvac-engine/tests/fixtures/engine-parity.json` records what
the 2.x native engine and scorer report for a frozen corpus. If a 2.x rule, the engine or the
scorer changes, this test fails until the fixture is regenerated, and the Rust parity test then
tells whether `slopvac-engine` still agrees.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

SCRIPT = (
    Path(__file__).resolve().parents[2]
    / "crates/slopvac-engine/tests/fixtures/generate.py"
)


def test_engine_parity_fixture_is_current() -> None:
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--check"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr or result.stdout
