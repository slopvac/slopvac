from __future__ import annotations

import importlib.util
import json
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "release_gate.py"
SPEC = importlib.util.spec_from_file_location("release_gate", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
gate = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = gate
SPEC.loader.exec_module(gate)


def fake_runner(beads: dict[str, list[dict]], command_exit: int = 0):
    def runner(argv):
        if argv[0] == "bd":
            label = argv[argv.index("--label") + 1]
            return subprocess.CompletedProcess(
                argv, 0, json.dumps(beads.get(label, [])), ""
            )
        return subprocess.CompletedProcess(argv, command_exit, "", "boom")

    return runner


def by_id(results):
    return {r.id: r for r in results}


CONFIG = {
    "beads": {"release_label": "release:3.0.0", "allowed_open": ["epic"]},
    "command": [
        {"id": suite, "title": suite, "run": ["python3", "-c", "pass"]}
        for suite in gate.MANDATORY_SUITES
    ],
}


def test_clean_state_passes():
    results = gate.run_gate(CONFIG, fake_runner({}))
    assert {r.status for r in results} == {"pass"}
    assert gate.exit_code(results) == 0


def test_open_release_bead_fails_but_allowed_one_does_not():
    beads = {"release:3.0.0": [{"id": "epic"}, {"id": "b.1"}]}
    result = by_id(gate.run_gate(CONFIG, fake_runner(beads)))["release-beads"]
    assert result.status == "fail"
    assert "b.1" in result.evidence and "epic" not in result.evidence


def test_open_stub_fails():
    result = by_id(gate.run_gate(CONFIG, fake_runner({"stub": [{"id": "s.1"}]})))[
        "no-stubs"
    ]
    assert result.status == "fail"


@pytest.mark.parametrize(
    ("task", "swap"),
    [
        ({"id": "t.1", "dependencies": [{"id": "s.1"}]}, {"id": "s.1"}),
        ({"id": "t.1", "comments": ["built against STUB(s.1)"]}, {"id": "s.1"}),
        ({"id": "t.1"}, {"id": "s.1", "description": "swaps the stub in t.1"}),
    ],
)
def test_soft_blocked_with_open_swap_fails(task, swap):
    beads = {"soft-blocked": [task], "stub": [swap]}
    result = by_id(gate.run_gate(CONFIG, fake_runner(beads)))["soft-blocked"]
    assert (result.status, result.evidence) == ("fail", "t.1 -> s.1")


def test_soft_blocked_id_prefix_is_not_a_match():
    beads = {
        "soft-blocked": [{"id": "t.1"}],
        "stub": [{"id": "s.1", "description": "swaps t.12"}],
    }
    assert (
        by_id(gate.run_gate(CONFIG, fake_runner(beads)))["soft-blocked"].status
        == "pass"
    )


def test_soft_blocked_with_closed_swap_passes():
    beads = {"soft-blocked": [{"id": "t.1", "comments": ["STUB(s.1)"]}]}
    assert (
        by_id(gate.run_gate(CONFIG, fake_runner(beads)))["soft-blocked"].status
        == "pass"
    )


def test_failing_command_fails_the_gate():
    results = gate.run_gate(CONFIG, fake_runner({}, command_exit=3))
    assert by_id(results)["golden"].status == "fail"
    assert gate.exit_code(results) == 1


def test_missing_tool_is_incomplete_not_pass():
    config = {
        **CONFIG,
        "command": [
            *CONFIG["command"],
            {"id": "x", "title": "X", "run": ["no-such-tool-xyz"]},
        ],
    }
    results = gate.run_gate(config, fake_runner({}))
    assert by_id(results)["x"].status == "incomplete"
    assert gate.exit_code(results) == 2


def test_missing_tracker_is_incomplete():
    def runner(argv):
        raise FileNotFoundError(2, "missing", "bd")

    assert by_id(gate.run_gate({}, runner))["release-beads"].status == "incomplete"


def test_shipped_config_lists_the_required_items():
    config = tomllib.loads(
        (SCRIPT.parent / "release-gate.toml").read_text(encoding="utf-8")
    )
    assert {c["id"] for c in config["command"]} == set(gate.MANDATORY_SUITES)
    assert gate.check_suites_configured(config).status == "pass"


def test_a_config_missing_a_mandatory_suite_fails():
    config = {
        "command": [c for c in CONFIG["command"] if c["id"] != "migration-e2e"],
    }
    result = gate.check_suites_configured(config)
    assert (result.status, "migration-e2e" in result.evidence) == ("fail", True)


def test_a_nextest_suite_that_can_pass_with_zero_tests_fails():
    nextest = ["cargo", "nextest", "run", "-E", "test(/^golden::/)"]
    config = {
        "command": [
            {"id": s, "title": s, "run": nextest if s == "golden" else ["true"]}
            for s in gate.MANDATORY_SUITES
        ]
    }
    result = gate.check_suites_configured(config)
    assert (result.status, "golden" in result.evidence) == ("fail", True)


def test_commands_run_from_the_repository_root(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    done = gate.run_process([sys.executable, "-c", "import os; print(os.getcwd())"])
    assert Path(done.stdout.strip()).resolve() == gate.REPO_ROOT.resolve()


@pytest.mark.skipif(
    shutil.which("cargo") is None or shutil.which("cargo-nextest") is None,
    reason="cargo-nextest is not installed",
)
def test_a_suite_that_matches_no_tests_fails_the_gate(tmp_path):
    """The shipped nextest command, run against a crate with no `golden::` test."""
    crate = tmp_path / "probe"
    (crate / "src").mkdir(parents=True)
    (crate / "Cargo.toml").write_text(
        '[package]\nname = "probe"\nversion = "0.0.0"\nedition = "2021"\n[workspace]\n',
        encoding="utf-8",
    )
    (crate / "src" / "lib.rs").write_text(
        "#[cfg(test)]\nmod other {\n    #[test]\n    fn passes() {}\n}\n",
        encoding="utf-8",
    )
    shipped = tomllib.loads(
        (SCRIPT.parent / "release-gate.toml").read_text(encoding="utf-8")
    )
    entry = next(c for c in shipped["command"] if c["id"] == "golden")

    def in_probe(argv):
        return subprocess.run(
            argv, capture_output=True, text=True, check=False, cwd=crate
        )

    result = gate.check_command(in_probe, entry)
    assert result.status == "fail"
