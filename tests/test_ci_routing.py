"""test.yml routing: path classes, job conditions and the Tests gate behave as classified.

Three layers are exercised by behaviour, not by text:

* path classes in .github/ci-paths.yml, evaluated against sample paths;
* each routed job's `if:` expression, evaluated as a truth table;
* the gate's inline acceptance script, run as a subprocess with crafted inputs.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github" / "workflows"
# BaseLoader keeps every scalar a string, so `on` stays a key and not a boolean.
TEST = yaml.load((WORKFLOWS / "test.yml").read_text(), Loader=yaml.BaseLoader)
CLASSES = yaml.safe_load((ROOT / ".github" / "ci-paths.yml").read_text())
JOBS = TEST["jobs"]
GATE_NEEDS = JOBS["gate"]["needs"]


def load(name: str) -> dict:
    return yaml.load((WORKFLOWS / name).read_text(), Loader=yaml.BaseLoader)


def test_every_path_class_is_a_changes_output_and_vice_versa():
    scope_outputs = {"force_all", "rust_scope", "rust_modified", "rust_affected"}
    assert set(JOBS["changes"]["outputs"]) - scope_outputs == set(CLASSES)


def test_gate_needs_every_job_except_itself_and_its_compat_shims():
    shims = {"lint-gate-compat", "security-gate-compat"}
    assert set(GATE_NEEDS) == set(JOBS) - {"gate"} - shims


# --- path classes -----------------------------------------------------------


def glob_to_regex(pattern: str) -> re.Pattern[str]:
    """Translate a picomatch-style glob, as dorny/paths-filter evaluates it.

    Assumed semantics (picomatch with dot=true): `**/` matches zero or more
    directories, a bare `**` matches anything including `/`, `*` and `?` stay
    inside one path segment, and dotfiles are matched like any other file.
    """
    out, i = [], 0
    while i < len(pattern):
        if pattern.startswith("**/", i):
            out.append("(?:.*/)?")
            i += 3
        elif pattern.startswith("**", i):
            out.append(".*")
            i += 2
        elif pattern[i] == "*":
            out.append("[^/]*")
            i += 1
        elif pattern[i] == "?":
            out.append("[^/]")
            i += 1
        else:
            out.append(re.escape(pattern[i]))
            i += 1
    return re.compile("".join(out) + r"\Z")


def in_class(klass: str, path: str) -> bool:
    return any(glob_to_regex(p).match(path) for p in CLASSES[klass])


@pytest.mark.parametrize(
    ("pattern", "path", "matches"),
    [
        ("evals/**", "evals/gate.py", True),
        ("evals/**", "evalsx/gate.py", False),
        ("**/*.md", "README.md", True),
        ("**/*.md", "docs/a/b.md", True),
        ("*.toml", "crates/Cargo.toml", False),
        ("**", ".github/ci-paths.yml", True),
    ],
)
def test_glob_translation_matches_picomatch_semantics(pattern, path, matches):
    assert bool(glob_to_regex(pattern).match(path)) is matches


@pytest.mark.parametrize(
    "path",
    [
        "schemas/v3/common.schema.json",
        "docs/contract/policies.md",
        "evals/gate.py",
        "packages/slopvac-lint/src/slopvac/cli.py",
        "packages/slopvac-lint/vale/styles-verified/x.yml",
        "scripts/install_vale.py",
        ".github/workflows/evals-ratchet.yml",
    ],
)
def test_evals_class_includes(path):
    assert in_class("evals", path)


@pytest.mark.parametrize("path", ["CLA.md", "tests/test_cla_text.py"])
def test_code_class_runs_the_cla_text_test(path):
    # tests/test_cla_text.py hashes CLA.md, so a change to the agreement text must run the tests.
    assert in_class("code", path)


@pytest.mark.parametrize(
    "path",
    [
        "site/astro.config.mjs",
        "docs/site.toml",
        "docs/site-lint.toml",
        "scripts/docs/build_site.py",
        "tests/docs/test_build.py",
        "schemas/v3/common.schema.json",
        "packages/slopvac-lint/src/slopvac/rules/ai-residue.yml",
        "crates/slopvac-engine/src/lib.rs",
        "Cargo.lock",
        "rust-toolchain.toml",
        "mise.toml",
        ".github/workflows/docs-site.yml",
    ],
)
def test_docs_site_class_includes(path):
    assert in_class("docs-site", path)


@pytest.mark.parametrize(
    "path", ["README.md", "site-notes.txt", "docs/design/x.md", "action.yml"]
)
def test_evals_class_excludes_unrelated_paths(path):
    assert not in_class("evals", path)


@pytest.mark.parametrize(
    "path",
    [
        "README.md",
        "evals/gate.py",
        "docs/contract/policies.md",
        "packages/slopvac-lint/src/slopvac/cli.py",
        "scripts/install_vale.py",
    ],
)
def test_docs_site_class_excludes_unrelated_paths(path):
    assert not in_class("docs-site", path)


# --- job conditions ---------------------------------------------------------

TERM = re.compile(r"needs\.changes\.outputs(?:\.(\w+)|\['([\w-]+)'\]) == 'true'")


def evaluate(condition: str, true_outputs: set[str]) -> bool:
    """Evaluate the `||`/`&&`/`==` subset that test.yml job conditions use."""
    expr = " ".join(condition.split())
    expr = TERM.sub(lambda m: str((m.group(1) or m.group(2)) in true_outputs), expr)
    expr = expr.replace("||", " or ").replace("&&", " and ")
    assert re.fullmatch(r"(?:True|False|or|and|\(|\)|\s)+", expr), expr
    return eval(expr, {"__builtins__": {}})


@pytest.mark.parametrize(
    ("job", "klass"),
    [
        ("evals-ratchet", "evals"),
        ("docs-site", "docs-site"),
        ("action-v3", "action-v3"),
        ("diagram-adapter", "diagram-adapter"),
    ],
)
@pytest.mark.parametrize(
    ("true_outputs", "runs"),
    [
        (set(), False),
        ({"force_all"}, True),
        ({"__class__"}, True),
        ({"code", "rust", "docs", "package"}, False),
    ],
)
def test_folded_job_runs_exactly_when_routed(job, klass, true_outputs, runs):
    outputs = {klass if o == "__class__" else o for o in true_outputs}
    assert evaluate(JOBS[job]["if"], outputs) is runs


@pytest.mark.parametrize(
    ("job", "other"), [("evals-ratchet", "docs-site"), ("docs-site", "evals")]
)
def test_folded_job_ignores_the_other_class(job, other):
    assert evaluate(JOBS[job]["if"], {other}) is False


@pytest.mark.parametrize(
    "path",
    [
        "crates/slopvac-diagrams/src/mermaid.rs",
        "crates/slopvac-engine/src/lint.rs",
        "evals/diagrams.py",
        "tests/test_evals_diagrams.py",
        "tests/fixtures/diagram_eval_output.json",
    ],
)
def test_live_adapter_path_also_runs_the_python_suite(path):
    # Main classifies these paths as `code`, so the Python suite runs with the adapter.
    outputs = classes_of([path])
    assert {"diagram-adapter", "code"} <= outputs
    assert evaluate(JOBS["diagram-adapter"]["if"], outputs) is True
    assert evaluate(JOBS["test"]["if"], outputs) is True


def test_adapter_class_alone_runs_the_adapter_and_not_the_python_suite():
    # The gates are isolated: the adapter job fires on its own class, and the
    # Python suite does not fire on it.
    outputs = {"diagram-adapter"}
    assert evaluate(JOBS["diagram-adapter"]["if"], outputs) is True
    assert evaluate(JOBS["test"]["if"], outputs) is False


@pytest.mark.parametrize("path", ["README.md", "docs/design/slopvac-3.md"])
def test_prose_change_runs_neither_the_adapter_nor_the_python_suite(path):
    outputs = classes_of([path])
    assert evaluate(JOBS["diagram-adapter"]["if"], outputs) is False
    assert evaluate(JOBS["test"]["if"], outputs) is False


def test_live_diagram_adapter_pytest_step_requires_the_real_engine():
    step = next(
        step
        for step in JOBS["diagram-adapter"]["steps"]
        if "pytest" in step.get("run", "")
    )
    assert step.get("env", {}).get("SLOPVAC_EVALS_LIVE_ENGINE") == "1"


@pytest.mark.parametrize(
    ("condition", "outputs", "expected"),
    [
        (
            "needs.changes.outputs.a == 'true' || needs.changes.outputs.b == 'true'",
            {"a"},
            True,
        ),
        (
            "needs.changes.outputs.a == 'true' || needs.changes.outputs.b == 'true'",
            set(),
            False,
        ),
        (
            "needs.changes.outputs.a == 'true' && needs.changes.outputs['b'] == 'true'",
            {"a"},
            False,
        ),
        (
            "needs.changes.outputs.a == 'true' && needs.changes.outputs['b'] == 'true'",
            {"a", "b"},
            True,
        ),
    ],
)
def test_evaluator_distinguishes_or_from_and(condition, outputs, expected):
    assert evaluate(condition, outputs) is expected


def classes_of(paths: list[str]) -> set[str]:
    return {klass for klass in CLASSES for path in paths if in_class(klass, path)}


DOCS_ONLY = ["README.md", "docs/design/slopvac-3.md", "packages/slopvac-lint/README.md"]
PYTHON_ONLY = [
    "packages/slopvac-lint/src/slopvac/report.py",
    "packages/slopvac-lint/tests/test_report.py",
    "tests/test_release_please_config.py",
]
HEAVY = ["rust-static", "rust", "rust-msrv", "rust-supply-chain", "test"]


@pytest.mark.parametrize(
    ("paths", "expected"),
    [
        (DOCS_ONLY, dict.fromkeys(HEAVY, False)),
        (PYTHON_ONLY, {**dict.fromkeys(HEAVY, False), "test": True}),
    ],
    ids=["docs-only", "python-only"],
)
def test_docs_and_python_changes_skip_the_rust_legs(paths, expected):
    outputs = classes_of(paths)
    assert {job: evaluate(JOBS[job]["if"], outputs) for job in HEAVY} == expected


@pytest.mark.parametrize(
    "path",
    [
        "schemas/v3/tagger.schema.json",
        "schemas/v3/conformance/blocklist/invalid/cases.toml",
    ],
)
def test_schema_changes_route_to_rust(path):
    # Rust crates load the schema files and the corpus, so a schema-only change must run Rust CI.
    assert in_class("rust", path)


@pytest.mark.parametrize(
    "path",
    [
        "crates/slopvac-engine/tests/fixtures/x.json",
        "crates/slopvac-rules/tests/fixtures/x.json",
    ],
)
def test_fixtures_read_by_contract_tests_run_the_full_suite(path):
    outputs = classes_of([path])
    assert "code" in outputs
    assert evaluate(JOBS["test"]["if"], outputs) is True


@pytest.mark.parametrize(
    "path",
    [
        "crates/slopvac-report/src/lib.rs",
        "crates/slopvac-engine/src/lib.rs",
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
    ],
)
def test_a_change_to_the_binary_sources_runs_the_real_binary_suites(path):
    outputs = classes_of([path])
    assert {"code", "action-v3"} <= outputs
    assert evaluate(JOBS["test"]["if"], outputs) is True
    assert evaluate(JOBS["action-v3"]["if"], outputs) is True


def test_a_change_to_the_action_alone_runs_its_contract_tests():
    # tests/test_action_v3.py reads action-v3/action.yml, so the full suite must run.
    outputs = classes_of(["action-v3/action.yml"])
    assert {"code", "action-v3"} <= outputs
    assert evaluate(JOBS["test"]["if"], outputs) is True
    assert evaluate(JOBS["action-v3"]["if"], outputs) is True


@pytest.mark.parametrize("path", ["README.md", "docs/design/x.md"])
def test_prose_only_changes_skip_the_action_v3_job(path):
    assert evaluate(JOBS["action-v3"]["if"], classes_of([path])) is False


def test_the_pytest_job_builds_the_binary_before_it_runs_the_suites():
    steps = JOBS["test"]["steps"]
    build = next(s for s in steps if s["name"] == "Build the slopvac binary")
    assert build["run"] == "cargo build --locked -p slopvac"
    assert JOBS["test"]["env"]["SLOPVAC_BIN"].endswith("/target/debug/slopvac")
    names = [s["name"] for s in steps]
    assert names.index("Build the slopvac binary") < names.index(
        "Run every discovered pytest suite"
    )


def test_the_rust_job_allows_user_namespaces_before_the_tests_run():
    steps = JOBS["rust"]["steps"]
    names = [s["name"] for s in steps]
    step = steps[
        names.index("Allow unprivileged user namespaces for the network-sandbox test")
    ]
    assert names.index(step["name"]) < names.index("Test")
    assert step["if"] == "runner.os == 'Linux'"
    assert "kernel.apparmor_restrict_unprivileged_userns=0" in step["run"]
    assert "unshare --user --map-root-user --net true" in step["run"]


@pytest.mark.parametrize(
    ("job", "workflow"),
    [
        ("evals-ratchet", "evals-ratchet.yml"),
        ("docs-site", "docs-site.yml"),
        ("action-v3", "action-v3-test.yml"),
    ],
)
def test_folded_job_calls_its_workflow_with_read_only_token(job, workflow):
    call = JOBS[job]
    assert call["uses"] == f"./.github/workflows/{workflow}"
    assert call["permissions"] == {"contents": "read"}


# --- workflow triggers and permissions ---------------------------------------


@pytest.mark.parametrize(
    "workflow", ["evals-ratchet.yml", "docs-site.yml", "action-v3-test.yml"]
)
def test_folded_workflow_has_no_standalone_pr_or_push_trigger(workflow):
    triggers = load(workflow)["on"]
    assert "workflow_call" in triggers
    assert not {"pull_request", "pull_request_target", "push"} & set(triggers)


def test_pages_permissions_exist_only_on_the_main_deploy_job():
    holders = []
    for name in ("docs-site.yml", "docs-deploy.yml"):
        for job_name, job in load(name)["jobs"].items():
            perms = job.get("permissions") or {}
            if "pages" in perms or "id-token" in perms:
                holders.append((name, job_name, job["if"]))
    assert len(holders) == 1
    name, job_name, condition = holders[0]
    assert (name, job_name) == ("docs-deploy.yml", "deploy")
    assert "github.ref == 'refs/heads/main'" in condition


def test_docs_deploy_runs_on_pushes_to_main():
    assert load("docs-deploy.yml")["on"]["push"]["branches"] == ["main"]


def test_docs_deploy_never_runs_on_pull_requests():
    assert "pull_request" not in load("docs-deploy.yml")["on"]


# --- the Tests gate ---------------------------------------------------------

GATE_STEP = JOBS["gate"]["steps"][0]
GATE_SCRIPT = re.search(
    r"python3 - <<'PY'\n(.*?)\nPY\n", GATE_STEP["run"], re.DOTALL
).group(1)
FLAG_ENV = {
    "FORCE_ALL": "force_all",
    "CODE": "code",
    "PACKAGE": "package",
    "RUST": "rust",
    "ACTION": "action",
    "ACTION_V3": "action-v3",
    "LINT": "lint",
    "DOCS": "docs",
    "SECRETS": "secrets",
    "DEPENDENCIES": "dependencies",
    "FILESYSTEM": "filesystem",
    "EVALS": "evals",
    "DOCS_SITE": "docs-site",
    "DIAGRAM_ADAPTER": "diagram-adapter",
}
ALWAYS = {"changes", "quick-lint"}


def run_gate(
    flags: set[str],
    results: dict[str, str],
    *,
    draft: bool = False,
    event: str = "pull_request",
    scope: tuple[str, str, str] = ("all", "--workspace", "--workspace"),
) -> subprocess.CompletedProcess:
    needs = {name: {"result": results.get(name, "skipped")} for name in GATE_NEEDS}
    env = {k: "true" if v in flags else "false" for k, v in FLAG_ENV.items()}
    env["RESULTS"] = json.dumps(needs)
    env["DRAFT"] = "true" if draft else "false"
    env["EVENT_NAME"] = event
    env["RUST_SCOPE"], env["RUST_MODIFIED"], env["RUST_AFFECTED"] = scope
    return subprocess.run(
        [sys.executable, "-c", GATE_SCRIPT],
        env={**os.environ, **env},
        capture_output=True,
        text=True,
        check=False,
    )


def ok(*names: str) -> dict[str, str]:
    return {name: "success" for name in (*ALWAYS, *names)}


def test_gate_step_declares_an_env_for_every_flag():
    declared = set(FLAG_ENV) | {"RESULTS", "DRAFT", "EVENT_NAME"}
    declared |= {"RUST_SCOPE", "RUST_MODIFIED", "RUST_AFFECTED"}
    assert set(GATE_STEP["env"]) >= declared


def test_gate_needs_include_the_folded_jobs():
    assert {"evals-ratchet", "docs-site"} <= set(GATE_NEEDS)


@pytest.mark.parametrize(
    ("flags", "results"),
    [
        (set(), ok()),
        ({"evals"}, ok("evals-ratchet")),
        ({"docs-site"}, ok("docs-site")),
        ({"action-v3"}, ok("action-v3")),
        ({"evals", "docs-site"}, ok("evals-ratchet", "docs-site")),
        ({"diagram-adapter"}, ok("diagram-adapter")),
        ({"force_all"}, ok(*(set(GATE_NEEDS) - ALWAYS))),
    ],
)
def test_gate_accepts_runs_and_skips_matching_the_classification(flags, results):
    proc = run_gate(flags, results)
    assert proc.returncode == 0, proc.stdout + proc.stderr


@pytest.mark.parametrize(
    ("flags", "results", "message"),
    [
        ({"evals"}, ok(), "evals-ratchet: got skipped, expected success"),
        ({"docs-site"}, ok(), "docs-site: got skipped, expected success"),
        ({"action-v3"}, ok(), "action-v3: got skipped, expected success"),
        (set(), ok("action-v3"), "action-v3: got success, expected skipped"),
        ({"diagram-adapter"}, ok(), "diagram-adapter: got skipped, expected success"),
        (
            set(),
            ok("diagram-adapter"),
            "diagram-adapter: got success, expected skipped",
        ),
        (
            {"diagram-adapter"},
            {**ok(), "diagram-adapter": "failure"},
            "diagram-adapter: got failure, expected success",
        ),
        (set(), ok("evals-ratchet"), "evals-ratchet: got success, expected skipped"),
        (set(), ok("docs-site"), "docs-site: got success, expected skipped"),
        (
            {"evals"},
            {**ok(), "evals-ratchet": "failure"},
            "evals-ratchet: got failure, expected success",
        ),
        (
            {"docs-site"},
            {**ok(), "docs-site": "cancelled"},
            "docs-site: got cancelled, expected success",
        ),
        ({"evals"}, ok("evals-ratchet", "docs-site"), "docs-site: got success"),
        (set(), {**ok(), "changes": "failure"}, "change classification was failure"),
        (set(), {**ok(), "quick-lint": "skipped"}, "quick-lint: got skipped"),
    ],
)
def test_gate_rejects_mismatches_failures_and_cancellations(flags, results, message):
    proc = run_gate(flags, results)
    assert proc.returncode == 1
    assert message in proc.stdout


# --- draft pull requests ----------------------------------------------------


@pytest.mark.parametrize(
    ("flags", "results"),
    [
        # What a draft run produces: every routed job skipped.
        (set(), {}),
        # Even a run where every suite succeeded is not a pass for a draft.
        ({"force_all"}, ok(*(set(GATE_NEEDS) - ALWAYS))),
    ],
    ids=["skipped", "all-green"],
)
def test_gate_fails_a_draft(flags, results):
    proc = run_gate(flags, results, draft=True)
    assert proc.returncode == 1
    assert "draft pull request" in proc.stdout


# A skipped job reports success to branch protection, so the required checks
# must run on a draft to fail it.
@pytest.mark.parametrize("job", ["gate", "lint-gate-compat", "security-gate-compat"])
def test_required_checks_are_never_skipped(job):
    assert JOBS[job]["if"] == "always()"


@pytest.mark.parametrize(
    ("result", "code"), [("success", 0), ("failure", 1), ("skipped", 1)]
)
@pytest.mark.parametrize("job", ["lint-gate-compat", "security-gate-compat"])
def test_compat_checks_pass_only_when_the_gate_passed(job, result, code):
    step = JOBS[job]["steps"][0]
    proc = subprocess.run(
        ["bash", "-c", step["run"]], env={**os.environ, "RESULT": result}, check=False
    )
    assert proc.returncode == code


# --- full suite on the queue and main ---------------------------------------

FORCE_STEP = next(s for s in JOBS["changes"]["steps"] if s.get("id") == "force")
ROUTED_JOBS = sorted(set(GATE_NEEDS) - ALWAYS - {"changes"})


def force_all_for(event: str, routing_changed: bool, tmp_path: Path) -> str:
    out = tmp_path / "output"
    out.touch()
    env = {
        **os.environ,
        "EVENT_NAME": event,
        "ROUTING_CHANGED": "true" if routing_changed else "false",
        "GITHUB_OUTPUT": str(out),
    }
    subprocess.run(["bash", "-c", FORCE_STEP["run"]], env=env, check=True)
    return out.read_text().strip()


def test_changes_job_exposes_the_forced_flag_not_the_raw_routing_filter():
    assert JOBS["changes"]["outputs"]["force_all"] == (
        "${{ steps.force.outputs.force_all }}"
    )


@pytest.mark.parametrize(
    ("event", "routing_changed", "expected"),
    [
        ("merge_group", False, "force_all=true"),
        ("push", False, "force_all=true"),
        ("pull_request", True, "force_all=true"),
        ("pull_request", False, "force_all=false"),
    ],
)
def test_force_all_is_set_for_the_queue_main_and_routing_edits(
    event, routing_changed, expected, tmp_path
):
    assert force_all_for(event, routing_changed, tmp_path) == expected


@pytest.mark.parametrize("event", ["merge_group", "push"])
@pytest.mark.parametrize("job", ROUTED_JOBS)
def test_every_routed_job_runs_on_the_queue_and_main_with_no_class_matched(
    event, job, tmp_path
):
    forced = force_all_for(event, False, tmp_path) == "force_all=true"
    outputs = {"force_all"} if forced else set()
    assert evaluate(JOBS[job]["if"], outputs) is True


@pytest.mark.parametrize("event", ["merge_group", "push"])
def test_gate_expects_every_suite_to_succeed_on_the_queue_and_main(event, tmp_path):
    forced = force_all_for(event, False, tmp_path) == "force_all=true"
    flags = {"force_all"} if forced else set()
    everything = ok(*(set(GATE_NEEDS) - ALWAYS))
    assert run_gate(flags, everything).returncode == 0
    skipped = {**everything, "test": "skipped"}
    assert run_gate(flags, skipped).returncode == 1


@pytest.mark.parametrize(
    "path",
    ["crates/slopvac/src/report/scrub.rs", "docs/engineering.md", "README.md"],
)
def test_a_change_outside_the_filesystem_class_still_gets_the_secret_scan(path):
    # #475 touched only crates/: no filesystem class, but every path is a secrets path.
    outputs = classes_of([path])
    assert "filesystem" not in outputs
    assert "secrets" in outputs
    assert evaluate(JOBS["security"]["if"], outputs) is True
    assert JOBS["security"]["with"]["secrets"] == (
        "${{ needs.changes.outputs.force_all == 'true' || needs.changes.outputs.secrets == 'true' }}"
    )
    scan_job = load("security.yml")["jobs"]["trivy-secret"]
    assert scan_job["if"] == "${{ inputs.secrets }}"


# --- Rust scope in the Tests gate -------------------------------------------

SCOPED = ("scoped", "-p slopvac-fs", "-p slopvac -p slopvac-fs")
RUST_RUN = ok("rust-static", "rust", "rust-msrv", "rust-supply-chain")


@pytest.mark.parametrize(
    ("flags", "event", "scope"),
    [
        ({"rust"}, "pull_request", SCOPED),
        ({"rust"}, "pull_request", ("all", "--workspace", "--workspace")),
        ({"rust", "force_all"}, "merge_group", ("all", "--workspace", "--workspace")),
        ({"rust", "force_all"}, "push", ("all", "--workspace", "--workspace")),
    ],
)
def test_gate_accepts_the_scopes_each_event_may_use(flags, event, scope):
    results = RUST_RUN if flags == {"rust"} else ok(*(set(GATE_NEEDS) - ALWAYS))
    proc = run_gate(flags, results, event=event, scope=scope)
    assert proc.returncode == 0, proc.stdout + proc.stderr


@pytest.mark.parametrize(
    ("flags", "event", "scope", "message"),
    [
        # A queue or main run must never test less than the whole workspace.
        ({"rust", "force_all"}, "merge_group", SCOPED, "scoped where the whole"),
        ({"rust", "force_all"}, "push", SCOPED, "scoped where the whole"),
        ({"rust"}, "push", SCOPED, "scoped where the whole"),
        # A routing edit forces everything even on a pull request.
        ({"rust", "force_all"}, "pull_request", SCOPED, "scoped where the whole"),
        # Scope outputs that are missing or contradict each other fail the gate.
        ({"rust"}, "pull_request", ("", "", ""), "expected all or scoped"),
        (
            {"rust"},
            "pull_request",
            ("all", "-p slopvac-fs", "-p slopvac-fs"),
            "arguments are not --workspace",
        ),
        (
            {"rust"},
            "pull_request",
            ("scoped", "--workspace", "-p slopvac-fs"),
            "scoped where the whole",
        ),
    ],
)
def test_gate_fails_a_scope_that_narrows_what_must_run_in_full(
    flags, event, scope, message
):
    results = RUST_RUN if flags == {"rust"} else ok(*(set(GATE_NEEDS) - ALWAYS))
    proc = run_gate(flags, results, event=event, scope=scope)
    assert proc.returncode == 1
    assert message in proc.stdout


# --- the Rust jobs take their scope from the changes job ---------------------


def step_runs(job: str) -> str:
    return "\n".join(s.get("run", "") for s in JOBS[job]["steps"])


@pytest.mark.parametrize(
    ("job", "variable", "output"),
    [
        ("rust-static", "RUST_MODIFIED", "rust_modified"),
        ("rust", "RUST_AFFECTED", "rust_affected"),
    ],
)
def test_rust_jobs_read_their_scope_from_the_changes_job(job, variable, output):
    assert JOBS[job]["env"][variable] == f"${{{{ needs.changes.outputs.{output} }}}}"
    assert f"${variable}" in step_runs(job)


# --- inputs the Rust crates embed or read from outside crates/ ----------------

INCLUDE = re.compile(
    r"include_(?:str|bytes)!\(\s*"
    r'(?:"(?P<plain>[^"]+)"'
    r'|concat!\(\s*env!\("CARGO_MANIFEST_DIR"\)\s*,\s*"(?P<tail>[^"]+)"\s*,?\s*\))',
)
PARENT_LITERAL = re.compile(r'"(?P<path>(?:\.\./)+[\w./-]+)"')


def crate_sources() -> list[Path]:
    listed = subprocess.run(
        ["git", "ls-files", "--", "crates/*.rs"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.split()
    return [ROOT / name for name in listed]


def crate_of(source: Path) -> Path:
    return ROOT / "crates" / source.relative_to(ROOT / "crates").parts[0]


def embedded_inputs() -> set[str]:
    """Repository paths outside crates/ that a crate source embeds or reads.

    `include_str!`/`include_bytes!` resolve against the including file. Other
    `../` literals (manifest-dir reads in tests and build scripts) resolve
    against the crate directory.
    """
    found: set[str] = set()
    for source in crate_sources():
        text = source.read_text(encoding="utf-8")
        resolved: list[Path] = []
        for match in INCLUDE.finditer(text):
            if match["plain"]:
                resolved.append(source.parent / match["plain"])
            else:
                resolved.append(crate_of(source) / match["tail"].lstrip("/"))
        resolved += [
            crate_of(source) / m["path"] for m in PARENT_LITERAL.finditer(text)
        ]
        for path in resolved:
            normal = Path(os.path.normpath(path))
            if (
                normal == ROOT
                or not normal.exists()
                or normal.is_relative_to(ROOT / "crates")
            ):
                continue
            found.add(normal.relative_to(ROOT).as_posix())
    return found


def covered(patterns: list[str], path: str) -> bool:
    # A directory is covered when a file inside it is.
    probe = f"{path}/x" if (ROOT / path).is_dir() else path
    return any(glob_to_regex(p).match(probe) for p in patterns)


def test_the_scan_finds_the_known_embedded_inputs():
    # Guards the scanner itself: a broken regex would make the routing tests vacuous.
    assert {
        "keys/slopvac-release.pub",
        "docs/contract/compat/profiles/legacy-strict.toml",
        "packages/slopvac-lint/examples/blocklist.toml",
        "tests/fixtures/workflow-escape-vectors.json",
        "docs/site.toml",
        "docs/reference/error-codes.lock",
        "Cargo.lock",
    } <= embedded_inputs()


@pytest.mark.parametrize("path", sorted(embedded_inputs()))
def test_every_input_a_crate_embeds_routes_to_rust(path):
    assert covered(CLASSES["rust"], path)


# The root manifest and lockfile are scoped by cargo-delta itself, by design.
SCOPED_BY_DELTA = {"Cargo.toml", "Cargo.lock"}


@pytest.mark.parametrize("path", sorted(embedded_inputs() - SCOPED_BY_DELTA))
def test_every_input_a_crate_embeds_trips_a_cargo_delta_wire(path):
    wires = tomllib.loads((ROOT / ".github" / "cargo-delta.toml").read_text())[
        "trip_wire_patterns"
    ]
    # cargo-delta follows include macros inside crates only, so outside paths need a wire.
    assert covered(wires, path)


@pytest.mark.parametrize(
    "path",
    [
        ".cargo/config.toml",
        "docs/contract/compat/dispositions.toml",
        "docs/contract/compat/removed-rules.toml",
        "packages/slopvac-lint/src/slopvac/rules/prose-craft.yaml",
        "packages/slopvac-lint/tests/fixtures/regression/issues/x.md",
        "docs/reference/cli.md",
        "docs/migrate-2-to-3.md",
        "docs/model-recommendations.md",
        "packages/slopvac-plugin/pyproject.toml",
        "scripts/revocation_list.py",
        "tests/golden/v3/inputs.toml",
        "evals/corpus/x.md",
        ".github/ISSUE_TEMPLATE/bug.yml",
    ],
)
def test_files_the_crate_tests_read_by_path_route_to_rust(path):
    # Reads built from CARGO_MANIFEST_DIR or a repository root are not include macros.
    assert in_class("rust", path)
