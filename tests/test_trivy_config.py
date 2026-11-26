"""The Trivy gates keep main policy, read trusted config and print nothing raw."""

from __future__ import annotations

import datetime
import json
import re
import shutil
import stat
import subprocess
import sys
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github/workflows"
JOBS = ["trivy", "trivy-secret"]
SCAN = "scripts/ci/run_trivy.sh"
WRITER = "scripts/ci/trivy_trusted_config.sh"
REPORTER = "scripts/print_trivy_findings.py"
ALL_SEVERITIES = {"UNKNOWN", "LOW", "MEDIUM", "HIGH", "CRITICAL"}
HOSTILE = "::error::injected"
GROUP = "##[group]injected"

# The effective policy of each job on main 4a7cceaf02, worked out from that
# workflow and trivy-action v0.36.0 (action.yaml, entrypoint.sh), not from the
# current workflow. The action unsets TRIVY_SEVERITY for format sarif, so the
# full scan gates on every severity. The secret scan prints a table, so its
# severity input applies.
MAIN_POLICY = {
    "trivy": {
        "scanners": {"vuln", "misconfig", "secret"},
        "severities": ALL_SEVERITIES,
        "exit_code": "1",
        "ignorefile": ".trivyignore.yaml",
        "version": "v0.76.0",
    },
    "trivy-secret": {
        "scanners": {"secret"},
        "severities": {"HIGH", "CRITICAL"},
        "exit_code": "1",
        "ignorefile": ".trivyignore.yaml",
        "version": "v0.76.0",
    },
}


def load_workflow() -> dict:
    return yaml.safe_load((WORKFLOWS / "security.yml").read_text())


def steps_of(job: str) -> list[dict]:
    return load_workflow()["jobs"][job]["steps"]


def step_running(job: str, script: str) -> dict:
    found = [s for s in steps_of(job) if script in str(s.get("run", ""))]
    assert len(found) == 1, f"{job}: expected one step that runs {script}"
    return found[0]


def test_ignore_file_has_one_scoped_expiring_entry() -> None:
    data = yaml.safe_load((ROOT / ".trivyignore.yaml").read_text())
    assert list(data) == ["vulnerabilities"]
    (entry,) = data["vulnerabilities"]
    assert entry["id"] == "CVE-2026-112366"
    assert entry["paths"] == ["site/package-lock.json"]
    assert entry["purls"] == ["pkg:npm/picomatch@4.0.7", "pkg:npm/picomatch@2.3.2"]
    assert "slopvac-8l4.14.44" in entry["statement"]
    assert entry["expired_at"] == datetime.date(2026, 12, 18)


@pytest.mark.parametrize("job", JOBS)
def test_trivy_gate_cannot_be_bypassed_by_continue_on_error(job: str) -> None:
    workflow_job = load_workflow()["jobs"][job]
    assert "continue-on-error" not in workflow_job
    assert not [s for s in workflow_job["steps"] if "continue-on-error" in s]


def test_secret_job_keeps_read_only_permissions() -> None:
    assert load_workflow()["jobs"]["trivy-secret"]["permissions"] == {
        "contents": "read"
    }


@pytest.mark.parametrize("job", JOBS)
def test_trivy_comes_from_the_pinned_setup_action_at_the_main_version(
    job: str,
) -> None:
    found = [
        s
        for s in steps_of(job)
        if str(s.get("uses", "")).startswith("aquasecurity/setup-trivy@")
    ]
    assert len(found) == 1, f"{job}: expected one setup-trivy step"
    setup = found[0]
    ref = setup["uses"].split("@")[1].split()[0]
    assert len(ref) == 40 and set(ref) <= set("0123456789abcdef")
    assert setup["with"]["version"] == MAIN_POLICY[job]["version"]


@pytest.mark.parametrize("job", JOBS)
def test_no_step_sets_a_trivy_environment_variable(job: str) -> None:
    for step in steps_of(job):
        assert not [k for k in step.get("env", {}) if k.startswith("TRIVY_")]
    assert not [k for k in load_workflow()["jobs"][job].get("env", {})]


# --- trusted config -----------------------------------------------------------


def run_text(job: str, script: str) -> str:
    return step_running(job, script)["run"]


def writer_severities(job: str) -> str:
    run = run_text(job, WRITER)
    assert run.strip().startswith(f"bash {WRITER} ")
    assert "${{" not in run  # fixed text, never an expression
    return run.split()[-1]


@pytest.mark.parametrize("job", JOBS)
def test_trusted_config_carries_the_severity_policy_of_main(
    job: str, tmp_path: Path
) -> None:
    done = subprocess.run(
        ["bash", str(ROOT / WRITER), writer_severities(job)],
        env={"RUNNER_TEMP": str(tmp_path), "PATH": "/usr/bin:/bin"},
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 0, done.stderr
    base = tmp_path / "trivy-trusted"
    config = yaml.safe_load((base / "trivy.yaml").read_text())
    assert set(config) == {"severity"}
    assert set(config["severity"]) == MAIN_POLICY[job]["severities"]
    assert (base / "trivy-secret.yaml").read_text().strip() == "{}"


def test_trusted_config_rejects_an_unknown_severity(tmp_path: Path) -> None:
    done = subprocess.run(
        ["bash", str(ROOT / WRITER), "HIGH,NONE"],
        env={"RUNNER_TEMP": str(tmp_path), "PATH": "/usr/bin:/bin"},
        capture_output=True,
        text=True,
        check=False,
    )
    assert done.returncode == 2


# --- scan harness: the real step text against a stub trivy ----------------------

STUB = """#!/usr/bin/env bash
printf "%s\\n" "$@" > "$STUB_ARGV"
out=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--output" ]; then out="$2"; fi
  shift
done
if [ -n "${STUB_SARIF:-}" ]; then cp "$STUB_SARIF" "$out"; fi
printf "%s\\n" "::error::injected" "##[group]injected"
printf "%s\\n" "::error::injected" "##[group]injected" >&2
exit "$STUB_EXIT"
"""


def finding(rule: str, text: str, uri: str) -> dict:
    return {
        "ruleId": rule,
        "level": "error",
        "message": {"text": text},
        "locations": [{"physicalLocation": {"artifactLocation": {"uri": uri}}}],
    }


def hostile_sarif() -> dict:
    result = finding(HOSTILE, GROUP + "\nsecond", "a::warning::b")
    return {"runs": [{"results": [result]}]}


class Scan:
    def __init__(self, job: str, tmp_path: Path, exit_code: int, sarif: bool) -> None:
        self.ws = tmp_path / "ws"
        self.runner_temp = tmp_path / "rt"
        self.ws.mkdir()
        self.runner_temp.mkdir()
        bin_dir = tmp_path / "bin"
        bin_dir.mkdir()
        stub = bin_dir / "trivy"
        stub.write_text(STUB)
        stub.chmod(stub.stat().st_mode | stat.S_IXUSR)
        self.argv_file = tmp_path / "argv.txt"
        sarif_file = tmp_path / "stub.sarif"
        sarif_file.write_text(json.dumps(hostile_sarif()))
        env = {
            "PATH": f"{bin_dir}:{Path(sys.executable).parent}:/usr/bin:/bin",
            "RUNNER_TEMP": str(self.runner_temp),
            "STUB_ARGV": str(self.argv_file),
            "STUB_EXIT": str(exit_code),
        }
        if sarif:
            env["STUB_SARIF"] = str(sarif_file)
        command = run_text(job, SCAN).replace(SCAN, str(ROOT / SCAN))
        self.done = subprocess.run(
            ["bash", "-c", command],
            cwd=self.ws,
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )

    def flags(self) -> dict[str, str]:
        argv = self.argv_file.read_text().splitlines()
        assert argv[:2] == ["fs", "."]
        pairs = {}
        for i, arg in enumerate(argv):
            if arg.startswith("--"):
                nxt = argv[i + 1] if i + 1 < len(argv) else ""
                pairs[arg] = "" if nxt.startswith("--") else nxt
        return pairs


@pytest.mark.parametrize("job", JOBS)
def test_scan_runs_with_the_exact_policy_of_main(job: str, tmp_path: Path) -> None:
    want = MAIN_POLICY[job]
    flags = Scan(job, tmp_path, 0, sarif=True).flags()
    assert set(flags["--scanners"].split(",")) == want["scanners"]
    assert set(flags["--severity"].split(",")) == want["severities"]
    assert flags["--exit-code"] == want["exit_code"]
    assert flags["--ignorefile"] == want["ignorefile"]
    assert flags["--format"] == "sarif"
    assert not [f for f in flags if f.startswith("--skip") or f == "--ignore-unfixed"]


@pytest.mark.parametrize("job", JOBS)
def test_trusted_config_and_scan_flags_share_one_severity_policy(job: str) -> None:
    words = run_text(job, SCAN).split()
    assert writer_severities(job) == words[-2]


@pytest.mark.parametrize("job", JOBS)
def test_scan_reads_config_and_cache_only_from_the_runner_temp(
    job: str, tmp_path: Path
) -> None:
    scan = Scan(job, tmp_path, 0, sarif=True)
    flags = scan.flags()
    temp = str(scan.runner_temp)
    assert flags["--config"] == f"{temp}/trivy-trusted/trivy.yaml"
    assert flags["--secret-config"] == f"{temp}/trivy-trusted/trivy-secret.yaml"
    assert flags["--cache-dir"].startswith(temp + "/")
    assert str(scan.ws) not in " ".join(flags.values())


@pytest.mark.parametrize("job", JOBS)
@pytest.mark.parametrize("code", [0, 1, 3], ids=["pass", "findings", "fatal"])
def test_scan_keeps_the_exit_code_and_prints_nothing_raw(
    job: str, code: int, tmp_path: Path
) -> None:
    done = Scan(job, tmp_path, code, sarif=True).done
    assert done.returncode == code
    both = done.stdout + done.stderr
    assert HOSTILE not in both
    assert GROUP not in both
    assert "##[" not in both
    for line in both.splitlines():
        assert not line.startswith("::")
    if code == 0:
        assert both == ""
    else:
        assert done.stdout.splitlines() == [
            f"Trivy scan failed (exit {code})",
            (
                "%3A%3Aerror%3A%3Ainjected [error] \\u{23}#[group]injected"
                " @ a%3A%3Awarning%3A%3Ab"
            ),
        ]
        assert done.stderr == ""


@pytest.mark.parametrize("job", JOBS)
def test_scan_failure_without_a_report_prints_two_fixed_lines(
    job: str, tmp_path: Path
) -> None:
    done = Scan(job, tmp_path, 1, sarif=False).done
    assert done.returncode == 1
    assert done.stdout.splitlines() == [
        "Trivy scan failed (exit 1)",
        "Trivy report is missing or unreadable",
    ]
    assert done.stderr == ""


# --- cache ---------------------------------------------------------------------


def trivy_cache_paths() -> list[tuple[str, str]]:
    """(where, path) for every actions/cache step that caches Trivy data."""
    found = []
    for path in sorted(WORKFLOWS.glob("*.y*ml")):
        for name, job in (yaml.safe_load(path.read_text()).get("jobs") or {}).items():
            for step in job.get("steps") or []:
                where = str(step.get("with", {}).get("path", ""))
                if str(step.get("uses", "")).startswith("actions/cache") and (
                    "trivy" in where
                ):
                    found.append((f"{path.name}:{name}", where))
    return found


def test_the_trivy_database_cache_steps_are_found() -> None:
    assert {w for w, _ in trivy_cache_paths()} == {
        "security.yml:trivy",
        "security.yml:trivy-secret",
    }


@pytest.mark.parametrize(("where", "path"), trivy_cache_paths())
def test_trivy_cache_is_outside_the_checkout(where: str, path: str) -> None:
    # A change under review can plant a vulnerability database in the checkout,
    # which would hide findings. The cache lives under the runner temp directory.
    assert path.startswith("${{ runner.temp }}/"), f"{where}: cache path {path}"
    assert "github.workspace" not in path


# --- reporter ------------------------------------------------------------------

UNUSABLE = "Trivy report is missing or unreadable\n"


def run_reporter(report: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(ROOT / REPORTER), str(report)],
        capture_output=True,
        text=True,
        check=False,
    )


@pytest.mark.parametrize(
    "content",
    [None, "", "{not json", "[1]", json.dumps({"runs": "x"})],
    ids=["missing", "empty", "malformed", "wrong-shape", "wrong-runs"],
)
def test_reporter_fails_with_a_fixed_message_on_an_unusable_report(
    content: str | None, tmp_path: Path
) -> None:
    report = tmp_path / "secret-dir-name" / "report.sarif"
    report.parent.mkdir()
    if content is not None:
        report.write_text(content)
    done = run_reporter(report)
    assert done.returncode != 0
    assert done.stdout == UNUSABLE
    assert done.stderr == ""


def test_reporter_prints_one_line_per_finding(tmp_path: Path) -> None:
    report = tmp_path / "r.sarif"
    results = [
        finding("CVE-2026-0001", "first line\nsecond line", "a/lock.json"),
        {"ruleId": "AVD-X-1", "level": "warning", "message": {"text": "no place"}},
    ]
    report.write_text(json.dumps({"runs": [{"results": results}, {}]}))
    done = run_reporter(report)
    assert done.returncode == 0
    assert done.stdout.splitlines() == [
        "CVE-2026-0001 [error] first line @ a/lock.json",
        "AVD-X-1 [warning] no place @ ?",
    ]


@pytest.mark.parametrize("field", ["rule", "text", "uri"])
def test_reporter_escapes_every_field(field: str, tmp_path: Path) -> None:
    values = {"rule": "CVE-1", "text": "t", "uri": "a"}
    values[field] = HOSTILE
    report = tmp_path / "r.sarif"
    result = finding(values["rule"], values["text"], values["uri"])
    report.write_text(json.dumps({"runs": [{"results": [result]}]}))
    done = run_reporter(report)
    assert done.returncode == 0
    assert "::" not in done.stdout
    assert len(done.stdout.splitlines()) == 1


# --- the installer must not remove files from the scanned checkout ---------------
#
# aquasecurity/setup-trivy@3fb12ec (action.yaml, steps "Checkout install script" and
# "Install Trivy") checks the Trivy repository out into ./trivy under the workspace,
# and later runs `rm -rf ./trivy`. Its only path input moves the install directory,
# not that checkout. Run before the repository checkout, it removes nothing of ours.

DESTRUCTIVE = ("rm ", "git clean", "git checkout", "git reset", "mv ")


def index_of(steps: list[dict], prefix: str) -> int:
    found = [
        i for i, s in enumerate(steps) if str(s.get("uses", "")).startswith(prefix)
    ]
    assert len(found) == 1, f"expected one step that uses {prefix}"
    return found[0]


def scan_index(steps: list[dict]) -> int:
    found = [i for i, s in enumerate(steps) if SCAN in str(s.get("run", ""))]
    assert len(found) == 1, f"expected one step that runs {SCAN}"
    return found[0]


@pytest.mark.parametrize("job", JOBS)
def test_no_step_between_checkout_and_scan_can_remove_workspace_files(
    job: str,
) -> None:
    steps = steps_of(job)
    checkout = index_of(steps, "actions/checkout@")
    between = steps[checkout + 1 : scan_index(steps)]
    for step in between:
        name = step.get("name", step.get("uses"))
        # The installer deletes ./trivy from the workspace (see above).
        assert not str(step.get("uses", "")).startswith("aquasecurity/setup-trivy@"), (
            f"{job}: step {name} runs after the checkout and removes ./trivy"
        )
        for token in DESTRUCTIVE:
            assert token not in str(step.get("run", "")), f"{job}: step {name}"


def apply_effects(steps: list[dict], workspace: Path) -> None:
    """Model what each step does to the workspace, from the cited action sources.

    actions/checkout puts the repository files in; setup-trivy makes ./trivy and then
    removes it. Every other step in these jobs leaves the workspace alone, which the
    test above checks for the run steps.
    """
    for step in steps:
        uses = str(step.get("uses", ""))
        if uses.startswith("actions/checkout@"):
            (workspace / "trivy").mkdir(exist_ok=True)
            (workspace / "trivy" / "probe.txt").write_text("a file under trivy/\n")
            (workspace / "main.txt").write_text("another file\n")
        elif uses.startswith("aquasecurity/setup-trivy@"):
            (workspace / "trivy").mkdir(exist_ok=True)
            shutil.rmtree(workspace / "trivy")


@pytest.mark.parametrize("job", JOBS)
def test_a_file_under_trivy_dir_reaches_the_scan(job: str, tmp_path: Path) -> None:
    steps = steps_of(job)
    apply_effects(steps[: scan_index(steps)], tmp_path)
    probe = tmp_path / "trivy" / "probe.txt"
    assert probe.exists(), f"{job}: a file under trivy/ was removed before the scan"
    assert probe.read_text() == "a file under trivy/\n"
    assert (tmp_path / "main.txt").exists()


# --- every Trivy call goes through the output boundary -----------------------------

# A command word: at a line start or after ; & | ( , past sudo, env and VAR=value.
TRIVY_CALL = re.compile(
    r"(?:^|[;&|(])\s*(?:sudo\s+|env\s+|\w+=\S+\s+)*trivy(?:\s|$)|aquasec/trivy"
)


def unsafe_trivy_steps(workflow: dict, file: str) -> list[str]:
    """Label (file:job:step) of each step that runs Trivy outside run_trivy.sh.

    The boundary script is the one place that calls trivy, with both streams in files.
    A trivy-action step runs Trivy inside the action, and a bare trivy command in a run
    step prints to the job log, so both are unsafe.
    """
    bad = []
    for job_name, job in (workflow.get("jobs") or {}).items():
        for step in job.get("steps") or []:
            label = f"{file}:{job_name}:{step.get('name', step.get('uses', '?'))}"
            uses = str(step.get("uses", ""))
            if "trivy-action" in uses or "aquasec/trivy" in uses:
                bad.append(label)
                continue
            lines = [
                line
                for line in str(step.get("run", "")).splitlines()
                if not line.lstrip().startswith("#")
            ]
            if any(TRIVY_CALL.search(line) for line in lines):
                bad.append(label)
    return bad


def workflow_with_step(step: dict) -> dict:
    return {"jobs": {"scan": {"steps": [{"name": "Checkout"}, step]}}}


BAD_STEPS = {
    "trivy-action": {
        "name": "Scan",
        "uses": "aquasecurity/trivy-action@ed142fd0673e97e23eac54620cfb913e5ce36c25",
    },
    "bare-trivy-fs": {"name": "Scan", "run": "trivy fs ."},
    "trivy-after-sudo": {"name": "Scan", "run": "set -e\nsudo trivy fs --quiet ."},
    "trivy-in-a-pipe": {"name": "Scan", "run": "echo go && trivy fs . | tee out"},
    "docker-image": {"name": "Scan", "uses": "docker://aquasec/trivy:0.76.0"},
}


@pytest.mark.parametrize("step", BAD_STEPS.values(), ids=list(BAD_STEPS))
def test_a_trivy_call_outside_the_boundary_is_reported_with_its_step(
    step: dict,
) -> None:
    assert unsafe_trivy_steps(workflow_with_step(step), "bad.yml") == [
        "bad.yml:scan:Scan"
    ]


@pytest.mark.parametrize(
    "step",
    [
        {"name": "Scan", "run": "bash scripts/ci/run_trivy.sh secret HIGH out.sarif"},
        {"name": "Install", "uses": "aquasecurity/setup-trivy@abc"},
        {"name": "Note", "run": "# trivy fs . is run by the script\necho done"},
    ],
    ids=["boundary-script", "installer", "comment"],
)
def test_the_boundary_script_and_the_installer_are_allowed(step: dict) -> None:
    assert unsafe_trivy_steps(workflow_with_step(step), "ok.yml") == []


@pytest.mark.parametrize("path", sorted(WORKFLOWS.glob("*.y*ml")), ids=lambda p: p.name)
def test_every_trivy_call_in_the_repository_goes_through_the_boundary(
    path: Path,
) -> None:
    assert unsafe_trivy_steps(yaml.safe_load(path.read_text()), path.name) == []


@pytest.mark.parametrize("job", JOBS)
@pytest.mark.parametrize("stream", ["stdout", "stderr"])
def test_the_boundary_keeps_trivy_streams_in_files_under_the_runner_temp(
    job: str, stream: str, tmp_path: Path
) -> None:
    scan = Scan(job, tmp_path, 1, sarif=True)
    kept = scan.runner_temp / "trivy-logs" / f"{stream}.txt"
    assert kept.read_text() == f"{HOSTILE}\n{GROUP}\n"
