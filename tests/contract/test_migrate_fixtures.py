"""Fail when slopvac-migrate's 2.x mapping data drifts from the contract.

`crates/slopvac-migrate` bundles the rename table, the `core:legacy-<profile>` styles,
the 2.x rule catalogue and the `--preserve-behavior` vectors. Each is generated from
`docs/contract/compat/profiles/`, the 2.x ruleset, the rule converter and
`tests/contract/preserve_expansion.py`. This test fails until the files are regenerated
with `crates/slopvac-migrate/tests/fixtures/generate.py`.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

SCRIPT = (
    Path(__file__).resolve().parents[2]
    / "crates/slopvac-migrate/tests/fixtures/generate.py"
)


def test_migrate_mapping_data_is_current() -> None:
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--check"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr or result.stdout
