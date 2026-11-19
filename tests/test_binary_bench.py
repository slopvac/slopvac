from __future__ import annotations

import json
import os
import sys
import tomllib
from pathlib import Path

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
    budgets = {
        "binary_size_bytes": 500,
        "scenario": {"s": {"median_ms": 50.0, "peak_rss_kb": 1000}},
    }
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
        [
            "run",
            "--binary",
            sys.executable,
            "--scenarios",
            str(scenarios),
            "--runs",
            "3",
            "--warmup",
            "1",
            "--output",
            str(out),
        ]
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
    argv = [
        "run",
        "--binary",
        sys.executable,
        "--scenarios",
        str(scenarios),
        "--runs",
        "1",
        "--warmup",
        "0",
        "--output",
        str(tmp_path / "o.json"),
        "--budgets",
        str(budgets),
    ]
    assert bb.main(argv) == 0
    assert bb.main([*argv, "--enforce"]) == 1


def test_shipped_scenarios_load_with_unique_names():
    assert len(bb.load_scenarios()) >= 5


BUDGETS = bb.ROOT / "scripts/perf/binary-budgets.toml"
BUDGET_KEYS = {"median_ms", "p95_ms", "peak_rss_kb"}


def test_shipped_scenarios_do_not_accept_usage_errors():
    for scenario in bb.load_scenarios():
        assert 2 not in scenario.get("accept_exits", [0]), scenario["name"]


def test_every_shipped_scenario_has_a_complete_budget():
    with BUDGETS.open("rb") as handle:
        budgets = tomllib.load(handle)
    names = {s["name"] for s in bb.load_scenarios()}
    assert set(budgets["scenario"]) == names
    assert budgets["binary_size_bytes"] > 0
    for name, limits in budgets["scenario"].items():
        assert set(limits) == BUDGET_KEYS, name
        assert all(value > 0 for value in limits.values()), name


def built_binary() -> Path | None:
    candidates = [os.environ.get("SLOPVAC_BIN", "")]
    candidates += [str(bb.ROOT / f"target/{p}/slopvac") for p in ("release", "debug")]
    return next((Path(c) for c in candidates if c and Path(c).is_file()), None)


# The engine has no comment-mode parser yet (STUB(slopvac-8l4.12.28) in
# crates/slopvac/src/engine.rs), so this scenario exits 2 for a reason that
# has nothing to do with config. Drop the entry when that bead lands.
UNWIRED_SCENARIOS = {"lint comments source"}


@pytest.mark.skipif(built_binary() is None, reason="no built slopvac binary")
def test_real_binary_accepts_every_shipped_scenario():
    binary = built_binary()
    failures = []
    for scenario in bb.load_scenarios():
        if scenario["name"] in UNWIRED_SCENARIOS:
            continue
        result = bb.measure(binary, scenario, runs=1, warmup=0)
        if result["status"] != "ok":
            failures.append(f"{scenario['name']}: exit {result['exit_code']}")
    assert failures == []


FIXTURES = bb.ROOT / "scripts/perf/fixtures"
FIXTURE_CONFIG = FIXTURES / "slopvac.toml"


def test_fixture_config_is_a_3_0_config():
    with FIXTURE_CONFIG.open("rb") as handle:
        assert tomllib.load(handle)["schema_version"] == 3


def test_every_config_reading_scenario_uses_the_fixture_config_and_inputs():
    reading = [s for s in bb.load_scenarios() if s["args"] != ["--version"]]
    assert reading
    for scenario in reading:
        args = [a.replace("{root}", str(bb.ROOT)) for a in scenario["args"]]
        assert args[args.index("--config") + 1] == str(FIXTURE_CONFIG), scenario["name"]
        if args[0] == "lint":
            target = Path(args[-1])
            assert target.is_relative_to(FIXTURES), scenario["name"]
