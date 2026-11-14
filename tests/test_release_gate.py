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

NEXTEST = ["cargo", "nextest", "run", "--workspace", "--no-tests=fail", "-E"]
COMMANDS = {
    "golden": [*NEXTEST, "test(/^golden::/)"],
    "accuracy": [*NEXTEST, "test(/^accuracy::/)"],
    "compat-preserve": [*NEXTEST, "test(/^compat::preserve/)"],
    "conformance": [*NEXTEST, "test(/^conformance::/)"],
    "conformance-contract": ["python3", "-m", "pytest", "-q", "tests/contract"],
    "migration-e2e": [*NEXTEST, "test(/^migration_e2e::/)"],
    "perf-budgets": [*NEXTEST, "test(/^perf_budget::/)"],
    "perf-installed-binary": [
        "python3",
        "scripts/perf/binary_bench.py",
        "run",
        "--binary",
        "target/release/slopvac",
        "--budgets",
        "scripts/perf/binary-budgets.toml",
        "--enforce",
    ],
}
TRIPLES = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "x86_64-pc-windows-msvc",
]
EVIDENCE_IDS = [
    "security-review",
    *(f"signed-artifacts-{t}" for t in TRIPLES),
    "action-contract",
    "hook-contract",
    "policies-doc",
    "accessibility-manual-pass",
]


def issue(bead_id: str, **extra) -> dict:
    return {"id": bead_id, "status": "open", "labels": [], **extra}


def fake_runner(beads: dict[str, list[dict]], command_exit: int = 0):
    def runner(argv):
        if argv[0] == "bd":
            label = argv[argv.index("--label") + 1]
            return subprocess.CompletedProcess(
                argv, 0, json.dumps(beads.get(label, [])), ""
            )
        return subprocess.CompletedProcess(argv, command_exit, "", "boom")

    return runner


def tracker_says(stdout: str, returncode: int = 0):
    def runner(argv):
        return subprocess.CompletedProcess(argv, returncode, stdout, "")

    return runner


def by_id(results):
    return {r.id: r for r in results}


@pytest.fixture(autouse=True)
def tools_installed(monkeypatch):
    """Pretend every tool but `no-such-*` is installed; fake runners decide outcomes."""
    monkeypatch.setattr(
        gate.shutil, "which", lambda name: None if name.startswith("no-such") else name
    )


def item_toml(item_id: str, **overrides) -> str:
    fields = {
        "id": item_id,
        "status": "pass",
        "evidence": [f"https://example.com/evidence/{item_id}"],
        "approver": "lead",
        "date": "2026-11-14",
        **overrides,
    }
    lines = []
    for key, value in fields.items():
        if value is None:
            continue
        rendered = value if key == "date" else json.dumps(value)
        lines.append(f"{key} = {rendered}")
    return "[[item]]\n" + "\n".join(lines) + "\n"


def write_evidence(path: Path, *, skip: str = "", **overrides_for_first) -> Path:
    """Write a full evidence file; `overrides_for_first` apply to security-review."""
    body = ""
    for item_id in EVIDENCE_IDS:
        if item_id == skip:
            continue
        extra = overrides_for_first if item_id == "security-review" else {}
        body += item_toml(item_id, **extra)
    path.write_text(body, encoding="utf-8")
    return path


@pytest.fixture
def evidence(tmp_path):
    return write_evidence(tmp_path / "release-evidence.toml")


CONFIG = {
    "beads": {"release_label": "release:3.0.0", "allowed_open": ["epic"]},
    "command": [
        {"id": suite, "title": suite, "run": run} for suite, run in COMMANDS.items()
    ],
}


def gate_with(runner, evidence_path, config=CONFIG):
    return gate.run_gate(config, runner, evidence_path)


def test_clean_state_passes(evidence):
    results = gate_with(fake_runner({}), evidence)
    assert {r.status for r in results} == {"pass"}
    assert gate.exit_code(results) == 0


def test_open_release_bead_fails_but_allowed_one_does_not(evidence):
    beads = {"release:3.0.0": [issue("epic"), issue("b.1")]}
    result = by_id(gate_with(fake_runner(beads), evidence))["release-beads"]
    assert result.status == "fail"
    assert "b.1" in result.evidence and "epic" not in result.evidence


def test_open_stub_fails(evidence):
    results = gate_with(fake_runner({"stub": [issue("s.1")]}), evidence)
    assert by_id(results)["no-stubs"].status == "fail"


@pytest.mark.parametrize(
    ("task", "swap"),
    [
        (issue("t.1", dependencies=[{"id": "s.1"}]), issue("s.1")),
        (issue("t.1", comments=["built against STUB(s.1)"]), issue("s.1")),
        (issue("t.1"), issue("s.1", description="swaps the stub in t.1")),
    ],
)
def test_soft_blocked_with_open_swap_fails(evidence, task, swap):
    beads = {"soft-blocked": [task], "stub": [swap]}
    result = by_id(gate_with(fake_runner(beads), evidence))["soft-blocked"]
    assert (result.status, result.evidence) == ("fail", "t.1 -> s.1")


def test_soft_blocked_id_prefix_is_not_a_match(evidence):
    beads = {
        "soft-blocked": [issue("t.1")],
        "stub": [issue("s.1", description="swaps t.12")],
    }
    results = gate_with(fake_runner(beads), evidence)
    assert by_id(results)["soft-blocked"].status == "pass"


def test_soft_blocked_with_closed_swap_passes(evidence):
    beads = {"soft-blocked": [issue("t.1", comments=["STUB(s.1)"])]}
    results = gate_with(fake_runner(beads), evidence)
    assert by_id(results)["soft-blocked"].status == "pass"


def test_failing_command_fails_the_gate(evidence):
    results = gate_with(fake_runner({}, command_exit=3), evidence)
    assert by_id(results)["golden"].status == "fail"
    assert gate.exit_code(results) == 1


def test_missing_tool_is_incomplete_not_pass(evidence, monkeypatch):
    monkeypatch.setattr(gate.shutil, "which", lambda name: None)
    results = gate_with(fake_runner({}), evidence)
    assert by_id(results)["golden"].status == "incomplete"
    assert gate.exit_code(results) == 2


def test_missing_tracker_is_incomplete(evidence):
    def runner(argv):
        raise FileNotFoundError(2, "missing", "bd")

    results = gate.run_gate({}, runner, evidence)
    assert by_id(results)["release-beads"].status == "incomplete"


# --- tracker output validation -------------------------------------------


@pytest.mark.parametrize(
    "stdout",
    [
        "",
        "not json",
        "{}",
        '"[]"',
        "null",
        "[1]",
        "[{}]",
        '[{"id": "a", "status": "open"}]',
        '[{"id": "", "status": "open", "labels": []}]',
        '[{"id": "a", "status": "open", "labels": "stub"}]',
    ],
)
def test_malformed_tracker_output_is_not_a_pass(evidence, stdout):
    results = gate_with(tracker_says(stdout), evidence)
    assert by_id(results)["no-stubs"].status == "incomplete"
    assert gate.exit_code(results) != 0


def test_tracker_failure_is_not_a_pass(evidence):
    results = gate_with(tracker_says("[]", returncode=1), evidence)
    assert by_id(results)["no-stubs"].status == "incomplete"


def test_empty_array_means_no_issues(evidence):
    results = gate_with(tracker_says("[]"), evidence)
    assert by_id(results)["no-stubs"].status == "pass"


# --- suite pinning ---------------------------------------------------------


def config_with(suite: str, run: list[str]) -> dict:
    return {
        "command": [
            {**c, "run": run} if c["id"] == suite else c for c in CONFIG["command"]
        ]
    }


def test_shipped_config_lists_the_required_items():
    config = tomllib.loads(
        (SCRIPT.parent / "release-gate.toml").read_text(encoding="utf-8")
    )
    assert {c["id"]: c["run"] for c in config["command"]} == COMMANDS
    assert gate.check_suites_configured(config).status == "pass"


def test_the_trusted_commands_are_exactly_these():
    assert {k: list(v) for k, v in gate.TRUSTED_COMMANDS.items()} == COMMANDS


def test_a_valid_config_is_accepted():
    assert gate.check_suites_configured(CONFIG).status == "pass"


def test_a_config_missing_a_mandatory_suite_fails():
    config = {"command": [c for c in CONFIG["command"] if c["id"] != "migration-e2e"]}
    result = gate.check_suites_configured(config)
    assert (result.status, "migration-e2e" in result.evidence) == ("fail", True)


@pytest.mark.parametrize(
    "run",
    [
        ["true"],
        ["python3", "-c", "pass"],
        ["no-such-tool-xyz"],
        [],
    ],
)
def test_a_no_op_or_unknown_command_is_rejected(run):
    result = gate.check_suites_configured(config_with("golden", run))
    assert (result.status, "golden" in result.evidence) == ("fail", True)


@pytest.mark.parametrize(
    "run",
    [
        # Missing --no-tests=fail.
        ["cargo", "nextest", "run", "--workspace", "-E", "test(/^golden::/)"],
        # Extra flag that could change what runs.
        [*COMMANDS["golden"], "--run-ignored", "all"],
        # Flags in another order.
        [
            "cargo",
            "nextest",
            "run",
            "--no-tests=fail",
            "--workspace",
            "-E",
            "test(/^golden::/)",
        ],
        # Different subcommand.
        [
            "cargo",
            "nextest",
            "list",
            "--workspace",
            "--no-tests=fail",
            "-E",
            "test(/^golden::/)",
        ],
        # Filter selects another suite.
        [*NEXTEST, "test(/^accuracy::/)"],
        # Negated prefix: the string still contains `golden::`.
        [*NEXTEST, "not test(/^golden::/) | test(/^nothing::/)"],
        # Prefix kept, everything else also selected.
        [*NEXTEST, "test(/^golden::/) | all()"],
        # Different runner.
        ["cargo", "test", "golden::"],
    ],
)
def test_an_alternative_nextest_form_is_rejected(run):
    result = gate.check_suites_configured(config_with("golden", run))
    assert (result.status, "golden" in result.evidence) == ("fail", True)


@pytest.mark.parametrize(
    "run",
    [
        ["python3", "-m", "pytest", "-q", "tests/contract/test_schemas.py"],
        ["python3", "-m", "pytest", "-q", "-k", "nothing", "tests/contract"],
        ["python3", "-m", "pytest", "-q"],
    ],
)
def test_a_narrowed_pytest_command_is_rejected(run):
    result = gate.check_suites_configured(config_with("conformance-contract", run))
    assert result.status == "fail"


def test_the_installed_binary_check_cannot_drop_enforcement():
    run = [a for a in COMMANDS["perf-installed-binary"] if a != "--enforce"]
    result = gate.check_suites_configured(config_with("perf-installed-binary", run))
    assert result.status == "fail"


def test_an_unlisted_extra_command_is_rejected():
    config = {
        "command": [*CONFIG["command"], {"id": "extra", "title": "x", "run": ["true"]}]
    }
    result = gate.check_suites_configured(config)
    assert (result.status, "extra" in result.evidence) == ("fail", True)


@pytest.mark.parametrize("bad_id", [["golden"], 7])
def test_a_non_string_command_id_is_reported_not_raised(evidence, bad_id):
    config = {"command": [*CONFIG["command"], {"id": bad_id, "run": ["true"]}]}
    result = gate.check_suites_configured(config)
    assert (result.status, "must be a string" in result.evidence) == ("fail", True)
    assert gate_with(fake_runner({}), evidence, config)


def test_a_rejected_command_is_never_run(evidence):
    ran = []

    def runner(argv):
        ran.append(argv[0])
        return subprocess.CompletedProcess(argv, 0, "[]", "")

    gate_with(runner, evidence, config_with("golden", ["true"]))
    assert "true" not in ran


def test_commands_run_from_the_repository_root(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    done = gate.run_process([sys.executable, "-c", "import os; print(os.getcwd())"])
    expected = Path(__file__).resolve().parents[1]
    assert Path(done.stdout.strip()).resolve() == expected


@pytest.mark.skipif(
    shutil.which("cargo") is None or shutil.which("cargo-nextest") is None,
    reason="cargo-nextest is not installed",
)
def test_a_suite_that_matches_no_tests_fails_the_gate(tmp_path, monkeypatch):
    """The shipped nextest command, run against a crate with no `golden::` test."""
    monkeypatch.undo()  # use the real shutil.which
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


# --- recorded evidence -----------------------------------------------------


def test_every_manual_approval_is_a_mandatory_item():
    assert list(gate.MANDATORY_EVIDENCE) == EVIDENCE_IDS


def test_suites_green_without_an_evidence_file_fails(tmp_path):
    results = gate_with(fake_runner({}), tmp_path / "absent.toml")
    assert gate.exit_code(results) == 1
    assert {r.status for r in results if r.id.startswith("evidence:")} == {"fail"}


def test_a_complete_evidence_file_passes_every_item(evidence):
    results = gate.check_evidence(evidence)
    assert [r.id for r in results] == [f"evidence:{i}" for i in EVIDENCE_IDS]
    assert {r.status for r in results} == {"pass"}


@pytest.mark.parametrize("missing", EVIDENCE_IDS)
def test_a_missing_item_fails_only_that_item(tmp_path, missing):
    path = write_evidence(tmp_path / "e.toml", skip=missing)
    failing = [r.id for r in gate.check_evidence(path) if r.status == "fail"]
    assert failing == [f"evidence:{missing}"]


@pytest.mark.parametrize(
    "override",
    [
        {"status": "pending"},
        {"status": "fail"},
        {"status": "PASS"},
        {"status": None},
        {"evidence": []},
        {"evidence": None},
        {"evidence": ["http://example.com/x"]},
        {"evidence": ["ftp://example.com/x"]},
        {"evidence": ["https://"]},
        {"evidence": ["not a url"]},
        {"evidence": ["https://example.com/ok", "http://example.com/bad"]},
        {"approver": ""},
        {"approver": None},
        {"date": None},
        {"date": '"2026-11-14"'},
    ],
)
def test_an_incomplete_record_is_not_a_pass(tmp_path, override):
    path = write_evidence(tmp_path / "e.toml", **override)
    results = by_id(gate.check_evidence(path))
    assert results["evidence:security-review"].status == "fail"
    assert gate.exit_code(list(results.values())) == 1


def test_an_unknown_key_in_a_record_is_rejected(tmp_path):
    path = write_evidence(tmp_path / "e.toml", notes="looks fine")
    results = by_id(gate.check_evidence(path))
    assert results["evidence-file"].status == "fail"
    assert "notes" in results["evidence-file"].evidence


def test_an_unknown_item_id_is_rejected(tmp_path):
    path = write_evidence(tmp_path / "e.toml")
    path.write_text(path.read_text() + item_toml("made-up"), encoding="utf-8")
    results = by_id(gate.check_evidence(path))
    assert "made-up" in results["evidence-file"].evidence


def test_a_duplicate_item_is_rejected(tmp_path):
    path = write_evidence(tmp_path / "e.toml")
    path.write_text(path.read_text() + item_toml("security-review"), encoding="utf-8")
    results = by_id(gate.check_evidence(path))
    assert "duplicate" in results["evidence-file"].evidence


def test_an_unknown_top_level_key_is_rejected(tmp_path):
    path = write_evidence(tmp_path / "e.toml")
    path.write_text("waived = true\n" + path.read_text(), encoding="utf-8")
    assert by_id(gate.check_evidence(path))["evidence-file"].status == "fail"


def test_a_malformed_evidence_file_fails_every_item(tmp_path):
    path = tmp_path / "e.toml"
    path.write_text("[[item\n", encoding="utf-8")
    results = gate.check_evidence(path)
    assert len(results) == len(EVIDENCE_IDS)
    assert {r.status for r in results} == {"fail"}


def test_omitting_the_musl_approval_fails_that_item_only(tmp_path):
    missing = "signed-artifacts-x86_64-unknown-linux-musl"
    path = write_evidence(tmp_path / "e.toml", skip=missing)
    failing = [r.id for r in gate.check_evidence(path) if r.status == "fail"]
    assert failing == [f"evidence:{missing}"]


@pytest.mark.parametrize("bad_id", ['["security-review"]', "7"])
def test_a_non_string_evidence_id_is_an_evidence_file_failure(tmp_path, bad_id):
    path = write_evidence(tmp_path / "e.toml")
    path.write_text(path.read_text() + f"[[item]]\nid = {bad_id}\n", encoding="utf-8")
    results = by_id(gate.check_evidence(path))
    assert results["evidence-file"].status == "fail"
    assert "must be a string" in results["evidence-file"].evidence
    assert results["evidence:security-review"].status == "pass"


# --- command line ------------------------------------------------------------


@pytest.fixture
def cli(monkeypatch, capsys):
    """Run `main` with a fake runner; return (exit code, parsed JSON rows)."""

    def run(evidence_path, beads=None, runner=None):
        monkeypatch.setattr(gate, "run_process", runner or fake_runner(beads or {}))
        code = gate.main(["--json", "--evidence", str(evidence_path)])
        return code, json.loads(capsys.readouterr().out)

    return run


def test_main_exits_zero_and_lists_every_item(cli, evidence):
    code, rows = cli(evidence)
    # 3 tracker items + suites-configured + 8 commands + 11 approvals.
    assert code == 0
    assert len(rows) == 23
    assert {r["status"] for r in rows} == {"pass"}


def test_main_exits_one_when_the_evidence_file_is_absent(cli, tmp_path):
    code, rows = cli(tmp_path / "absent.toml")
    statuses = {r["id"]: r["status"] for r in rows}
    assert code == 1
    assert statuses["evidence:security-review"] == "fail"
    assert statuses["golden"] == "pass"


def test_main_exits_two_when_only_the_tracker_is_unavailable(cli, evidence):
    def no_bd(argv):
        if argv[0] == "bd":
            raise FileNotFoundError(2, "missing", "bd")
        return subprocess.CompletedProcess(argv, 0, "", "")

    code, rows = cli(evidence, runner=no_bd)
    assert code == 2
    assert {r["status"] for r in rows} == {"pass", "incomplete"}


def test_main_forwards_the_evidence_path(cli, tmp_path):
    missing = "hook-contract"
    path = write_evidence(tmp_path / "custom.toml", skip=missing)
    code, rows = cli(path)
    failing = [r["id"] for r in rows if r["status"] == "fail"]
    # The default path does not exist, so only the one skipped item failing
    # shows the custom file was the one read.
    assert (code, failing) == (1, [f"evidence:{missing}"])


def test_main_prints_a_table_by_default(monkeypatch, capsys, evidence):
    monkeypatch.setattr(gate, "run_process", fake_runner({}))
    assert gate.main(["--evidence", str(evidence)]) == 0
    out = capsys.readouterr().out.splitlines()
    assert out[0] == "| Item | Status | Evidence |"
    assert len(out) == 2 + 23
