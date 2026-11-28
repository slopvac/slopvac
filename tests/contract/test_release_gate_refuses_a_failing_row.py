"""The release gate's conformance command refuses a failing binary-level row.

The fixture package under `tests/fixtures/gate_conformance` holds one failing and one passing
test, both under `mod conformance` in an integration test binary, as the binary-level rows of
policies.md section 4 are. The test runs the exact trusted argv of the `conformance` item on a
scratch copy of that package. The item passes only on exit 0, so a non-zero exit here is the
gate refusing.
"""

from __future__ import annotations

import importlib.util
import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
FIXTURE = REPO / "tests/fixtures/gate_conformance"

SPEC = importlib.util.spec_from_file_location("release_gate", REPO / "scripts/release_gate.py")
assert SPEC is not None and SPEC.loader is not None
gate = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = gate
SPEC.loader.exec_module(gate)


def _has_nextest() -> bool:
    if not shutil.which("cargo"):
        return False
    done = subprocess.run(
        ["cargo", "nextest", "--version"], capture_output=True, text=True, check=False
    )
    return done.returncode == 0


needs_nextest = pytest.mark.skipif(
    not _has_nextest() and not os.environ.get("SLOPVAC_REQUIRE_NEXTEST"),
    reason="cargo-nextest is not installed (CI sets SLOPVAC_REQUIRE_NEXTEST)",
)


@needs_nextest
def test_the_conformance_command_selects_a_binary_level_row_and_exits_non_zero(
    tmp_path: Path,
) -> None:
    package = tmp_path / "package"
    shutil.copytree(FIXTURE, package)
    env = {**os.environ, "CARGO_TARGET_DIR": str(tmp_path / "target")}
    argv = gate.TRUSTED_COMMANDS["conformance"]

    done = subprocess.run(
        list(argv), cwd=package, env=env, capture_output=True, text=True, check=False
    )

    output = done.stdout + done.stderr
    assert "conformance::n98_a_seeded_row_fails" in output, output
    assert "conformance::n98_a_seeded_row_passes" in output, output
    assert done.returncode != 0, output
