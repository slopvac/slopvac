"""Workflows write only through minted App tokens, apart from the approved departures.

docs/engineering.md (Supply chain and documentation) says CI write credentials are
GitHub App installation tokens and GITHUB_TOKEN only reads. The approved departures
table lists the exceptions. This module reads every workflow and checks each job's
effective GITHUB_TOKEN grants against that list, so a new write grant fails here until
it either moves to a minted token or gets a recorded departure.
"""

from __future__ import annotations

from copy import deepcopy
from dataclasses import dataclass
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github" / "workflows"
MINT_ACTION = "actions/create-github-app-token@"

# `id-token: write` is an OIDC request, not a repository write, so the rule leaves it out.
OIDC = {"id-token"}

# Approved departures: scope -> the (workflow file, job) pairs that hold it today.
# Add a pair only together with a row in docs/engineering.md.
DEPARTURES: dict[str, set[tuple[str, str]]] = {
    "packages": {("revocations.yml", "push")},
    "security-events": {
        ("lint.yml", "action"),
        ("security.yml", "codeql"),
        ("security.yml", "osv"),
        ("security.yml", "trivy"),
        ("test.yml", "lint"),
        ("test.yml", "security"),
    },
    "attestations": {
        ("publish-pypi.yml", "prerelease"),
        ("release.yml", "build-local-artifacts"),
    },
    "pages": {("docs-deploy.yml", "deploy")},
}

REPOSITORY = "${{ github.event.repository.name }}"
OWNER = "${{ github.repository_owner }}"


@dataclass(frozen=True)
class Writer:
    """A job that writes through a minted token, and the steps that must use it."""

    workflow: str
    job: str
    mint_id: str
    grants: tuple[str, ...]  # the permission-* inputs, all at `write`
    consumers: tuple[tuple[str, str, str], ...]  # (step id or name, section, key)


POST = (
    ("Post review comments", "env", "GH_TOKEN"),
    ("Post review comments", "env", "REVIEWDOG_GITHUB_API_TOKEN"),
)

# Jobs that write to the repository, and so must do it through a minted App token.
MINTED_WRITERS = [
    Writer(
        "release.yml",
        "host",
        "release-token",
        ("contents",),
        (
            ("Upload the artifacts to the draft release", "env", "GH_TOKEN"),
            ("Publish the release", "env", "GH_TOKEN"),
        ),
    ),
    Writer(
        "release.yml",
        "publish-legacy-release",
        "release-token",
        ("contents",),
        (("Publish the release", "env", "GH_TOKEN"),),
    ),
    Writer(
        "publish-pypi.yml",
        "prerelease",
        "release-token",
        ("contents",),
        (("Create the prerelease", "env", "GH_TOKEN"),),
    ),
    Writer(
        "release-please.yml",
        "release-please",
        "app-token",
        ("contents", "pull-requests"),
        (("Run release-please", "with", "token"),),
    ),
    Writer(
        "pin-check.yml",
        "pin-check",
        "issue-token",
        ("issues",),
        (("Open, update or close the issue", "env", "GH_TOKEN"),),
    ),
    Writer("reviewdog.yml", "comment", "comment-token", ("pull-requests",), POST),
    Writer("reviewdog.yml", "comment-v3", "comment-token", ("pull-requests",), POST),
]
WRITER_IDS = [f"{w.workflow}:{w.job}" for w in MINTED_WRITERS]


def workflow_paths(directory: Path) -> list[Path]:
    """Every workflow GitHub runs: both `.yml` and `.yaml` files."""
    return sorted([*directory.glob("*.yml"), *directory.glob("*.yaml")])


def load_workflows(directory: Path = WORKFLOWS) -> dict[str, dict]:
    # BaseLoader keeps every scalar a string, so `on` stays a key and not a boolean.
    return {
        path.name: yaml.load(path.read_text(), Loader=yaml.BaseLoader)
        for path in workflow_paths(directory)
    }


def effective_permissions(workflow: dict, job: dict) -> dict[str, str] | None:
    """The job's own `permissions:`, else the workflow's; None when neither is set."""
    block = job.get("permissions", workflow.get("permissions"))
    if block is None:
        return None
    if block == "write-all":
        return {"*": "write"}
    if block == "read-all":
        return {"*": "read"}
    assert isinstance(block, dict), f"unsupported permissions syntax: {block!r}"
    return block


def written_scopes(workflow: dict, job: dict) -> set[str]:
    grants = effective_permissions(workflow, job) or {}
    return {scope for scope, level in grants.items() if level == "write"}


def find_step(job: dict, ident: str) -> tuple[int, dict]:
    for index, step in enumerate(job.get("steps", [])):
        if ident in (step.get("id"), step.get("name")):
            return index, step
    raise LookupError(ident)


def token_problems(job: dict, writer: Writer) -> list[str]:
    """Everything that stops the job's writes from using its minimal, minted token."""
    try:
        mint_at, mint = find_step(job, writer.mint_id)
    except LookupError:
        return ["no mint step"]
    problems = []
    if not str(mint.get("uses", "")).startswith(MINT_ACTION):
        problems.append("the mint step does not use create-github-app-token")
    inputs = mint.get("with", {})
    granted = {k: v for k, v in inputs.items() if k.startswith("permission-")}
    if granted != {f"permission-{scope}": "write" for scope in writer.grants}:
        problems.append(f"grant is {granted}")
    if inputs.get("repositories") != REPOSITORY or inputs.get("owner", OWNER) != OWNER:
        problems.append("the token is not scoped to this repository")
    if "continue-on-error" in mint or "continue-on-error" in job:
        problems.append("a failed mint can be ignored")
    expected = f"${{{{ steps.{writer.mint_id}.outputs.token }}}}"
    for ident, section, key in writer.consumers:
        try:
            at, step = find_step(job, ident)
        except LookupError:
            problems.append(f"no consuming step {ident}")
            continue
        if at < mint_at:
            problems.append(f"{ident} runs before the mint")
        if "continue-on-error" in step:
            problems.append(f"{ident} can fail silently")
        if step.get(section, {}).get(key) != expected:
            problems.append(f"{ident} {key} is not the minted token")
        used = str(step.get("env", {})) + str(step.get("with", {}))
        if "GITHUB_TOKEN" in used or "github.token" in used:
            problems.append(f"{ident} names the workflow token")
    return problems


def violations(workflows: dict[str, dict]) -> list[str]:
    """Each GITHUB_TOKEN write, or missing permissions block, that nothing excuses."""
    found = []
    for name, workflow in workflows.items():
        for job_id, job in workflow["jobs"].items():
            if effective_permissions(workflow, job) is None:
                found.append(f"{name}:{job_id} sets no permissions")
            for scope in sorted(written_scopes(workflow, job) - OIDC):
                if (name, job_id) not in DEPARTURES.get(scope, set()):
                    found.append(f"{name}:{job_id} writes {scope} with GITHUB_TOKEN")
    return found


def test_no_job_writes_with_github_token_beyond_the_approved_departures():
    assert violations(load_workflows()) == []


@pytest.mark.parametrize("writer", MINTED_WRITERS, ids=WRITER_IDS)
def test_a_repository_writer_uses_a_minimal_minted_token(writer):
    job = load_workflows()[writer.workflow]["jobs"][writer.job]
    assert token_problems(job, writer) == []


@pytest.mark.parametrize("writer", MINTED_WRITERS, ids=WRITER_IDS)
def test_a_repository_writer_holds_no_github_token_write_beyond_a_departure(writer):
    workflow = load_workflows()[writer.workflow]
    job = workflow["jobs"][writer.job]
    assert written_scopes(workflow, job) - OIDC - set(DEPARTURES) == set()


def _mint(job, writer):
    return find_step(job, writer.mint_id)[1]


def _first_consumer(job, writer):
    ident, section, key = writer.consumers[0]
    return find_step(job, ident)[1][section], key


def _swap_token(value):
    def mutate(job, writer):
        holder, key = _first_consumer(job, writer)
        holder[key] = value

    return mutate


def _fall_back(job, writer):
    holder, key = _first_consumer(job, writer)
    holder[key] += " || secrets.GITHUB_TOKEN"


def _flag_consumer(job, writer):
    find_step(job, writer.consumers[0][0])[1]["continue-on-error"] = "true"


def _widen(job, writer):
    _mint(job, writer)["with"]["permission-actions"] = "write"


def _raise(job, writer):
    _mint(job, writer)["with"][f"permission-{writer.grants[0]}"] = "admin"


def _drop_grant(job, writer):
    del _mint(job, writer)["with"][f"permission-{writer.grants[0]}"]


def _drop_scope(job, writer):
    del _mint(job, writer)["with"]["repositories"]


def _flag_mint(job, writer):
    _mint(job, writer)["continue-on-error"] = "true"


def _remove_mint(job, writer):
    job["steps"].remove(_mint(job, writer))


MUTATIONS = {
    "consumer-uses-GITHUB_TOKEN": _swap_token("${{ secrets.GITHUB_TOKEN }}"),
    "consumer-uses-github-token-context": _swap_token("${{ github.token }}"),
    "consumer-falls-back-to-GITHUB_TOKEN": _fall_back,
    "mint-continues-on-error": _flag_mint,
    "consumer-continues-on-error": _flag_consumer,
    "grant-widened": _widen,
    "grant-raised": _raise,
    "grant-dropped": _drop_grant,
    "repository-scope-dropped": _drop_scope,
    "mint-removed": _remove_mint,
}


@pytest.mark.parametrize("mutation", MUTATIONS)
@pytest.mark.parametrize("writer", MINTED_WRITERS, ids=WRITER_IDS)
def test_a_broken_token_setup_is_reported(writer, mutation):
    job = deepcopy(load_workflows()[writer.workflow]["jobs"][writer.job])
    MUTATIONS[mutation](job, writer)
    assert token_problems(job, writer) != []


@pytest.mark.parametrize("suffix", [".yml", ".yaml"])
def test_workflow_discovery_reads_both_extensions(tmp_path, suffix):
    (tmp_path / f"w{suffix}").write_text("jobs: {}\n")
    (tmp_path / "notes.txt").write_text("jobs: {}\n")
    assert list(load_workflows(tmp_path)) == [f"w{suffix}"]


def test_departures_name_only_jobs_that_still_hold_the_write():
    workflows = load_workflows()
    stale = [
        f"{name}:{job_id} no longer writes {scope}"
        for scope, jobs in DEPARTURES.items()
        for name, job_id in sorted(jobs)
        if scope not in written_scopes(workflows[name], workflows[name]["jobs"][job_id])
    ]
    assert stale == []


def synthetic(workflow_permissions, job_permissions, steps=()):
    job = {"steps": list(steps)}
    if job_permissions is not None:
        job["permissions"] = job_permissions
    doc = {"jobs": {"j": job}}
    if workflow_permissions is not None:
        doc["permissions"] = workflow_permissions
    return {"x.yml": doc}


@pytest.mark.parametrize(
    ("workflow_permissions", "job_permissions", "expected"),
    [
        pytest.param(
            {"contents": "write"},
            None,
            ["x.yml:j writes contents with GITHUB_TOKEN"],
            id="workflow-level",
        ),
        pytest.param(
            {"contents": "read"},
            {"contents": "write"},
            ["x.yml:j writes contents with GITHUB_TOKEN"],
            id="job-level",
        ),
        pytest.param(
            {"contents": "write"},
            {"contents": "read"},
            [],
            id="job-level-narrows-workflow",
        ),
        pytest.param(
            "write-all", None, ["x.yml:j writes * with GITHUB_TOKEN"], id="write-all"
        ),
        pytest.param(
            {"contents": "read"}, {"id-token": "write"}, [], id="oidc-is-not-a-write"
        ),
        pytest.param(
            {"contents": "read"},
            {"pages": "write"},
            ["x.yml:j writes pages with GITHUB_TOKEN"],
            id="departure-is-per-job",
        ),
        pytest.param(None, None, ["x.yml:j sets no permissions"], id="unset"),
    ],
)
def test_violations_read_the_effective_permissions(
    workflow_permissions, job_permissions, expected
):
    assert violations(synthetic(workflow_permissions, job_permissions)) == expected


def test_minted_token_does_not_excuse_a_github_token_write():
    mint = {"uses": MINT_ACTION + "abc", "with": {"permission-contents": "write"}}
    docs = synthetic({"contents": "read"}, {"contents": "write"}, [mint])
    assert violations(docs) == ["x.yml:j writes contents with GITHUB_TOKEN"]
