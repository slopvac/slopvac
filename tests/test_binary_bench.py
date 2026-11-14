from __future__ import annotations

import json
import sys

import pytest

from scripts.perf import binary_bench as bb


@pytest.mark.parametrize(
    ("values", "fraction", "expected"),
    [
        ([5.0], 0.95, 5.0),
        ([1.0, 2.0, 3.0, 4.0], 0.5, 2.0),
        ([float(n) for n in range(1, 21)], 0.95, 19.0),
        ([3.0, 1.0, 2.0], 1.0, 3.0),
    ],
)
def test_percentile_is_nearest_rank(values, fraction, expected):
    assert bb.percentile(values, fraction) == expected


@pytest.mark.parametrize(
    ("base", "head", "regressed"),
    [
        (100.0, 130.0, True),
        (100.0, 120.0, False),  # under the 1.25 ratio
        (2.0, 4.0, False),  # doubles, but only 2 ms: below the floor
        (100.0, 80.0, False),
    ],
)
def test_regression_needs_ratio_and_floor(base, head, regressed):
    assert bb.regressed(base, head, 1.25, 5.0) is regressed


def result(median, size=1000, status="ok"):
    scenario = {"name": "s", "status": status}
    if status == "ok":
        scenario.update(median_ms=median, p95_ms=median, peak_rss_kb=900)
    return {"binary_size_bytes": size, "scenarios": [scenario]}


def test_compare_flags_regression_and_reports_size():
    table, bad = bb.compare_table(result(100.0), result(200.0, size=1100), 1.25, 5.0)
    assert bad
    assert "+100.0% REGRESSION" in table
    assert "1000 -> 1100 bytes (+10.0%)" in table


def test_compare_skips_unmeasured_scenarios():
    table, bad = bb.compare_table(result(100.0), result(0, status="error"), 1.25, 5.0)
    assert not bad
    assert "not comparable" in table


def test_budget_lists_only_exceeded_limits():
    budgets = {"binary_size_bytes": 500, "scenario": {"s": {"median_ms": 50.0, "peak_rss_kb": 1000}}}
    over = bb.check_budgets(result(60.0), budgets)
    assert over == ["binary size 1000 > 500 bytes", "s: median_ms 60.0 > 50.0"]


def test_run_measures_a_real_command_and_records_failures(tmp_path, capsys):
    scenarios = tmp_path / "scenarios.toml"
    scenarios.write_text(
        '[[scenario]]\nname = "ok"\nargs = ["-c", "pass"]\n'
        '[[scenario]]\nname = "bad"\nargs = ["-c", "raise SystemExit(2)"]\n'
    )
    out = tmp_path / "out.json"
    code = bb.main(
        ["run", "--binary", sys.executable, "--scenarios", str(scenarios), "--runs", "3", "--warmup", "1", "--output", str(out)]
    )
    data = json.loads(out.read_text())
    ok, bad = data["scenarios"]
    assert code == 0
    assert ok["status"] == "ok" and ok["runs"] == 3 and ok["peak_rss_kb"] > 0
    assert bad == {"name": "bad", "status": "error", "exit_code": 2}
    assert "bad exited 2" in capsys.readouterr().err


def test_enforce_fails_when_over_budget(tmp_path):
    budgets = tmp_path / "b.toml"
    budgets.write_text("binary_size_bytes = 1\n")
    scenarios = tmp_path / "s.toml"
    scenarios.write_text('[[scenario]]\nname = "ok"\nargs = ["-c", "pass"]\n')
    argv = ["run", "--binary", sys.executable, "--scenarios", str(scenarios), "--runs", "1", "--warmup", "0",
            "--output", str(tmp_path / "o.json"), "--budgets", str(budgets)]
    assert bb.main(argv) == 0
    assert bb.main([*argv, "--enforce"]) == 1


def test_shipped_scenarios_load_with_unique_names():
    assert len(bb.load_scenarios()) >= 5
