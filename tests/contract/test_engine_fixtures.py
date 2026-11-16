"""Fail when a slopvac-engine reference fixture drifts from the Python reference.

STUB(slopvac-8l4.3.44): the golden suite records no vocabulary findings and no locale input,
so the engine keeps these fixtures until it does. Each generator's `--check` recomputes what it
records (the vocabulary one replays the stored Vale results when no usable Vale is installed)
and fails when the stored file differs. The Rust tests then tell whether `slopvac-engine`
still agrees.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import pytest

FIXTURES = Path(__file__).resolve().parents[2] / "crates/slopvac-engine/tests/fixtures"


@pytest.mark.parametrize(
    "script", ["generate_vocabulary.py", "generate_fix.py", "generate_locale.py"]
)
def test_engine_fixture_is_current(script: str) -> None:
    result = subprocess.run(
        [sys.executable, str(FIXTURES / script), "--check"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr or result.stdout
