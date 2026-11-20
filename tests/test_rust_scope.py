"""scripts/ci/rust_scope.py narrows pull requests to affected crates and fails closed.

The script is driven through its command line and GITHUB_OUTPUT. Most tests put a stub
in place of cargo-delta, because the stub is the process boundary this script owns. The
integration test at the end runs the pinned cargo-delta on a throwaway git workspace, so
both directions are proven with the real mapping: an affected crate cannot be skipped and
an unaffected one is.
"""

from __future__ import annotations

import os
import shutil
import stat
import subprocess
import sys
import textwrap
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "ci" / "rust_scope.py"
CONFIG = ROOT / ".github" / "cargo-delta.toml"

# A stand-in for `cargo delta`: records that it ran, then writes the payload to --output.
STUB = """#!/usr/bin/env python3
import os, sys, time
from pathlib import Path
Path(os.getenv("STUB_CALLS")).open("a").write(" ".join(sys.argv[1:]) + "\\n")
args = sys.argv[1:]
if os.getenv("STUB_SLEEP"):
    time.sleep(float(os.getenv("STUB_SLEEP")))
target = Path(args[args.index("--output") + 1])
if os.getenv("STUB_BYTES"):
    target.write_bytes(bytes.fromhex(os.getenv("STUB_BYTES")))
elif os.getenv("STUB_DIRECTORY"):
    target.mkdir()
elif os.getenv("STUB_OUTPUT") is not None:
    target.write_text(os.getenv("STUB_OUTPUT"))
sys.exit(int(os.getenv("STUB_EXIT", "0")))
"""

IMPACT = (
    '{"Modified": ["slopvac-fs"], '
    '"Affected": ["slopvac", "slopvac-fs", "slopvac-engine"], "Required": []}'
)


@pytest.fixture
def stub(tmp_path):
    path = tmp_path / "delta-stub"
    path.write_text(STUB)
    path.chmod(path.stat().st_mode | stat.S_IEXEC)
    calls = tmp_path / "calls"
    out = tmp_path / "github_output"
    out.touch()

    class Stub:
        calls_file = calls

        @staticmethod
        def run(
            *, payload: str | None = IMPACT, exit_code: int = 0, **env: str
        ) -> dict[str, str]:
            environment = {
                **os.environ,
                "STUB_CALLS": str(calls),
                "STUB_EXIT": str(exit_code),
                "GITHUB_OUTPUT": str(out),
                "EVENT_NAME": "pull_request",
                "FORCE_ALL": "false",
                "BASE_REF": "main",
                **env,
            }
            if payload is not None:
                environment["STUB_OUTPUT"] = payload
            subprocess.run(
                [sys.executable, str(SCRIPT), "--delta", str(path)],
                env=environment,
                check=True,
                capture_output=True,
                text=True,
            )
            return dict(line.split("=", 1) for line in out.read_text().splitlines())

        @staticmethod
        def invocations() -> list[str]:
            return calls.read_text().splitlines() if calls.exists() else []

    return Stub


def test_a_pull_request_runs_lint_on_modified_crates_and_build_on_affected_ones(stub):
    outputs = stub.run()
    assert outputs == {
        "rust_scope": "scoped",
        "rust_modified": "-p slopvac-fs",
        "rust_affected": "-p slopvac -p slopvac-engine -p slopvac-fs",
    }


def test_the_base_branch_is_the_diff_base(stub):
    stub.run(BASE_REF="release/3.0")
    assert "--base-ref origin/release/3.0" in stub.invocations()[0]


@pytest.mark.parametrize("event", ["merge_group", "push", "workflow_dispatch"])
def test_events_other_than_pull_request_run_the_whole_workspace_without_asking(
    stub, event
):
    outputs = stub.run(EVENT_NAME=event)
    assert outputs == {
        "rust_scope": "all",
        "rust_modified": "--workspace",
        "rust_affected": "--workspace",
    }
    assert stub.invocations() == []


@pytest.mark.parametrize("force_all", ["true", "", "maybe"])
def test_anything_but_an_explicit_false_force_flag_runs_the_whole_workspace(
    stub, force_all
):
    assert stub.run(FORCE_ALL=force_all)["rust_scope"] == "all"
    assert stub.invocations() == []


@pytest.mark.parametrize(
    "run",
    [
        pytest.param({"exit_code": 1}, id="delta-fails"),
        pytest.param({"payload": None}, id="no-output-file"),
        pytest.param(
            {"payload": None, "STUB_BYTES": "ff fe".replace(" ", "")}, id="invalid-utf8"
        ),
        pytest.param({"payload": None, "STUB_DIRECTORY": "1"}, id="unreadable-output"),
        pytest.param(
            {"payload": None, "STUB_SLEEP": "5", "RUST_SCOPE_TIMEOUT": "0.5"},
            id="timeout",
        ),
        pytest.param({"payload": ""}, id="empty-output-file"),
        pytest.param({"payload": "not json"}, id="garbage"),
        pytest.param({"payload": '{"Modified": ["a"]}'}, id="missing-tier"),
        pytest.param({"payload": '{"Modified": [], "Affected": []}'}, id="no-crates"),
        pytest.param(
            {"payload": '{"Modified": ["a"], "Affected": []}'}, id="no-affected"
        ),
        pytest.param(
            {"payload": '{"Modified": ["a"], "Affected": ["b"]}'}, id="not-a-superset"
        ),
        pytest.param(
            {"payload": '{"Modified": ["a; rm -rf /"], "Affected": ["a; rm -rf /"]}'},
            id="shell-metacharacters",
        ),
        pytest.param(
            {"payload": '{"Modified": ["-Zunstable"], "Affected": ["-Zunstable"]}'},
            id="looks-like-a-flag",
        ),
        pytest.param(
            {"payload": '{"Modified": "a", "Affected": "a"}'}, id="not-a-list"
        ),
        pytest.param({"BASE_REF": "main; echo"}, id="odd-base-branch"),
        pytest.param({"BASE_REF": ""}, id="no-base-branch"),
    ],
)
def test_any_doubt_about_cargo_delta_runs_the_whole_workspace(stub, run):
    outputs = stub.run(**run)
    assert outputs == {
        "rust_scope": "all",
        "rust_modified": "--workspace",
        "rust_affected": "--workspace",
    }


def test_a_missing_cargo_delta_runs_the_whole_workspace(tmp_path):
    out = tmp_path / "out"
    out.touch()
    subprocess.run(
        [sys.executable, str(SCRIPT), "--delta", str(tmp_path / "absent")],
        env={
            **os.environ,
            "GITHUB_OUTPUT": str(out),
            "EVENT_NAME": "pull_request",
            "FORCE_ALL": "false",
            "BASE_REF": "main",
        },
        check=True,
        capture_output=True,
    )
    assert "rust_scope=all" in out.read_text().splitlines()


# --- trip wires, through the real cargo-delta -------------------------------------

CARGO_DELTA = shutil.which("cargo-delta")
needs_delta = pytest.mark.skipif(
    CARGO_DELTA is None and not os.environ.get("SLOPVAC_REQUIRE_CARGO_DELTA"),
    reason="cargo-delta is not installed (CI sets SLOPVAC_REQUIRE_CARGO_DELTA)",
)


def git(cwd: Path, *args: str) -> None:
    subprocess.run(
        ["git", "-c", "user.name=t", "-c", "user.email=t@example.test", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
    )


@pytest.fixture
def workspace(tmp_path):
    """A three-crate workspace: app depends on lib, and solo stands alone."""
    repo = tmp_path / "repo"
    files = {
        "Cargo.toml": '[workspace]\nresolver = "2"\nmembers = ["crates/*"]\n',
        "crates/lib/Cargo.toml": '[package]\nname = "lib"\nversion = "0.1.0"\nedition = "2021"\n',
        "crates/lib/src/lib.rs": "pub fn one() -> u8 { 1 }\n",
        "crates/app/Cargo.toml": textwrap.dedent(
            """\
            [package]
            name = "app"
            version = "0.1.0"
            edition = "2021"
            [dependencies]
            lib = { path = "../lib" }
            """
        ),
        "crates/app/src/lib.rs": "pub fn two() -> u8 { lib::one() + 1 }\n",
        "crates/solo/Cargo.toml": '[package]\nname = "solo"\nversion = "0.1.0"\nedition = "2021"\n',
        "crates/solo/src/lib.rs": "pub fn three() -> u8 { 3 }\n",
        "crates/solo/tests/fixtures/data.json": "{}\n",
        ".github/cargo-delta.toml": CONFIG.read_text(),
        ".github/workflows/ci.yml": "name: ci\n",
    }
    for name, text in files.items():
        path = repo / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
    git(tmp_path, "init", "-q", "-b", "main", str(repo))
    subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=repo, check=True)
    git(repo, "add", "-A")
    git(repo, "commit", "-q", "-m", "base")
    git(repo, "update-ref", "refs/remotes/origin/main", "HEAD")
    git(repo, "checkout", "-q", "-b", "topic")
    return repo


def scope_of(repo: Path, tmp_path: Path) -> dict[str, str]:
    out = tmp_path / "github_output"
    out.write_text("")
    subprocess.run(
        [sys.executable, str(SCRIPT)],
        cwd=repo,
        env={
            **os.environ,
            "GITHUB_OUTPUT": str(out),
            "EVENT_NAME": "pull_request",
            "FORCE_ALL": "false",
            "BASE_REF": "main",
        },
        check=True,
        capture_output=True,
    )
    return dict(line.split("=", 1) for line in out.read_text().splitlines())


EVERY_CRATE = {"app", "lib", "solo"}


def packages(arguments: str) -> set[str]:
    """The crates a cargo argument string selects; `--workspace` selects them all."""
    if arguments == "--workspace":
        return EVERY_CRATE
    return set(arguments.replace("-p ", "").split())


@needs_delta
def test_changing_a_leaf_crate_checks_it_and_its_dependents_and_skips_the_rest(
    workspace, tmp_path
):
    (workspace / "crates/lib/src/lib.rs").write_text("pub fn one() -> u8 { 11 }\n")
    git(workspace, "commit", "-qam", "change lib")
    outputs = scope_of(workspace, tmp_path)
    assert outputs == {
        "rust_scope": "scoped",
        "rust_modified": "-p lib",
        "rust_affected": "-p app -p lib",
    }


@needs_delta
def test_changing_a_top_crate_leaves_its_dependencies_unchecked(workspace, tmp_path):
    (workspace / "crates/app/src/lib.rs").write_text("pub fn two() -> u8 { 22 }\n")
    git(workspace, "commit", "-qam", "change app")
    outputs = scope_of(workspace, tmp_path)
    assert outputs["rust_affected"] == "-p app"


@needs_delta
@pytest.mark.parametrize(
    "path",
    [
        ".github/workflows/ci.yml",
        ".github/cargo-delta.toml",
        "crates/solo/tests/fixtures/data.json",
        "rust-toolchain.toml",
        "schemas/v3/x.json",
    ],
)
def test_a_trip_wire_path_selects_every_crate(workspace, tmp_path, path):
    target = workspace / path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(target.read_text() + "# change\n" if target.exists() else "x\n")
    git(workspace, "add", "-A")
    git(workspace, "commit", "-qm", "change a trip wire")
    outputs = scope_of(workspace, tmp_path)
    assert packages(outputs["rust_modified"]) == EVERY_CRATE
    assert packages(outputs["rust_affected"]) == EVERY_CRATE


@needs_delta
def test_a_change_that_maps_to_no_crate_runs_the_whole_workspace(workspace, tmp_path):
    (workspace / "NOTES.txt").write_text("not read by any crate\n")
    git(workspace, "add", "-A")
    git(workspace, "commit", "-qm", "stray file")
    assert packages(scope_of(workspace, tmp_path)["rust_affected"]) == EVERY_CRATE


@needs_delta
def test_a_root_manifest_setting_outside_the_dependency_table_selects_every_crate(
    workspace, tmp_path
):
    manifest = workspace / "Cargo.toml"
    manifest.write_text(manifest.read_text() + "[profile.release]\nlto = true\n")
    git(workspace, "commit", "-qam", "profile")
    outputs = scope_of(workspace, tmp_path)
    assert packages(outputs["rust_affected"]) == EVERY_CRATE


@needs_delta
def test_a_root_profile_change_beside_one_crate_change_selects_every_crate(
    workspace, tmp_path
):
    # The scoped root manifest (dependency-table edits only) must not hide a global setting
    # that arrives in the same pull request as an edit to one crate.
    manifest = workspace / "Cargo.toml"
    manifest.write_text(manifest.read_text() + "[profile.release]\nlto = true\n")
    (workspace / "crates/solo/src/lib.rs").write_text("pub fn three() -> u8 { 33 }\n")
    git(workspace, "commit", "-qam", "profile and solo")
    outputs = scope_of(workspace, tmp_path)
    assert packages(outputs["rust_modified"]) == EVERY_CRATE
    assert packages(outputs["rust_affected"]) == EVERY_CRATE
