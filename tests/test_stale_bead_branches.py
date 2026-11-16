from __future__ import annotations

import importlib.util
import json
import stat
import sys
from datetime import UTC, datetime
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "stale_bead_branches.py"
WORKFLOW = ROOT / ".github" / "workflows" / "trunk-dev-stale-branches.yml"
SPEC = importlib.util.spec_from_file_location("stale_bead_branches", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
stale = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = stale
SPEC.loader.exec_module(stale)

NOW = datetime(2026, 1, 10, 12, 0, tzinfo=UTC)


def bead(
    bead_id, started_at, *, branch="feat/x", status="in_progress", assignee="alice"
):
    item = {"id": bead_id, "status": status, "assignee": assignee}
    if started_at is not None:
        item["started_at"] = started_at
    item["metadata"] = {"branch": branch} if branch is not None else {}
    return item


# Ages relative to NOW: 49h, 47h, exactly 48h, exactly 72h.
OLD = "2026-01-08T11:00:00Z"
FRESH = "2026-01-08T13:00:00Z"
EDGE_48H = "2026-01-08T12:00:00Z"
EDGE_72H = "2026-01-07T12:00:00Z"


def fake_bd(tmp_path, stdout, code=0, stderr=""):
    """Write an executable that stands in for `bd` and records its arguments."""
    payload = tmp_path / "payload.json"
    payload.write_text(stdout if isinstance(stdout, str) else json.dumps(stdout))
    err = tmp_path / "stderr.txt"
    err.write_text(stderr)
    args = tmp_path / "args.txt"
    script = tmp_path / "bd"
    script.write_text(
        f'#!/bin/sh\nprintf "%s\\n" "$@" > "{args}"\ncat "{payload}"\ncat "{err}" >&2\nexit {code}\n'
    )
    script.chmod(script.stat().st_mode | stat.S_IEXEC)
    return script, args


def run(tmp_path, stdout, capsys, *, hours=None, code=0, stderr=""):
    script, args = fake_bd(tmp_path, stdout, code, stderr)
    env = {"BD_BIN": str(script)}
    if hours is not None:
        env["TRUNK_DEV_STALE_HOURS"] = hours
    status = stale.main(env=env, now=NOW)
    captured = capsys.readouterr()
    return status, captured, args


def test_a_stale_bead_fails_the_run_and_is_listed(tmp_path, capsys):
    status, out, _ = run(tmp_path, [bead("b-1", OLD, branch="feat/old")], capsys)
    assert status == 1
    assert out.out == (
        "## Stale in-progress branches older than 48 hours (1)\n\n"
        "- b-1: `feat/old` (2.0 days; alice)\n"
    )


def test_fresh_beads_pass(tmp_path, capsys):
    status, out, _ = run(tmp_path, [bead("b-1", FRESH)], capsys)
    assert status == 0
    assert out.out == "No in-progress bead branches older than 48 hours.\n"


def test_the_summary_lists_only_the_stale_beads(tmp_path, capsys):
    beads = [
        bead("b-old", OLD),
        bead("b-new", FRESH),
        bead("b-older", EDGE_72H, assignee=None),
    ]
    status, out, _ = run(tmp_path, beads, capsys)
    assert status == 1
    assert "(2)" in out.out
    assert "b-old:" in out.out
    assert "b-older: `feat/x` (3.0 days; unassigned)" in out.out
    assert "b-new" not in out.out


def test_a_bead_exactly_at_the_threshold_is_not_stale(tmp_path, capsys):
    status, _, _ = run(tmp_path, [bead("b-1", EDGE_48H)], capsys)
    assert status == 0


def test_a_bead_one_second_past_the_threshold_is_stale(tmp_path, capsys):
    status, _, _ = run(tmp_path, [bead("b-1", "2026-01-08T11:59:59Z")], capsys)
    assert status == 1


@pytest.mark.parametrize(
    ("hours", "expected"),
    [("24", 1), ("48.5", 1), ("49", 0), ("72", 0)],
)
def test_the_threshold_comes_from_the_environment(tmp_path, capsys, hours, expected):
    # The bead is 49 hours old; 49 is the boundary and is not stale.
    status, _, _ = run(tmp_path, [bead("b-1", OLD)], capsys, hours=hours)
    assert status == expected


def test_a_custom_threshold_is_reported_in_the_summary(tmp_path, capsys):
    status, out, _ = run(tmp_path, [bead("b-1", OLD)], capsys, hours="24")
    assert status == 1
    assert out.out.startswith("## Stale in-progress branches older than 24 hours (1)")


@pytest.mark.parametrize("branch", [None, ""])
def test_beads_without_a_branch_are_ignored(tmp_path, capsys, branch):
    status, _, _ = run(tmp_path, [bead("b-1", OLD, branch=branch)], capsys)
    assert status == 0


def test_a_bead_without_metadata_is_ignored(tmp_path, capsys):
    item = {"id": "b-1", "status": "in_progress", "started_at": OLD}
    status, _, _ = run(tmp_path, [item], capsys)
    assert status == 0


def test_a_missing_started_at_is_ignored(tmp_path, capsys):
    status, out, _ = run(tmp_path, [bead("b-1", None)], capsys)
    assert status == 0
    assert out.err == ""


def test_an_unparseable_started_at_is_skipped_with_a_warning(tmp_path, capsys):
    status, out, _ = run(tmp_path, [bead("b-1", "yesterday")], capsys)
    assert status == 0
    assert "b-1" in out.err
    assert "unparseable started_at" in out.err


def test_beads_that_are_not_in_progress_are_ignored(tmp_path, capsys):
    beads = [bead("b-1", OLD, status="open"), bead("b-2", OLD, status="closed")]
    status, _, _ = run(tmp_path, beads, capsys)
    assert status == 0


def test_an_empty_ledger_passes(tmp_path, capsys):
    status, _, _ = run(tmp_path, [], capsys)
    assert status == 0


def test_the_script_asks_bd_for_in_progress_beads_with_a_branch_read_only(
    tmp_path, capsys
):
    _, _, args = run(tmp_path, [], capsys)
    assert args.read_text().split() == [
        "--readonly",
        "list",
        "--status",
        "in_progress",
        "--has-metadata-key",
        "branch",
        "--json",
        "--limit",
        "0",
    ]


def test_a_failing_bd_is_an_error_even_with_stale_looking_output(tmp_path, capsys):
    status, out, _ = run(tmp_path, [], capsys, code=3, stderr="database locked")
    assert status == 2
    assert "bd exited 3" in out.err
    assert "database locked" in out.err


def test_a_missing_bd_binary_is_an_error(tmp_path, capsys):
    status = stale.main(env={"BD_BIN": str(tmp_path / "absent")}, now=NOW)
    assert status == 2
    assert "cannot run" in capsys.readouterr().err


@pytest.mark.parametrize("payload", ["not json", '{"id": "b-1"}'])
def test_output_that_is_not_a_json_list_is_an_error(tmp_path, capsys, payload):
    status, _, _ = run(tmp_path, payload, capsys)
    assert status == 2


def test_the_workflow_job_runs_the_script_with_pipefail():
    workflow = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
    job = workflow["jobs"]["stale-bead-branches"]
    step = next(s for s in job["steps"] if s["name"].startswith("Report stale"))
    assert step["run"].startswith("python3 scripts/stale_bead_branches.py")
    assert step["shell"] == "bash"
