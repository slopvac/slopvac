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


DEFAULT_TOML = '[toolchain]\nchannel = "9.8.7"\ncomponents = ["clippy"]\n'


def run_helper(
    tmp_path: Path, fail_first: int, toml: str = DEFAULT_TOML
) -> tuple[int, list[str]]:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    fake = bin_dir / "rustup"
    fake.write_text(FAKE_RUSTUP)
    fake.chmod(0o755)
    repo = tmp_path / "repo"
    repo.mkdir()
    (repo / "rust-toolchain.toml").write_text(toml)
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


@pytest.mark.parametrize(
    "toml",
    [
        '[toolchain]\nchannel = "9.8.7"\n',
        "[toolchain]\nchannel = '9.8.7'\n",
        '[toolchain]\nchannel = "9.8.7" # pinned\n',
        '[toolchain]\n  channel   =   "9.8.7"  \ncomponents = ["clippy"]\n',
    ],
    ids=["double", "single", "comment", "whitespace"],
)
def test_channel_is_parsed_as_toml(tmp_path: Path, toml: str) -> None:
    code, calls = run_helper(tmp_path, fail_first=1, toml=toml)
    assert code == 0
    assert "toolchain uninstall 9.8.7" in calls


def test_missing_channel_fails_before_rustup(tmp_path: Path) -> None:
    code, calls = run_helper(tmp_path, fail_first=0, toml="[toolchain]\n")
    assert code != 0
    assert calls == []


# Jobs that name their own toolchain or install Rust through cargo-dist.
EXEMPT = {
    ("test.yml", "rust-msrv"),  # cargo "+$MSRV" only
    ("release.yml", "build-local-artifacts"),  # cargo-dist installs rustup
    ("release.yml", "build-global-artifacts"),
}
# Jobs whose Rust use is indirect: the step that triggers it must follow the helper.
INDIRECT = {
    ("test.yml", "test"): re.compile(r"\bpytest\b"),  # Full test suite
    ("lint.yml", "prek"): re.compile(r"\bprek\b"),  # Rust hooks
}
RUST_COMMAND = re.compile(r"(?:^|[\s;&|(])(?:cargo|rustup|rustc|rustfmt)\s")


def _first_use(
    runs: list[str], indirect: re.Pattern[str] | None
) -> tuple[int, int] | None:
    """Position (step, line) of the first Rust use outside the helper line."""
    for step, run in enumerate(runs):
        for line_no, line in enumerate(run.splitlines()):
            if HELPER in line or line.lstrip().startswith("#"):
                continue
            if RUST_COMMAND.search(line) or (indirect and indirect.search(line)):
                return step, line_no
    return None


def _helper_at(runs: list[str]) -> tuple[int, int] | None:
    for step, run in enumerate(runs):
        for line_no, line in enumerate(run.splitlines()):
            if HELPER in line and not line.lstrip().startswith("#"):
                return step, line_no
    return None


def _runs(job: dict) -> list[str]:
    return [s.get("run", "") for s in job.get("steps", [])]


def _jobs() -> list[tuple[str, str, dict]]:
    found = []
    for path in sorted(WORKFLOWS.glob("*.yml")):
        workflow = yaml.safe_load(path.read_text())
        for name, job in (workflow.get("jobs") or {}).items():
            key = (path.name, name)
            if key not in EXEMPT and _first_use(_runs(job), INDIRECT.get(key)):
                found.append((path.name, name, job))
    return found


def test_indirect_jobs_exist() -> None:
    found = {(f, n) for f, n, _ in _jobs()}
    assert set(INDIRECT) <= found


@pytest.mark.parametrize(
    ("file", "name", "job"), _jobs(), ids=lambda v: v if isinstance(v, str) else ""
)
def test_rust_jobs_use_the_toolchain_helper(file: str, name: str, job: dict) -> None:
    runs = _runs(job)
    helper = _helper_at(runs)
    assert helper is not None, f"{file}:{name} runs Rust without the helper"
    first = _first_use(runs, INDIRECT.get((file, name)))
    assert first is not None
    assert helper < first, f"{file}:{name} uses Rust before the helper"
