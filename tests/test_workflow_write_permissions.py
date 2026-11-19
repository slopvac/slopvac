"""Workflows write only through minted App tokens, apart from the approved departures.

docs/engineering.md (Supply chain and documentation) says CI write credentials are
GitHub App installation tokens and GITHUB_TOKEN only reads. The approved departures
table lists the exceptions. This module reads every workflow and checks each job's
effective GITHUB_TOKEN grants against that list, so a new write grant fails here until
it either moves to a minted token or gets a recorded departure.
"""

from __future__ import annotations

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
        ("candidate.yml", "prerelease"),
        ("release.yml", "build-local-artifacts"),
    },
    "pages": {("docs-deploy.yml", "deploy")},
}

# Jobs that write to the repository, and so must do it through a minted App token.
MINTED_WRITERS = [
    ("release.yml", "host"),
    ("release.yml", "publish-legacy-release"),
    ("candidate.yml", "prerelease"),
    ("release-please.yml", "release-please"),
    ("pin-check.yml", "pin-check"),
    ("reviewdog.yml", "comment"),
    ("reviewdog.yml", "comment-v3"),
]


def load_workflows() -> dict[str, dict]:
    # BaseLoader keeps every scalar a string, so `on` stays a key and not a boolean.
    return {
        path.name: yaml.load(path.read_text(), Loader=yaml.BaseLoader)
        for path in sorted(WORKFLOWS.glob("*.yml"))
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


def mint_steps(job: dict) -> list[dict]:
    return [
        s
        for s in job.get("steps", [])
        if str(s.get("uses", "")).startswith(MINT_ACTION)
    ]


def minted_writes(job: dict) -> bool:
    return any(
        key.startswith("permission-") and value == "write"
        for step in mint_steps(job)
        for key, value in step.get("with", {}).items()
    )


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


@pytest.mark.parametrize(("name", "job_id"), MINTED_WRITERS)
def test_a_repository_writer_mints_its_token_and_holds_no_write_grant(name, job_id):
    workflow = load_workflows()[name]
    job = workflow["jobs"][job_id]
    assert minted_writes(job), "no create-github-app-token step with a write permission"
    assert written_scopes(workflow, job) - OIDC - set(DEPARTURES) == set()


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
