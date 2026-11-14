"""Fail when the Rust text crate's Unicode tables drift from Python's.

`crates/slopvac-text/src/unicode_data.rs` holds the `str.casefold` and `str.isdigit`
tables generated from the Python reference. The generator's `--check` mode compares
them with the running interpreter. It exits with status 77 on an interpreter whose
Unicode version differs from the pinned one, and this test skips there.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import pytest

SCRIPT = (
    Path(__file__).resolve().parents[2]
    / "crates/slopvac-text/tools/generate_unicode.py"
)
WRONG_VERSION_STATUS = 77


def test_text_unicode_tables_are_current() -> None:
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--check"],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode == WRONG_VERSION_STATUS:
        pytest.skip(result.stderr.strip())
    assert result.returncode == 0, result.stderr or result.stdout
