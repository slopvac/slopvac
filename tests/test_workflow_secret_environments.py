"""A job may read a minisign signing secret only from inside a deployment environment.

A job may read a signing secret only from inside a reviewed environment, so a
deployment policy (the refs that may deploy) and optional reviewers guard every read,
and a repository-level copy is never what a job reads. A job without `environment:`
resolves to repository secrets, which a workflow run on any ref can read. This test parses each workflow and flags a `MINISIGN_*` secret reference
(or a computed `secrets[...]` index, which may name one) in a job that names no
environment, wherever the reference sits: job or step `env`, step `with`, `run`, or a
reusable-workflow `secrets` map.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest
import yaml

WORKFLOWS = Path(__file__).resolve().parent.parent / ".github" / "workflows"

# `secrets.MINISIGN_X`, `secrets['MINISIGN_X']` and `secrets["MINISIGN_X"]`, or an index
# that is not a single string literal (`secrets[format(...)]`, `secrets[env.NAME]`), which
# may resolve to a signing secret. The secrets context is case-insensitive.
SECRET_READ = re.compile(
    r"""(?<![\w.\-])secrets\s*(?:
        (?:\.|\[\s*['"])\s*MINISIGN_
        | \[(?!\s*(?:'[^']*'|"[^"]*")\s*\])
    )""",
    re.IGNORECASE | re.VERBOSE,
)
SECRET = "${{ secrets.MINISIGN_SECRET_KEY }}"


def load_workflows(directory: Path = WORKFLOWS) -> dict[str, dict]:
    # BaseLoader keeps every scalar a string, so `on` stays a key and not a boolean.
    paths = sorted([*directory.glob("*.yml"), *directory.glob("*.yaml")])
    return {p.name: yaml.load(p.read_text(), Loader=yaml.BaseLoader) for p in paths}


def reads_signing_secret(node: object) -> bool:
    """True when any string under `node` (keys included) names a MINISIGN secret."""
    if isinstance(node, str):
        return SECRET_READ.search(node) is not None
    if isinstance(node, dict):
        return any(
            reads_signing_secret(k) or reads_signing_secret(v) for k, v in node.items()
        )
    if isinstance(node, list):
        return any(reads_signing_secret(item) for item in node)
    return False


# Reviewed environments that may hold the signing key. Each is a literal name; an
# expression or any other name is rejected, so a new home for the key needs a change here.
ALLOWED_ENVIRONMENTS = frozenset({"release", "revocations", "candidate-signing"})

# A single-quoted literal; `''` inside it is an escaped quote. `}}` in one does not end
# the expression.
STRING_LITERAL = r"'(?:[^']|'')*'"
EXPRESSION = re.compile(
    rf"\$\{{\{{((?:{STRING_LITERAL}|[^'}}]|}}(?!}})|')*?)(?:\}}\}}|\Z)", re.DOTALL
)
# `secrets` as a whole context: not a named property access (`secrets.NAME`,
# `secrets['NAME']`), and not itself a property (`inputs.secrets`). A wildcard
# projection (`secrets.*`) selects every secret, so it counts as the whole context.
BARE_SECRETS = re.compile(
    r"(?<![\w.\-])secrets\b(?!\s*(?:\.(?!\s*\*)|\[))", re.IGNORECASE
)


def uses_whole_secrets_context(expression: str) -> bool:
    return BARE_SECRETS.search(re.sub(STRING_LITERAL, "", expression)) is not None


def forwards_all_secrets(node: object, key: str = "") -> bool:
    """True for `secrets: inherit` and for the whole secrets context in an expression."""
    if isinstance(node, str):
        if key == "secrets" and node.strip().lower() == "inherit":
            return True
        if key == "if":  # an `if` value is an expression even without `${{ }}`
            return uses_whole_secrets_context(node)
        return any(uses_whole_secrets_context(m) for m in EXPRESSION.findall(node))
    if isinstance(node, dict):
        return any(forwards_all_secrets(v, str(k)) for k, v in node.items())
    if isinstance(node, list):
        return any(forwards_all_secrets(item) for item in node)
    return False


def environment_name(job: dict) -> str | None:
    environment = job.get("environment")
    if isinstance(environment, dict):
        environment = environment.get("name")
    return environment if isinstance(environment, str) else None


def has_reviewed_environment(job: dict) -> bool:
    return environment_name(job) in ALLOWED_ENVIRONMENTS


def violations(workflows: dict[str, dict]) -> list[str]:
    found = []
    for name, workflow in workflows.items():
        # Anything outside `jobs` (workflow-level env) reaches jobs with no environment.
        outside = {k: v for k, v in workflow.items() if k != "jobs"}
        if reads_signing_secret(outside):
            found.append(f"{name} reads a MINISIGN secret outside any job")
        if forwards_all_secrets(outside):
            found.append(f"{name} forwards the whole secrets context outside any job")
        for job_id, job in (workflow.get("jobs") or {}).items():
            if forwards_all_secrets(job):
                found.append(
                    f"{name}:{job_id} forwards the whole secrets context"
                    " (inherit or bulk use)"
                )
            if reads_signing_secret(job) and not has_reviewed_environment(job):
                found.append(
                    f"{name}:{job_id} reads a MINISIGN secret without an environment"
                )
    return found


def test_every_job_that_reads_a_signing_secret_declares_an_environment():
    assert violations(load_workflows()) == []


def test_the_scan_sees_the_signing_jobs_it_is_meant_to_guard():
    readers = {
        f"{name}:{job_id}"
        for name, workflow in load_workflows().items()
        for job_id, job in workflow["jobs"].items()
        if reads_signing_secret(job)
    }
    assert {
        "release.yml:sign",
        "revocations.yml:build-sign",
        "publish-pypi.yml:sign",
    } <= readers


# Indexes the scan cannot resolve to a name, so each may be a signing-secret read.
COMPUTED_INDEXES = {
    "computed-format": "${{ secrets[format('MINISIGN_{0}', 'SECRET_KEY')] }}",
    "computed-env": "${{ secrets[env.NAME] }}",
    "computed-spaced": "${{ secrets[ inputs.k ] }}",
    "computed-multiline": "${{\n  secrets[\n    env.NAME\n  ]\n}}",
}


def synthetic(job: dict, **workflow) -> dict[str, dict]:
    return {"x.yml": {"jobs": {"j": job}, **workflow}}


@pytest.mark.parametrize(
    "job",
    [
        pytest.param({"steps": [{"env": {"K": SECRET}}]}, id="step-env"),
        pytest.param({"env": {"K": SECRET}, "steps": []}, id="job-env"),
        pytest.param(
            {"steps": [{"uses": "a/b@1", "with": {"key": SECRET}}]}, id="step-with"
        ),
        pytest.param(
            {"steps": [{"run": 'echo "${{ secrets.MINISIGN_PASSWORD }}"'}]}, id="run"
        ),
        pytest.param(
            {"steps": [{"run": "echo ${{ secrets['MINISIGN_SECRET_KEY'] }}"}]},
            id="index",
        ),
        pytest.param(
            {"steps": [{"env": {"K": "${{ Secrets.minisign_secret_key }}"}}]},
            id="case",
        ),
        pytest.param(
            {"uses": "./.github/workflows/x.yml", "secrets": {"k": SECRET}},
            id="reusable",
        ),
        pytest.param(
            {"environment": "", "steps": [{"env": {"K": SECRET}}]},
            id="empty-environment",
        ),
        *(
            pytest.param({"steps": [{"run": f"echo {expr}"}]}, id=name)
            for name, expr in COMPUTED_INDEXES.items()
        ),
    ],
)
def test_a_signing_secret_read_without_an_environment_is_reported(job):
    assert violations(synthetic(job)) == [
        "x.yml:j reads a MINISIGN secret without an environment"
    ]


@pytest.mark.parametrize("expression", COMPUTED_INDEXES.values(), ids=COMPUTED_INDEXES)
def test_a_computed_secrets_index_inside_a_reviewed_environment_is_allowed(expression):
    job = {
        "environment": "candidate-signing",
        "steps": [{"run": f"echo {expression}"}],
    }
    assert violations(synthetic(job)) == []


@pytest.mark.parametrize("name", ("release", "revocations", "candidate-signing"))
@pytest.mark.parametrize("form", ["string", "mapping"])
def test_a_signing_secret_read_inside_a_reviewed_environment_is_allowed(name, form):
    environment = name if form == "string" else {"name": name, "url": "https://x.test"}
    job = {"environment": environment, "steps": [{"env": {"K": SECRET}}]}
    assert violations(synthetic(job)) == []


@pytest.mark.parametrize(
    "job",
    [
        pytest.param(
            {"uses": "./.github/workflows/x.yml", "secrets": "inherit"}, id="inherit"
        ),
        pytest.param(
            {
                "environment": "release",
                "steps": [{"env": {"A": "${{ toJSON(secrets) }}"}}],
            },
            id="to-json-env",
        ),
        pytest.param(
            {"steps": [{"uses": "a/b@1", "with": {"all": "${{ toJson( secrets ) }}"}}]},
            id="to-json-with",
        ),
        pytest.param(
            {"steps": [{"run": "echo '${{ format('{0}', secrets) }}'"}]},
            id="bare-in-run",
        ),
        pytest.param(
            {"if": "${{ secrets != null }}", "steps": []}, id="bare-in-if-expression"
        ),
        pytest.param(
            {"if": "toJSON(secrets) != ''", "steps": []}, id="bare-in-if-field"
        ),
        pytest.param(
            {"steps": [{"run": "echo ${{ toJSON(secrets.*) }}"}]}, id="wildcard"
        ),
        pytest.param(
            {"steps": [{"run": "echo ${{ toJSON(secrets . * ) }}"}]},
            id="wildcard-spaced",
        ),
        pytest.param(
            {"steps": [{"run": "echo ${{ format('}}{0}', toJSON(secrets)) }}"}]},
            id="after-braces-in-string-literal",
        ),
        pytest.param(
            {"steps": [{"run": "echo ${{ format('it''s }}', toJSON(secrets)) }}"}]},
            id="after-escaped-quote-and-braces",
        ),
    ],
)
def test_bulk_secret_forwarding_is_reported(job):
    assert violations(synthetic(job)) == [
        "x.yml:j forwards the whole secrets context (inherit or bulk use)"
    ]


@pytest.mark.parametrize(
    "job",
    [
        pytest.param(
            {"steps": [{"env": {"T": "${{ inputs.secrets }}"}}]},
            id="input-named-secrets",
        ),
        pytest.param(
            {"steps": [{"env": {"T": "${{ steps.f.outputs.secrets }}"}}]}, id="output"
        ),
        pytest.param({"steps": [{"run": "echo secrets are not printed"}]}, id="prose"),
        pytest.param(
            {"steps": [{"run": "echo '${{ 'secrets' }}'"}]}, id="string-literal"
        ),
    ],
)
def test_text_that_only_mentions_secrets_is_not_bulk_forwarding(job):
    assert violations(synthetic(job)) == []


@pytest.mark.parametrize(
    "environment",
    [
        pytest.param("staging", id="unknown-name"),
        pytest.param("${{ inputs.target }}", id="expression"),
        pytest.param("${{ 'release' }}", id="literal-expression"),
        pytest.param({"name": "staging"}, id="mapping-unknown-name"),
        pytest.param({"name": "${{ inputs.target }}"}, id="mapping-expression"),
        pytest.param({"url": "https://example.test"}, id="mapping-without-name"),
        pytest.param("Release", id="case-differs"),
    ],
)
def test_a_signing_secret_read_in_an_unreviewed_environment_is_reported(environment):
    job = {"environment": environment, "steps": [{"env": {"K": SECRET}}]}
    assert violations(synthetic(job)) == [
        "x.yml:j reads a MINISIGN secret without an environment"
    ]


def test_a_workflow_level_signing_secret_is_reported_even_with_job_environments():
    job = {"environment": "release", "steps": []}
    assert violations(synthetic(job, env={"K": SECRET})) == [
        "x.yml reads a MINISIGN secret outside any job"
    ]


@pytest.mark.parametrize(
    "expression",
    ["${{ secrets.GITHUB_TOKEN }}", "${{ secrets['GITHUB_TOKEN'] }}"],
    ids=["property", "literal-index"],
)
def test_other_secrets_need_no_environment(expression):
    job = {"steps": [{"env": {"T": expression}}]}
    assert violations(synthetic(job)) == []


def test_a_workflow_level_bulk_secrets_use_is_reported():
    job = {"environment": "release", "steps": []}
    workflow_env = {"A": "${{ toJSON(secrets) }}"}
    assert violations(synthetic(job, env=workflow_env)) == [
        "x.yml forwards the whole secrets context outside any job"
    ]
