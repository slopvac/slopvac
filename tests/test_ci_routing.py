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
    assert set(JOBS["changes"]["outputs"]) - {"force_all"} == set(CLASSES)


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
    [("evals-ratchet", "evals"), ("docs-site", "docs-site")],
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


@pytest.mark.parametrize("job", ["evals-ratchet", "docs-site"])
def test_folded_job_calls_its_workflow_with_read_only_token(job):
    call = JOBS[job]
    assert call["uses"] == f"./.github/workflows/{job}.yml"
    assert call["permissions"] == {"contents": "read"}


# --- workflow triggers and permissions ---------------------------------------


@pytest.mark.parametrize("workflow", ["evals-ratchet.yml", "docs-site.yml"])
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
    "LINT": "lint",
    "DOCS": "docs",
    "SECRETS": "secrets",
    "DEPENDENCIES": "dependencies",
    "FILESYSTEM": "filesystem",
    "EVALS": "evals",
    "DOCS_SITE": "docs-site",
}
ALWAYS = {"changes", "quick-lint"}


def run_gate(
    flags: set[str], results: dict[str, str], *, draft: bool = False
) -> subprocess.CompletedProcess:
    needs = {name: {"result": results.get(name, "skipped")} for name in GATE_NEEDS}
    env = {k: "true" if v in flags else "false" for k, v in FLAG_ENV.items()}
    env["RESULTS"] = json.dumps(needs)
    env["DRAFT"] = "true" if draft else "false"
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
    assert set(GATE_STEP["env"]) >= set(FLAG_ENV) | {"RESULTS", "DRAFT"}


def test_gate_needs_include_the_folded_jobs():
    assert {"evals-ratchet", "docs-site"} <= set(GATE_NEEDS)


@pytest.mark.parametrize(
    ("flags", "results"),
    [
        (set(), ok()),
        ({"evals"}, ok("evals-ratchet")),
        ({"docs-site"}, ok("docs-site")),
        ({"evals", "docs-site"}, ok("evals-ratchet", "docs-site")),
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
