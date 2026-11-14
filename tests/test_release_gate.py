from __future__ import annotations

import importlib.util
import json
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
    "command": [{"id": "golden", "title": "Golden", "run": ["python3", "-c", "pass"]}],
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
    config = {"command": [{"id": "x", "title": "X", "run": ["no-such-tool-xyz"]}]}
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
    assert {c["id"] for c in config["command"]} == {
        "golden",
        "accuracy",
        "compat-preserve",
        "conformance",
        "perf-budgets",
    }
