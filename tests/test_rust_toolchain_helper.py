"""The pinned Rust toolchain helper recovers from a partial install.

The script runs against a fake ``rustup`` on PATH that logs each call and fails
the first N ``toolchain install`` calls.
"""

from __future__ import annotations

import os
import re
import subprocess
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "ci" / "install-rust-toolchain.sh"
WORKFLOWS = ROOT / ".github" / "workflows"
HELPER = "scripts/ci/install-rust-toolchain.sh"

FAKE_RUSTUP = """#!/usr/bin/env bash
echo "$*" >> "$RUSTUP_LOG"
if [[ "$1 $2" == "toolchain install" ]]; then
    n=$(( $(cat "$RUSTUP_COUNT" 2>/dev/null || echo 0) + 1 ))
    echo "$n" > "$RUSTUP_COUNT"
    if (( n <= RUSTUP_FAIL_FIRST )); then
        echo "detected conflict bin/cargo-clippy" >&2
        exit 1
    fi
fi
exit 0
"""


def run_helper(tmp_path: Path, fail_first: int) -> tuple[int, list[str]]:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    fake = bin_dir / "rustup"
    fake.write_text(FAKE_RUSTUP)
    fake.chmod(0o755)
    repo = tmp_path / "repo"
    repo.mkdir()
    (repo / "rust-toolchain.toml").write_text(
        '[toolchain]\nchannel = "9.8.7"\ncomponents = ["clippy"]\n'
    )
    log = tmp_path / "log"
    env = {
        **os.environ,
        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
        "RUST_TOOLCHAIN_DIR": str(repo),
        "RUSTUP_LOG": str(log),
        "RUSTUP_COUNT": str(tmp_path / "count"),
        "RUSTUP_FAIL_FIRST": str(fail_first),
    }
    result = subprocess.run(
        ["bash", str(SCRIPT)], env=env, capture_output=True, text=True, check=False
    )
    calls = log.read_text().splitlines() if log.exists() else []
    return result.returncode, calls


def test_clean_install_does_not_uninstall(tmp_path: Path) -> None:
    code, calls = run_helper(tmp_path, fail_first=0)
    assert code == 0
    assert calls == ["toolchain install", "show active-toolchain"]


def test_partial_install_is_uninstalled_then_reinstalled(tmp_path: Path) -> None:
    code, calls = run_helper(tmp_path, fail_first=1)
    assert code == 0
    assert calls == [
        "toolchain install",
        "toolchain uninstall 9.8.7",
        "toolchain install",
        "show active-toolchain",
    ]


def test_failed_retry_exits_non_zero(tmp_path: Path) -> None:
    code, calls = run_helper(tmp_path, fail_first=2)
    assert code != 0
    assert "show active-toolchain" not in calls


# Jobs that name their own toolchain or install Rust through cargo-dist.
EXEMPT = {
    ("test.yml", "rust-msrv"),  # cargo "+$MSRV" only
    ("release.yml", "build-local-artifacts"),  # cargo-dist installs rustup
    ("release.yml", "build-global-artifacts"),
}
RUST_COMMAND = re.compile(r"(?:^|[\s;&|(])(?:cargo|rustup)\s", re.MULTILINE)


def _jobs() -> list[tuple[str, str, dict]]:
    found = []
    for path in sorted(WORKFLOWS.glob("*.yml")):
        workflow = yaml.safe_load(path.read_text())
        for name, job in (workflow.get("jobs") or {}).items():
            runs = [s.get("run", "") for s in job.get("steps", [])]
            if (path.name, name) not in EXEMPT and any(
                RUST_COMMAND.search(r) for r in runs
            ):
                found.append((path.name, name, job))
    return found


@pytest.mark.parametrize(
    ("file", "name", "job"), _jobs(), ids=lambda v: v if isinstance(v, str) else ""
)
def test_rust_jobs_use_the_toolchain_helper(file: str, name: str, job: dict) -> None:
    runs = [s.get("run", "") for s in job.get("steps", [])]
    helper_at = next((i for i, r in enumerate(runs) if HELPER in r), None)
    assert helper_at is not None, f"{file}:{name} runs cargo/rustup without the helper"
    first_rust = next(
        i for i, r in enumerate(runs) if RUST_COMMAND.search(r) and HELPER not in r
    )
    assert helper_at < first_rust, f"{file}:{name} installs after first use"
