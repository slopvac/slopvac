from __future__ import annotations

import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "patch_release_workflow.py"
SPEC = importlib.util.spec_from_file_location("patch_release_workflow", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
patcher = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = patcher
SPEC.loader.exec_module(patcher)

WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"


@pytest.fixture(scope="module")
def workflow() -> dict:
    # PyYAML reads the bare `on` key as boolean True.
    return yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))


def github_glob_to_regex(pattern: str) -> re.Pattern[str]:
    """Translate the subset of GitHub's tag filter syntax this workflow uses."""
    out = ""
    for char in pattern.replace("[0-9]+", "\0"):
        out += (
            "[0-9]+"
            if char == "\0"
            else (
                "[^/]*"
                if char == "*"
                else (
                    "\\."
                    if char == "."
                    else re.escape(char)
                    if char.isalnum() or char in "-_/"
                    else char
                )
            )
        )
    return re.compile(out)


@pytest.mark.parametrize(
    ("tag", "triggers"),
    [
        ("v3.0.0", True),
        ("v10.20.30", True),
        ("rc/v3.0.0-rc.1", False),
        ("releases/v3.0.0", False),
        ("v3.0.0-rc.1", False),
        ("3.0.0", False),
    ],
)
def test_only_plain_version_tags_trigger_the_release(workflow, tag, triggers):
    (pattern,) = workflow[True]["push"]["tags"]
    assert bool(github_glob_to_regex(pattern).fullmatch(tag)) is triggers


def _validation_regex() -> re.Pattern[str]:
    text = WORKFLOW.read_text(encoding="utf-8")
    match = re.search(r'\[\[ ! "\$REF_NAME" =~ (\S+) \]\]', text)
    assert match, "plan job has no tag validation"
    return re.compile(match.group(1))


@pytest.mark.parametrize(
    ("tag", "accepted"),
    [
        ("v3.0.0", True),
        ("v3.0.0-rc.1", False),
        ("v3.0.0$(id)", False),
        ("v3.0.0`id`", False),
        ("v3.0.0\nx", False),
        ("v3.0", False),
        ("v0.0.0", True),
        ("v03.0.0", False),
        ("v3.00.0", False),
        ("v3.0.00", False),
    ],
)
def test_the_plan_job_validates_the_tag(tag, accepted):
    assert bool(_validation_regex().fullmatch(tag)) is accepted


def test_validation_is_the_first_plan_step(workflow):
    first = workflow["jobs"]["plan"]["steps"][0]
    assert first["name"] == "Validate the tag"
    assert first["env"] == {"REF_NAME": "${{ github.ref_name }}"}


def test_only_the_host_job_can_write_contents(workflow):
    assert workflow["permissions"] == {"contents": "read"}
    writers = {
        name
        for name, job in workflow["jobs"].items()
        if (job.get("permissions") or {}).get("contents") == "write"
    }
    assert writers == {"host", "publish-legacy-release"}


@pytest.mark.parametrize(
    "text",
    [
        "name: Release\n",
        'permissions:\n  "contents": "write"\non:\n  push:\n    tags:\n      - \'**[0-9]+.[0-9]+.[0-9]+*\'\n',
    ],
)
def test_a_changed_template_makes_the_patcher_fail(text):
    with pytest.raises(ValueError, match="expected exactly one"):
        patcher.harden(text)


def test_hardening_refuses_a_tag_expression_it_did_not_rewrite(raw):
    marker = "rm -f artifacts/*-dist-manifest.json"
    assert marker in raw
    injected = raw.replace(marker, "echo ${{ github.ref_name }}\n          " + marker)
    with pytest.raises(ValueError, match="tag interpolated"):
        patcher.harden(injected)


def test_host_requires_the_provenance_job(workflow):
    host = workflow["jobs"]["host"]
    assert "custom-release-provenance" in host["needs"]
    assert "needs.custom-release-provenance.result == 'success'" in host["if"]


def test_the_provenance_job_gets_no_secrets_and_read_permissions(workflow):
    job = workflow["jobs"]["custom-release-provenance"]
    assert job["uses"] == "./.github/workflows/release-provenance.yml"
    # Only the app key reaches the called workflow; never `secrets: inherit`.
    assert job["secrets"] == {"RELEASE_APP_PRIVATE_KEY": "${{ secrets.RELEASE_APP_PRIVATE_KEY }}"}
    assert job["permissions"] == {"actions": "read", "contents": "read"}


def _publish_jobs() -> dict:
    return yaml.safe_load(
        (ROOT / ".github" / "workflows" / "publish-lint.yml").read_text(
            encoding="utf-8"
        )
    )["jobs"]


def test_the_stable_pypi_job_waits_for_the_provenance_job():
    publish = _publish_jobs()
    assert "provenance" in publish["pypi"]["needs"]
    assert publish["provenance"]["uses"] == "./.github/workflows/release-provenance.yml"


def test_the_provenance_gate_applies_only_to_gated_versions():
    publish = _publish_jobs()
    assert "needs.build.outputs.gated == 'true'" in publish["provenance"]["if"]
    # 2.x skips the gate job, and the stable job must still run then.
    condition = publish["pypi"]["if"]
    assert (
        "(needs.build.outputs.gated != 'true' || needs.provenance.result == 'success')"
        in condition
    )
    assert "!cancelled()" in condition


# --- hardening the raw cargo-dist output -------------------------------------

RAW = ROOT / "tests" / "fixtures" / "release.generated.yml"


@pytest.fixture(scope="module")
def raw() -> str:
    return RAW.read_text(encoding="utf-8")


@pytest.fixture(scope="module")
def hardened(raw) -> str:
    return patcher.harden(raw)


def test_hardening_the_pinned_dist_output_reproduces_the_committed_workflow(hardened):
    assert hardened == WORKFLOW.read_text(encoding="utf-8")


# Each string is a spot in dist's template that a rewrite depends on.
ANCHORS = [
    "      - '**[0-9]+.[0-9]+.[0-9]+*'\n",
    "on:\n  pull_request:\n  push:\n",
    "dist ${{ (!github.event.pull_request && format('host --steps=create --tag={0}', github.ref_name)) || 'plan' }} --output-format=json",
    '          gh release upload "${{ needs.plan.outputs.tag }}" artifacts/*\n',
    '          gh release edit "${{ needs.plan.outputs.tag }}" --target',
    '  plan:\n    runs-on: "ubuntu-22.04"\n',
    'permissions:\n  "contents": "write"\n',
    "  custom-release-provenance:\n    uses: $/.github/workflows/release-provenance.yml\n    secrets: inherit\n",
    "  host:\n    needs:\n      - plan\n",
    "      - name: Build artifacts\n        run: |\n",
    "steps.cargo-cyclonedx.output.paths",
    "      - id: host\n",
    "      - name: Install dist\n        run: ${{ matrix.install_dist.run }}\n",
    "      - name: Install cargo-auditable\n        run: ${{ matrix.install_cargo_auditable.run }}\n",
    "      - name: Install cargo-cyclonedx\n        # we specify bash to get pipefail; it guards against the `curl` command\n        # failing. otherwise `sh` won't catch that `curl` return non-0\n        run: \"curl --proto '=https' --tlsv1.2 -LsSf https://github.com/CycloneDX/cyclonedx-rust-cargo/releases/download/cargo-cyclonedx-0.5.5/cargo-cyclonedx-installer.sh | sh\"\n        shell: bash\n",
    "      - name: Install Rust non-interactively if not already installed\n        if: ${{ matrix.container }}\n        run: |\n          if ! command -v cargo > /dev/null 2>&1; then\n            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y\n            echo \"$HOME/.cargo/bin\" >> $GITHUB_PATH\n          fi\n",
    "          rm -f artifacts/*-dist-manifest.json\n      - name: Create GitHub Release\n",
    "      publishing: ${{ !github.event.pull_request }}\n",
]


@pytest.mark.parametrize("anchor", ANCHORS)
def test_a_missing_anchor_fails_the_patch(raw, anchor):
    assert anchor in raw
    with pytest.raises(ValueError, match="expected"):
        patcher.harden(raw.replace(anchor, ""))


@pytest.mark.parametrize("anchor", ANCHORS)
def test_a_duplicated_anchor_fails_the_patch(raw, anchor):
    with pytest.raises(ValueError, match="expected"):
        patcher.harden(raw.replace(anchor, anchor + anchor))


def test_a_tag_flag_in_an_unexpected_command_shape_fails_the_patch(raw):
    old = "dist build ${{ needs.plan.outputs.tag-flag }}"
    assert old in raw
    with pytest.raises(ValueError, match="tag interpolated"):
        patcher.harden(
            raw.replace(old, "dist build --verbose ${{ needs.plan.outputs.tag-flag }}")
        )


def _run_scripts(text: str):
    for name, job in yaml.safe_load(text)["jobs"].items():
        for step in job.get("steps", []):
            if "run" in step:
                yield name, step["run"]


def test_no_run_script_of_the_hardened_output_interpolates_the_tag(hardened):
    scripts = list(_run_scripts(hardened))
    assert scripts
    for job, script in scripts:
        for expression in re.findall(r"\$\{\{.*?\}\}", script):
            assert "ref_name" not in expression, (job, expression)
            assert "outputs.tag" not in expression, (job, expression)


def test_the_raw_output_does_interpolate_the_tag(raw):
    # Guards the check above: it must be able to fail.
    assert any(
        "needs.plan.outputs.tag" in script or "github.ref_name" in script
        for _, script in _run_scripts(raw)
    )


def test_the_sbom_paths_reach_the_global_upload(workflow):
    steps = workflow["jobs"]["build-global-artifacts"]["steps"]
    upload = next(step for step in steps if step.get("name") == "Upload artifacts")
    assert "${{ steps.cargo-cyclonedx.outputs.paths }}" in upload["with"]["path"]
    assert ".output." not in WORKFLOW.read_text(encoding="utf-8")


def test_the_local_build_step_runs_under_bash_so_the_tag_flag_expands(workflow):
    steps = workflow["jobs"]["build-local-artifacts"]["steps"]
    build = next(step for step in steps if step.get("name") == "Build artifacts")
    assert build["shell"] == "bash"
    assert "$TAG_FLAG" in build["run"]


def test_the_host_job_rechecks_the_tag_before_dist_and_before_the_release(workflow):
    steps = workflow["jobs"]["host"]["steps"]
    checks = [i for i, s in enumerate(steps) if "verify-tag" in s.get("run", "")]
    host = next(i for i, s in enumerate(steps) if s.get("id") == "host")
    publish = next(
        i for i, s in enumerate(steps) if s.get("name") == "Publish the release"
    )
    assert checks == [host - 1, publish - 1]
    assert "gh release edit" in steps[publish]["run"]
    assert "--draft=false" in steps[publish]["run"]
    for index in checks:
        step = steps[index]
        assert step["env"] == {
            "RELEASE_TAG": "${{ needs.plan.outputs.tag }}",
            "RELEASE_COMMIT": "${{ github.sha }}",
        }
        assert '"$RELEASE_TAG" "$RELEASE_COMMIT"' in step["run"]


# --- draft-first publication and routing by major version ----------------------


def test_artifacts_upload_to_the_draft_and_the_undraft_comes_last(workflow):
    steps = workflow["jobs"]["host"]["steps"]
    names = [s.get("name") for s in steps]
    upload = names.index("Upload the artifacts to the draft release")
    publish = names.index("Publish the release")
    assert upload < publish
    assert "gh release upload" in steps[upload]["run"]


def _transitive_needs(jobs: dict, name: str) -> set[str]:
    seen: set[str] = set()
    todo = _as_list(jobs[name].get("needs"))
    while todo:
        dep = todo.pop()
        if dep not in seen:
            seen.add(dep)
            todo.extend(_as_list(jobs[dep].get("needs")))
    return seen


def _as_list(needs) -> list[str]:
    return [needs] if isinstance(needs, str) else list(needs or [])


def test_only_the_dist_entry_jobs_and_the_legacy_job_read_the_route(workflow):
    jobs = workflow["jobs"]
    routed = {n for n, j in jobs.items() if "route" in _as_list(j.get("needs"))}
    assert routed == {"plan", "custom-release-provenance", "publish-legacy-release"}
    assert jobs["plan"]["if"] == "${{ needs.route.outputs.gated == 'true' }}"
    assert jobs["custom-release-provenance"]["if"] == jobs["plan"]["if"]
    legacy = jobs["publish-legacy-release"]
    assert legacy["if"] == "${{ needs.route.outputs.gated == 'false' }}"


def test_every_dist_job_depends_on_the_routed_plan(workflow):
    jobs = workflow["jobs"]
    others = set(jobs) - {
        "route",
        "plan",
        "custom-release-provenance",
        "publish-legacy-release",
    }
    assert others
    for name in others:
        assert "plan" in _transitive_needs(jobs, name), name


def test_the_host_job_cannot_run_when_plan_was_skipped(workflow):
    assert "needs.plan.result == 'success'" in workflow["jobs"]["host"]["if"]


def _legacy_steps(workflow) -> list[dict]:
    return workflow["jobs"]["publish-legacy-release"]["steps"]


def test_the_legacy_job_takes_its_values_from_the_environment(workflow):
    job = workflow["jobs"]["publish-legacy-release"]
    assert job["permissions"] == {"contents": "write"}
    assert job["env"]["RELEASE_TAG"] == "${{ github.ref_name }}"
    assert job["env"]["RELEASE_COMMIT"] == "${{ github.sha }}"
    for step in _legacy_steps(workflow):
        assert "${{" not in step.get("run", ""), step["run"]


def test_the_legacy_job_checks_everything_before_it_publishes(workflow):
    runs = [
        s["run"].strip()
        for s in _legacy_steps(workflow)
        if "run" in s and s["name"] != "Explain a failed token mint"
    ]
    assert runs == [
        'python3 scripts/release_ruleset.py check "$RELEASE_TAG" --repo "$REPO"',
        'python3 scripts/release_provenance.py check-draft "$RELEASE_TAG" --repo "$REPO" --app-slug "$APP_SLUG"',
        'python3 scripts/release_provenance.py verify-tag "$RELEASE_TAG" "$RELEASE_COMMIT"',
        'gh release edit "$RELEASE_TAG" --repo "$REPO" --draft=false',
    ]


def test_the_route_job_fails_closed_on_a_bad_tag(workflow):
    steps = workflow["jobs"]["route"]["steps"]
    route = next(s for s in steps if s.get("id") == "route")
    assert route["env"]["REF_NAME"] == "${{ github.ref_name }}"
    assert 'route-tag "$REF_NAME" >> "$GITHUB_OUTPUT"' in route["run"]
    assert "set +e" not in route["run"]


def test_release_please_makes_a_tagged_draft_that_dist_then_publishes():
    import json
    import tomllib

    config = json.loads(
        (ROOT / "release-please-config.json").read_text(encoding="utf-8")
    )
    assert config["draft"] is True
    assert config["force-tag-creation"] is True
    dist = tomllib.loads((ROOT / "dist-workspace.toml").read_text(encoding="utf-8"))
    assert dist["dist"]["create-release"] is False


def _mint_step(steps: list[dict]) -> dict:
    return next(s for s in steps if s.get("id") == "ruleset-token")


def test_the_legacy_ruleset_check_reads_with_the_app_token_and_no_fallback(workflow):
    steps = workflow["jobs"]["publish-legacy-release"]["steps"]
    checks = [s for s in steps if "release_ruleset.py" in s.get("run", "")]
    assert [c["env"] for c in checks] == [
        {"GH_TOKEN": "${{ steps.ruleset-token.outputs.token }}"}
    ]
    assert steps.index(_mint_step(steps)) < steps.index(checks[0])


def test_the_pypi_gate_passes_only_the_app_key():
    provenance = _publish_jobs()["provenance"]
    assert provenance["secrets"] == {"RELEASE_APP_PRIVATE_KEY": "${{ secrets.RELEASE_APP_PRIVATE_KEY }}"}


# --- the reusable provenance workflow ------------------------------------------

PROVENANCE = ROOT / ".github" / "workflows" / "release-provenance.yml"


@pytest.fixture(scope="module")
def provenance() -> dict:
    return yaml.safe_load(PROVENANCE.read_text(encoding="utf-8"))


# A minimal evaluator for the `if:` subset these workflows use: ==, !=, &&, ||, !,
# parentheses, string literals and context names. Merge it with tests/workflow_expr.py
# once that helper lands from the publish-lint test work.
_TOKEN = re.compile(r"\s*(==|!=|&&|\|\||!|\(|\)|'[^']*'|[A-Za-z_][A-Za-z0-9_.]*)")


def evaluate_condition(expression: str, context: dict[str, str]) -> bool:
    text = expression.strip()
    if text.startswith("${{") and text.endswith("}}"):
        text = text[3:-2]
    tokens = _TOKEN.findall(text)
    position = 0

    def peek() -> str | None:
        return tokens[position] if position < len(tokens) else None

    def take() -> str:
        nonlocal position
        position += 1
        return tokens[position - 1]

    def atom() -> str | bool:
        token = take()
        if token == "(":
            value = either()
            assert take() == ")"
            return value
        if token == "!":
            return not truthy(atom())
        if token.startswith("'"):
            return token[1:-1]
        return context[token]  # a missing context name is a test error

    def truthy(value: str | bool) -> bool:
        return bool(value)

    def comparison() -> str | bool:
        left = atom()
        if peek() in ("==", "!="):
            operator = take()
            right = atom()
            return (left == right) if operator == "==" else (left != right)
        return left

    def both() -> str | bool:
        value = comparison()
        while peek() == "&&":
            take()
            right = comparison()
            value = right if truthy(value) else value
        return value

    def either() -> str | bool:
        value = both()
        while peek() == "||":
            take()
            right = both()
            value = value if truthy(value) else right
        return value

    result = either()
    assert position == len(tokens), f"unparsed input in {expression!r}"
    return truthy(result)


def test_the_app_key_is_a_required_workflow_call_secret(provenance):
    # PyYAML reads the bare `on` key as boolean True.
    secrets = provenance[True]["workflow_call"]["secrets"]
    assert list(secrets) == ["RELEASE_APP_PRIVATE_KEY"]
    assert secrets["RELEASE_APP_PRIVATE_KEY"]["required"] is True


def test_the_ruleset_check_reads_with_the_minted_token_and_no_fallback(provenance):
    steps = provenance["jobs"]["provenance"]["steps"]
    check = next(s for s in steps if "release_ruleset.py" in s.get("run", ""))
    assert check["env"]["GH_TOKEN"] == "${{ steps.ruleset-token.outputs.token }}"
    assert steps.index(_mint_step(steps)) < steps.index(check)


def test_a_failed_mint_stops_the_job_with_an_instruction(provenance):
    steps = provenance["jobs"]["provenance"]["steps"]
    explain = steps[steps.index(_mint_step(steps)) + 1]
    assert "steps.ruleset-token.outcome == 'failure'" in explain["if"]
    assert "Administration: Read-only" in explain["run"]
    assert explain["run"].rstrip().endswith("exit 1")


def test_the_dispatch_input_is_a_required_tag_that_only_the_check_reads(provenance):
    dispatch = provenance[True]["workflow_dispatch"]["inputs"]["tag"]
    assert dispatch["required"] is True
    steps = {s["name"]: s for s in provenance["jobs"]["provenance"]["steps"] if "name" in s}
    check = steps["Require an immutable tag ruleset"]
    assert check["env"]["TAG"] == "${{ inputs.tag || github.ref_name }}"
    assert "inputs" not in check["run"]


# Which provenance steps run, by how the workflow starts. `inputs.tag` is empty when
# a release workflow calls it, even a caller that was itself dispatched, so those
# callers still get the candidate check. A pull request or a plain branch run
# starts no check: only a tag can publish.
GATED_STEPS = {
    "Check the candidate": "candidate",
    "Mint the ruleset read token": "mint",
    "Require an immutable tag ruleset": "ruleset",
}
RUNS = [
    pytest.param("branch", "v2.9.1", {"mint", "ruleset"}, id="dispatch-from-a-branch"),
    pytest.param("tag", "v2.9.1", {"mint", "ruleset"}, id="dispatch-on-a-tag-ref"),
    pytest.param("tag", "", {"candidate", "mint", "ruleset"}, id="called-by-a-release"),
    pytest.param("branch", "", set(), id="pull-request-or-branch-call"),
]


@pytest.mark.parametrize(("ref_type", "tag", "expected"), RUNS)
def test_each_start_runs_exactly_the_intended_provenance_steps(
    provenance, ref_type, tag, expected
):
    context = {"github.ref_type": ref_type, "inputs.tag": tag}
    steps = {s["name"]: s for s in provenance["jobs"]["provenance"]["steps"] if "name" in s}
    ran = {
        label
        for name, label in GATED_STEPS.items()
        if evaluate_condition(steps[name]["if"], context)
    }
    assert ran == expected


def test_the_provenance_workflow_has_no_write_permission(provenance):
    assert set(provenance["permissions"].values()) == {"read"}
    assert "permissions" not in provenance["jobs"]["provenance"]


def test_the_candidate_check_keeps_the_workflow_token(provenance):
    steps = provenance["jobs"]["provenance"]["steps"]
    candidate = next(s for s in steps if s.get("name") == "Check the candidate")
    assert candidate["env"]["GH_TOKEN"] == "${{ github.token }}"


def test_dist_args_reach_the_build_through_the_environment(workflow):
    steps = workflow["jobs"]["build-local-artifacts"]["steps"]
    build = next(step for step in steps if step.get("name") == "Build artifacts")
    assert build["env"] == {"DIST_ARGS": "${{ matrix.dist_args }}"}
    assert "$DIST_ARGS" in build["run"]
    assert "matrix.dist_args" not in build["run"]


@pytest.mark.parametrize(
    ("expression", "key_line_offset"),
    [
        ("matrix.packages_install", -1),  # the expression sits under `run: |`
    ],
)
def test_matrix_commands_keep_a_justified_zizmor_ignore_on_their_run_line(
    expression, key_line_offset
):
    lines = WORKFLOW.read_text(encoding="utf-8").splitlines()
    (at,) = [i for i, line in enumerate(lines) if "${{ " + expression + " }}" in line]
    key_line = lines[at + key_line_offset]
    assert key_line.strip().startswith("run:")
    assert "# zizmor: ignore[template-injection]" in key_line
    assert "dist-workspace.toml" in key_line


# --- minisign signing in a fresh job, verified before the draft is published ----


def _steps(workflow, job: str) -> list[dict]:
    return workflow["jobs"][job]["steps"]


def _step_index(workflow, job: str, name: str) -> int:
    return next(i for i, s in enumerate(_steps(workflow, job)) if s.get("name") == name)


def test_the_host_job_waits_for_the_signing_job_and_requires_its_success(workflow):
    host = workflow["jobs"]["host"]
    assert "sign" in host["needs"]
    assert "needs.sign.result == 'success'" in host["if"]


def test_the_signing_job_needs_the_build_jobs_and_runs_when_the_host_job_would(
    workflow,
):
    sign = workflow["jobs"]["sign"]
    assert set(sign["needs"]) == {
        "plan",
        "custom-release-provenance",
        "build-local-artifacts",
        "build-global-artifacts",
    }
    assert sign["if"] == workflow["jobs"]["host"]["if"].replace(
        " && needs.sign.result == 'success'", ""
    )


def test_the_signing_job_never_installs_or_runs_dist(workflow):
    for step in _steps(workflow, "sign"):
        assert "cargo-dist" not in str(step)
        assert "install_cargo_dist" not in str(step)
        assert not re.search(r"(?m)^\s*dist\b(?!-)", step.get("run", ""))


def test_the_signing_job_keeps_the_read_only_root_token(workflow):
    assert "permissions" not in workflow["jobs"]["sign"]
    assert workflow["permissions"] == {"contents": "read"}


def test_artifacts_are_signed_after_cleanup_and_uploaded_as_signatures(workflow):
    names = [s.get("name") for s in _steps(workflow, "sign")]
    order = [
        names.index(n)
        for n in (
            "Confirm the tag still points at the built commit",
            "Fetch artifacts",
            "Cleanup",
            "Install minisign",
            "Sign the release artifacts",
            "Upload the signatures",
        )
    ]
    assert order == sorted(order)
    upload = _steps(workflow, "sign")[order[-1]]
    assert upload["with"]["path"] == "artifacts/*.minisig"
    assert upload["with"]["if-no-files-found"] == "error"
    # The `artifacts-*` fetch pattern must not pick the signatures up a second time.
    assert not upload["with"]["name"].startswith("artifacts-")


def test_the_signing_step_signs_the_fetched_directory_for_the_tag_version(workflow):
    step = _steps(workflow, "sign")[
        _step_index(workflow, "sign", "Sign the release artifacts")
    ]
    assert (
        step["run"].strip()
        == 'python3 scripts/release_signing.py sign artifacts --version "${RELEASE_TAG#v}"'
    )
    # The plan job validated the tag as vX.Y.Z; the step reads it from the environment.
    assert step["env"]["RELEASE_TAG"] == "${{ needs.plan.outputs.tag }}"


def test_the_signing_secret_is_read_by_one_step_of_the_signing_job_only(workflow):
    text = WORKFLOW.read_text(encoding="utf-8")
    assert text.count("MINISIGN_SECRET_KEY") == 2  # the env name and its secret
    assert text.count("secrets.MINISIGN_SECRET_KEY") == 1
    holders = [
        (job, s["name"])
        for job, body in workflow["jobs"].items()
        for s in body.get("steps", [])
        if "MINISIGN_SECRET_KEY" in str(s)
    ]
    assert holders == [("sign", "Sign the release artifacts")]
    assert "MINISIGN_PASSWORD" not in text


def test_minisign_is_installed_by_the_pinned_script_in_both_jobs(workflow):
    for job in ("sign", "host"):
        step = _steps(workflow, job)[_step_index(workflow, job, "Install minisign")]
        assert step["run"] == "bash scripts/install_minisign.sh"


def test_the_host_job_fetches_and_verifies_the_signatures_before_the_upload(workflow):
    steps = _steps(workflow, "host")
    names = [s.get("name") for s in steps]
    order = [
        names.index(n)
        for n in (
            "Cleanup",
            "Fetch the signatures",
            "Install minisign",
            "Verify the signatures against the committed key",
            "Upload the artifacts to the draft release",
            "Publish the release",
        )
    ]
    assert order == sorted(order)
    assert steps[order[1]]["with"] == {"name": "release-signatures", "path": "artifacts"}
    verify = steps[order[3]]["run"]
    assert "scripts/release_signing.py verify" in verify
    assert "release_signing.py sign" not in verify


def test_the_host_job_drops_its_own_manifest_so_every_upload_is_signed(workflow):
    cleanup = _steps(workflow, "host")[_step_index(workflow, "host", "Cleanup")]["run"]
    assert "rm -f artifacts/dist-manifest.json" in cleanup


# --- dist comes from a pinned archive ------------------------------------------


def test_no_job_pipes_a_downloaded_dist_installer_into_a_shell():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "cargo-dist-installer" not in text
    assert "matrix.install_dist" not in text


def test_every_dist_install_step_runs_the_pinned_script(workflow):
    installs = [
        (job, s)
        for job, body in workflow["jobs"].items()
        for s in body.get("steps", [])
        if s.get("name") == "Install dist"
    ]
    assert [job for job, _ in installs] == ["plan", "build-local-artifacts"]
    assert {s["run"] for _, s in installs} == {"bash scripts/install_cargo_dist.sh"}


# --- Homebrew tap token minted from the release App -----------------------------


def _homebrew_steps(workflow) -> list[dict]:
    return workflow["jobs"]["publish-homebrew-formula"]["steps"]


def test_the_tap_token_is_minted_from_the_release_app_before_the_checkout(workflow):
    steps = _homebrew_steps(workflow)
    mint = steps[0]
    assert mint["id"] == "tap-token"
    assert (
        mint["uses"]
        == "actions/create-github-app-token@64ab729a6294000f5aa4a4bc0b73d1f32bc45510"
    )
    assert mint["with"] == {
        "client-id": "${{ vars.RELEASE_APP_CLIENT_ID }}",
        "private-key": "${{ secrets.RELEASE_APP_PRIVATE_KEY }}",
        "owner": "slopvac",
        "repositories": "homebrew-tap",
        "permission-contents": "write",
    }
    checkout = steps[1]
    assert checkout["with"]["repository"] == "slopvac/homebrew-tap"
    assert checkout["with"]["token"] == "${{ steps.tap-token.outputs.token }}"
    # No stored secret feeds the tap push, in this or any other workflow.
    for path in (ROOT / ".github" / "workflows").glob("*.yml"):
        assert "HOMEBREW_TAP_TOKEN" not in path.read_text(encoding="utf-8"), path.name


def test_the_app_action_matches_the_pin_release_please_uses():
    release_please = (ROOT / ".github" / "workflows" / "release-please.yml").read_text(
        encoding="utf-8"
    )
    pin = "actions/create-github-app-token@64ab729a6294000f5aa4a4bc0b73d1f32bc45510 # v3.3.0"
    assert pin in release_please
    assert pin in WORKFLOW.read_text(encoding="utf-8")


def test_a_second_tap_token_in_the_template_fails_the_patch(raw):
    old = "          token: ${{ secrets.HOMEBREW_TAP_TOKEN }}\n"
    assert old in raw
    with pytest.raises(ValueError, match="expected exactly one Homebrew"):
        patcher.harden(raw.replace(old, old + old))
    with pytest.raises(ValueError, match="expected exactly one Homebrew"):
        patcher.harden(raw.replace(old, ""))


def _tap_checkout(raw: str) -> str:
    match = patcher._TAP_CHECKOUT.search(raw)
    assert match
    return match.group(0)


def test_a_missing_tap_checkout_in_the_homebrew_job_fails_the_patch(raw):
    block = _tap_checkout(raw)
    with pytest.raises(ValueError, match="expected exactly one Homebrew tap checkout"):
        patcher.harden(raw.replace(block, ""))


def test_a_duplicated_tap_checkout_in_the_homebrew_job_fails_the_patch(raw):
    block = _tap_checkout(raw)
    with pytest.raises(ValueError, match="expected"):
        patcher.harden(raw.replace(block, block + block))


def test_a_tap_checkout_outside_the_homebrew_job_is_not_rewritten(raw):
    # Rename the job: the checkout then sits in no job the patcher recognises.
    renamed = raw.replace(patcher._TAP_JOB, "  publish-formula-elsewhere:\n")
    with pytest.raises(ValueError, match="expected exactly one Homebrew publish job"):
        patcher.harden(renamed)


@pytest.mark.parametrize("job", ["build-local-artifacts", "build-global-artifacts"])
def test_the_dist_build_jobs_install_the_pinned_toolchain_before_dist(workflow, job):
    steps = workflow["jobs"][job]["steps"]
    runs = [s.get("run", "") for s in steps]
    helper = runs.index("bash scripts/ci/install-rust-toolchain.sh")
    assert steps[helper - 1]["uses"].startswith("actions/setup-python@")
    dist = next(i for i, s in enumerate(steps) if "dist" in s.get("name", "").lower())
    assert helper < dist


def test_a_missing_dist_build_job_fails_the_toolchain_patch(raw):
    with pytest.raises(ValueError, match="expected"):
        patcher.harden(raw.replace("  build-global-artifacts:\n", "  other:\n"))


# --- app tokens, never personal access tokens -----------------------------------

ALL_WORKFLOWS = sorted((ROOT / ".github" / "workflows").glob("*.yml"))
SECRET_REF = re.compile(r"secrets\.([A-Za-z0-9_]+)")
APP_TOKEN = "actions/create-github-app-token@"


def test_no_workflow_reads_a_token_secret_other_than_the_workflow_token():
    found = {
        (path.name, name)
        for path in ALL_WORKFLOWS
        for name in SECRET_REF.findall(path.read_text(encoding="utf-8"))
        if name != "GITHUB_TOKEN" and re.search(r"(TOKEN|PAT)$", name)
    }
    assert found == set()


def _app_token_steps():
    for path in ALL_WORKFLOWS:
        doc = yaml.safe_load(path.read_text(encoding="utf-8"))
        for job in doc["jobs"].values():
            for step in job.get("steps", []):
                if step.get("uses", "").startswith(APP_TOKEN):
                    yield path.name, step


def test_every_ruleset_check_token_asks_only_for_administration_read():
    ruleset = [
        (name, step["with"])
        for name, step in _app_token_steps()
        if step.get("id") == "ruleset-token"
    ]
    assert {name for name, _ in ruleset} == {
        "release.yml",
        "release-provenance.yml",
    }
    for _, inputs in ruleset:
        permissions = {k: v for k, v in inputs.items() if k.startswith("permission-")}
        assert permissions == {"permission-administration": "read"}
        assert inputs["repositories"] == "${{ github.event.repository.name }}"


# --- no tool is installed from an unchecked download ---------------------------

# Written apart from the patcher's own pattern on purpose.
SHELL_PIPE = re.compile(
    r"(curl|wget|irm|iwr)\b[^\n]*\|\s*(sudo\s+)?(bash|zsh|sh)\b"
    r"|\|\s*iex\b"
    r"|sh\s+-c\s+[\"']?\$\((curl|wget)"
    r"|<\((curl|wget)",
    re.I,
)

DOWNLOAD_TO_SHELL = [
    "curl --proto '=https' -LsSf https://example.test/install.sh | sh",
    "curl -fsSL https://example.test/install.sh | bash -s -- -y",
    "wget -qO- https://example.test/install.sh | sh",
    "irm https://example.test/install.ps1 | iex",
    'sh -c "$(curl -fsSL https://example.test/install.sh)"',
    "bash <(curl -s https://example.test/install.sh)",
]


def test_no_run_script_of_the_regenerated_workflow_pipes_a_download_into_a_shell():
    text = WORKFLOW.read_text(encoding="utf-8")
    offenders = [
        (job, line)
        for job, script in _run_scripts(text)
        for line in script.splitlines()
        if SHELL_PIPE.search(line)
    ]
    assert offenders == []
    assert "matrix.install_" not in text


@pytest.mark.parametrize("command", DOWNLOAD_TO_SHELL)
def test_the_scan_recognises_each_download_to_shell_shape(command):
    assert SHELL_PIPE.search(command)


@pytest.mark.parametrize("command", DOWNLOAD_TO_SHELL)
def test_hardening_refuses_a_download_piped_into_a_shell(raw, command):
    marker = "rm -f artifacts/*-dist-manifest.json"
    injected = raw.replace(marker, command + "\n          " + marker)
    with pytest.raises(ValueError, match="piped into a shell"):
        patcher.harden(injected)


def test_hardening_refuses_a_matrix_install_command_it_did_not_replace(raw):
    marker = "rm -f artifacts/*-dist-manifest.json"
    injected = raw.replace(
        marker, "${{ matrix.install_other.run }}\n          " + marker
    )
    with pytest.raises(ValueError, match="piped into a shell"):
        patcher.harden(injected)


@pytest.mark.parametrize(
    ("job", "name", "script"),
    [
        ("build-local-artifacts", "Install cargo-auditable", "install_cargo_auditable"),
        ("build-global-artifacts", "Install cargo-cyclonedx", "install_cargo_cyclonedx"),
    ],
)
def test_the_cargo_tools_install_from_the_pinned_scripts(workflow, job, name, script):
    (step,) = [s for s in _steps(workflow, job) if s.get("name") == name]
    assert step["run"] == f"bash scripts/{script}.sh"


def test_the_rustup_bootstrap_is_gone_and_the_toolchain_step_is_kept(workflow):
    for job in ("build-local-artifacts", "build-global-artifacts"):
        runs = [s.get("run", "") for s in _steps(workflow, job)]
        assert not any("sh.rustup.rs" in r for r in runs)
        assert "bash scripts/ci/install-rust-toolchain.sh" in runs


# --- the host condition must really gain the signing requirement ----------------


def test_a_host_condition_without_space_before_the_closing_braces_fails_the_patch(raw):
    pattern = re.compile(
        r"^(    if: \$\{\{ always\(\) && needs\.plan\.result .*?) \}\}$", re.M
    )
    assert pattern.search(raw)
    drifted = pattern.sub(r"\1}}", raw)
    with pytest.raises(ValueError, match="host condition"):
        patcher.harden(drifted)


# --- the Homebrew job holds the tap token and runs no fetched code -----------------


def _job_text(job: str) -> str:
    text = WORKFLOW.read_text(encoding="utf-8")
    start = text.index(f"\n  {job}:\n")
    nxt = re.search(r"\n  [a-z][\w-]*:\n", text[start + 1 :])
    return text[start : start + 1 + nxt.start()] if nxt else text[start:]


def test_the_homebrew_job_runs_no_brew_command():
    assert not re.search(r"\bbrew\b", _job_text("publish-homebrew-formula"))


def test_the_tap_checkout_does_not_persist_the_token(workflow):
    steps = _homebrew_steps(workflow)
    (checkout,) = [
        s for s in steps if s.get("with", {}).get("repository") == "slopvac/homebrew-tap"
    ]
    assert checkout["with"]["persist-credentials"] is False


def test_only_the_push_step_of_the_homebrew_job_reads_the_tap_token(workflow):
    steps = _homebrew_steps(workflow)
    readers = [
        s.get("name", s.get("uses"))
        for s in steps
        if "steps.tap-token.outputs.token" in str(s.get("env", {})) + str(s.get("run", ""))
    ]
    assert readers == ["Push the formula files"]
    commit = next(s for s in steps if s.get("name") == "Commit formula files")
    assert "git push" not in commit["run"]


def test_the_commit_step_refuses_an_unexpected_formula_file_name(workflow):
    commit = next(
        s for s in _homebrew_steps(workflow) if s.get("name") == "Commit formula files"
    )
    assert "unexpected formula file name" in commit["run"]
    assert "set -euo pipefail" in commit["run"]


def _workflow(permissions: str, job_permissions: str, step: str) -> str:
    return (
        f"{permissions}jobs:\n  build:\n    runs-on: ubuntu-latest\n"
        f"{job_permissions}    steps:\n      - run: {step}\n"
    )


READ = 'permissions:\n  "contents": "read"\n\n'


@pytest.mark.parametrize(
    "job_permissions",
    [
        "    permissions: { contents: write }\n",
        "    permissions: ${{ fromJSON(needs.plan.outputs.perms) }}\n",
        "    permissions: &perms\n      contents: write\n",
        "    permissions:\n      contents: ${{ inputs.level }}\n",
        "    permissions:\n      contents: admin\n",
        "    permissions:\n      - contents\n",
    ],
)
def test_the_reader_fails_closed_on_permissions_syntax_it_does_not_understand(
    job_permissions,
):
    text = _workflow(READ, job_permissions, "echo hi")
    with pytest.raises(ValueError, match="unsupported permissions syntax"):
        patcher._assert_permissions_understood(text)


def test_the_reader_fails_closed_on_workflow_permissions_it_does_not_understand():
    text = _workflow("permissions: { contents: write }\n\n", "", "echo hi")
    with pytest.raises(ValueError, match="unsupported permissions syntax"):
        patcher._assert_permissions_understood(text)


def test_a_changed_brew_script_in_the_template_stops_the_homebrew_rewrite(raw):
    unpinned = raw.replace(
        "            brew update\n",
        "            brew update\n            brew upgrade\n",
    )
    assert unpinned != raw
    # The commit-script anchor no longer matches, so the rewrite stops loudly.
    with pytest.raises(ValueError, match="expected"):
        patcher.harden(unpinned)


# --- the tap token reaches git through the environment, not a command line ---------


def _push_step(workflow) -> dict:
    return next(
        s for s in _homebrew_steps(workflow) if s.get("name") == "Push the formula files"
    )


def test_the_tap_token_never_appears_in_a_run_command_line(workflow):
    token = "steps.tap-token.outputs.token"
    for job, script in _run_scripts(WORKFLOW.read_text(encoding="utf-8")):
        assert token not in script, job
    push = _push_step(workflow)
    assert push["env"] == {"TAP_TOKEN": "${{ " + token + " }}"}
    # The variable may be read only by shell builtins and pipes into base64, never as an
    # argument of git or another external command.
    for line in push["run"].splitlines():
        if "TAP_TOKEN" in line:
            assert line.lstrip().startswith("GIT_CONFIG_VALUE_0=")
            assert "printf 'x-access-token:%s' \"$TAP_TOKEN\" | base64" in line


def test_git_receives_the_auth_header_from_its_environment_only(workflow):
    run = _push_step(workflow)["run"]
    assert "export GIT_CONFIG_COUNT=1" in run
    assert "export GIT_CONFIG_KEY_0=http.https://github.com/.extraheader" in run
    assert "export GIT_CONFIG_VALUE_0" in run
    git_lines = [ln.strip() for ln in run.splitlines() if re.match(r"\s*git\b", ln)]
    assert git_lines == ["git push origin HEAD"]
    assert "set -x" not in run
    assert "xtrace" not in run
    assert "-o xtrace" not in run


# --- the commit script runs against real git --------------------------------------

JQ = shutil.which("jq")


def _commit_script(workflow) -> str:
    return next(
        s["run"]
        for s in _homebrew_steps(workflow)
        if s.get("name") == "Commit formula files"
    )


@pytest.fixture
def tap(tmp_path, workflow):
    """A git checkout of a tap with a runner for the commit script."""
    if JQ is None:
        pytest.skip("jq is not installed")
    repo = tmp_path / "tap"
    (repo / "Formula").mkdir(parents=True)
    git = ["git", "-C", str(repo)]
    subprocess.run([*git, "init", "-q"], check=True)
    subprocess.run([*git, "config", "commit.gpgsign", "false"], check=True)
    script = _commit_script(workflow)

    class Tap:
        path = repo

        @staticmethod
        def formula(name: str, body: str = "class X < Formula\nend\n") -> None:
            target = repo / "Formula" / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(body)

        @staticmethod
        def run(artifact: str) -> subprocess.CompletedProcess[str]:
            plan = json.dumps(
                {"releases": [{"artifacts": [artifact], "app_version": "1.2.3"}]}
            )
            env = {
                **os.environ,
                "HOME": str(tmp_path),
                "PLAN": plan,
                "GITHUB_USER": "bot",
                "GITHUB_EMAIL": "bot@example.test",
            }
            return subprocess.run(
                ["bash", "-c", script],
                cwd=repo,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )

        @staticmethod
        def subjects() -> list[str]:
            done = subprocess.run(
                [*git, "log", "--format=%s"], capture_output=True, text=True
            )
            return done.stdout.split("\n")[:-1] if done.returncode == 0 else []

    return Tap


def test_a_valid_formula_is_committed_with_its_name_and_version(tap):
    tap.formula("slopvac.rb")
    result = tap.run("slopvac.rb")
    assert result.returncode == 0, result.stderr
    assert tap.subjects() == ["slopvac 1.2.3"]


@pytest.mark.parametrize(
    "artifact",
    ["../evil.rb", "-x.rb", "--force.rb", "sub/dir.rb", ".hidden.rb"],
)
def test_an_unsafe_formula_file_name_is_refused_before_any_commit(tap, artifact):
    tap.formula("slopvac.rb")
    result = tap.run(artifact)
    assert result.returncode != 0
    assert "unexpected formula file name" in result.stderr
    assert tap.subjects() == []


def test_a_formula_file_name_with_a_space_fails_without_a_commit(tap):
    tap.formula("slopvac.rb")
    result = tap.run("has space.rb")
    assert result.returncode != 0
    assert tap.subjects() == []


def test_an_artifact_that_is_not_a_formula_commits_nothing(tap):
    tap.formula("slopvac.txt")
    result = tap.run("slopvac.txt")
    assert result.returncode == 0
    assert tap.subjects() == []


def test_a_missing_formula_file_is_refused(tap):
    result = tap.run("absent.rb")
    assert result.returncode != 0
    assert "missing or empty formula" in result.stderr
    assert tap.subjects() == []


def test_an_empty_formula_file_is_refused(tap):
    tap.formula("empty.rb", body="")
    result = tap.run("empty.rb")
    assert result.returncode != 0
    assert "missing or empty formula" in result.stderr
    assert tap.subjects() == []


# --- permissions forms that must not hide a write grant ---------------------------

_LOCAL_PERMISSIONS = (
    '    permissions:\n      "attestations": "write"\n      "contents": "read"\n'
    '      "id-token": "write"\n    steps:\n'
)
_LOCAL_INSTALL = (
    "      - name: Install cargo-auditable\n"
    "        run: ${{ matrix.install_cargo_auditable.run }}\n"
)
UNPINNED_STEP = "      - name: Injected\n        run: pip install requests\n"


def _local_job_with(raw: str, permissions: str) -> str:
    assert raw.count(_LOCAL_PERMISSIONS) == 1
    assert raw.count(_LOCAL_INSTALL) == 1
    return raw.replace(_LOCAL_PERMISSIONS, permissions).replace(
        _LOCAL_INSTALL, UNPINNED_STEP + _LOCAL_INSTALL
    )


@pytest.mark.parametrize("quote", ['"', "'"], ids=["double", "single"])
def test_hardening_refuses_a_quoted_job_permissions_key(raw, quote):
    quoted = _LOCAL_PERMISSIONS.replace(
        "    permissions:", f"    {quote}permissions{quote}:"
    )
    with pytest.raises(ValueError, match="unsupported permissions syntax"):
        patcher.harden(_local_job_with(raw, quoted))


@pytest.mark.parametrize("separator", ["# a comment at column 0\n", "\n", "      # indented\n"])
def test_hardening_refuses_a_comment_or_blank_line_inside_the_permissions_block(
    raw, separator
):
    block = (
        '    permissions:\n      "contents": "read"\n'
        + separator
        + '      "id-token": "write"\n    steps:\n'
    )
    with pytest.raises(ValueError, match="unsupported permissions syntax"):
        patcher.harden(_local_job_with(raw, block))


@pytest.mark.parametrize("quote", ['"', "'"], ids=["double", "single"])
def test_hardening_refuses_a_quoted_workflow_level_permissions_key(raw, quote):
    assert raw.count("\njobs:\n") == 1
    stray = f"{quote}permissions{quote}:\n  contents: write\n\n"
    with pytest.raises(ValueError, match="unsupported permissions syntax"):
        patcher.harden(raw.replace("\njobs:\n", "\n" + stray + "jobs:\n"))


# --- the finished workflow is pinned, job by job ---------------------------------

PIN_FILE = ROOT / "scripts" / "release_workflow_pins.json"
HOST_ANCHOR = "      - id: host\n"
SECRET_ANCHOR = "      # So we have access to the formula\n"
# `announce` reads no secret and holds no write permission.
PLAIN_ANCHOR = "  announce:\n    needs:\n"


@pytest.fixture(scope="module")
def committed() -> str:
    return WORKFLOW.read_text(encoding="utf-8")


def _insert_step(text: str, anchor: str, step: str) -> str:
    assert text.count(anchor) == 1
    return text.replace(anchor, step + anchor)


def _run_step(command: str) -> str:
    body = command.replace("\n", "\n        ")
    return f"      - name: Injected\n        run: {body}\n"


# Every way found so far to run fetched code from a job; the pins catch each one.
INJECTED_STEPS = [
    pytest.param(_run_step("true && cargo install ripgrep"), id="and-chain"),
    pytest.param(_run_step("true; cargo install ripgrep"), id="semicolon-chain"),
    pytest.param(_run_step("true | cargo install ripgrep"), id="pipe-chain"),
    pytest.param(_run_step("echo $(cargo install ripgrep)"), id="substitution"),
    pytest.param(_run_step("echo `cargo install ripgrep`"), id="backticks"),
    pytest.param(_run_step("sh -c 'pip install foo'"), id="sh-c"),
    pytest.param(_run_step('bash -c "pip install foo"'), id="bash-c"),
    pytest.param(_run_step("eval pip install foo"), id="eval"),
    pytest.param(_run_step("cargo +stable install ripgrep"), id="toolchain-selector"),
    pytest.param(_run_step("car\\\ngo install ripgrep"), id="continuation-in-a-word"),
    pytest.param(
        _run_step("cargo install \\\n    ripgrep"), id="continuation-with-indented-arg"
    ),
    pytest.param(
        '      - name: Injected\n        "run": cargo install ripgrep\n', id="quoted-run-key"
    ),
    pytest.param(_run_step("cat install.sh |\n  sh"), id="multi-line-pipe-into-sh"),
    pytest.param(_run_step("cat install.sh | /bin/sh"), id="pipe-into-bin-sh"),
    pytest.param(_run_step("npx cowsay hi"), id="npx"),
    pytest.param(_run_step("pnpm dlx cowsay hi"), id="pnpm"),
    pytest.param(_run_step("uvx ruff check"), id="uvx"),
    pytest.param(_run_step("python -m pip install requests"), id="python-m-pip"),
]


def test_the_committed_workflow_passes_with_the_committed_pins(committed):
    patcher.check_pins(committed, PIN_FILE)


def test_the_pin_table_is_sorted_and_stable(committed):
    text = PIN_FILE.read_text(encoding="utf-8")
    table = json.loads(text)
    assert text == json.dumps(table, indent=2, sort_keys=True) + "\n"
    assert table == patcher.pin_table(committed)
    assert "same review as a workflow change" in table["_comment"]


@pytest.fixture
def generate(tmp_path, monkeypatch, capsys):
    """Run the real entry point on a raw template; return (exit code, stderr)."""
    target = tmp_path / "release.yml"
    monkeypatch.setattr(patcher, "WORKFLOW", target)
    monkeypatch.delenv(patcher.UPDATE_PINS_ENV, raising=False)

    def run(raw_text: str) -> tuple[int, str]:
        target.write_text(raw_text, encoding="utf-8")
        code = patcher.main([])
        return code, capsys.readouterr().err

    run.target = target
    return run


def test_generating_the_pinned_raw_template_succeeds_and_writes_the_committed_file(
    raw, generate
):
    code, err = generate(raw)
    assert (code, err) == (0, "")
    assert generate.target.read_text(encoding="utf-8") == WORKFLOW.read_text(
        encoding="utf-8"
    )


@pytest.mark.parametrize("anchor", [SECRET_ANCHOR, HOST_ANCHOR], ids=["secret-job", "host"])
@pytest.mark.parametrize("step", INJECTED_STEPS)
def test_a_step_injected_into_a_credentialed_job_fails_generation(
    raw, generate, anchor, step
):
    code, err = generate(_insert_step(raw, anchor, step))
    assert code == 1
    assert "changed" in err


def test_a_change_in_a_job_without_credentials_fails_the_pins(committed):
    changed = committed.replace(
        PLAIN_ANCHOR, PLAIN_ANCHOR.replace("needs:\n", "needs:\n      - plan\n", 1)
    )
    assert changed != committed
    with pytest.raises(ValueError, match="job announce changed"):
        patcher.check_pins(changed, PIN_FILE)


def test_a_single_added_byte_inside_a_job_fails_the_pins(committed):
    changed = committed.replace('runs-on: "ubuntu-22.04"', 'runs-on: "ubuntu-22.04" ', 1)
    assert changed != committed
    with pytest.raises(ValueError, match="changed"):
        patcher.check_pins(changed, PIN_FILE)


@pytest.mark.parametrize("quote", ['"', "'"], ids=["double", "single"])
def test_a_quoted_permissions_key_in_a_job_fails_the_pins(committed, quote):
    local = '    permissions:\n      "attestations": "write"\n'
    assert committed.count(local) == 1
    changed = committed.replace(
        local, local.replace("    permissions:", f"    {quote}permissions{quote}:")
    )
    with pytest.raises(ValueError, match="job build-local-artifacts changed"):
        patcher.check_pins(changed, PIN_FILE)


def test_a_new_job_fails_the_pins(committed):
    changed = committed + "\n  extra:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n"
    with pytest.raises(ValueError, match="job extra is new"):
        patcher.check_pins(changed, PIN_FILE)


def test_a_removed_job_fails_the_pins(committed):
    chunks = patcher._split_jobs(committed)
    changed = "".join(c for c in chunks if not c.startswith("  announce:"))
    assert changed != committed
    with pytest.raises(ValueError, match="job announce was removed or renamed"):
        patcher.check_pins(changed, PIN_FILE)


def test_a_renamed_job_fails_the_pins(committed):
    changed = committed.replace("\n  announce:\n", "\n  announcement:\n", 1)
    assert changed != committed
    with pytest.raises(ValueError) as caught:
        patcher.check_pins(changed, PIN_FILE)
    assert "job announcement is new" in str(caught.value)
    assert "job announce was removed or renamed" in str(caught.value)


@pytest.mark.parametrize(
    ("old", "new", "block"),
    [
        ("\npermissions:\n", "\npermissions: # edited\n", "permissions"),
        ("\non:\n", "\non: # edited\n", "on"),
        ("\nname: Release\n", "\nname: Release2\n", "name"),
    ],
    ids=["permissions", "on", "name"],
)
def test_a_change_to_a_workflow_level_block_fails_the_pins(committed, old, new, block):
    assert committed.count(old) == 1
    with pytest.raises(ValueError, match=f"workflow block {block} changed"):
        patcher.check_pins(committed.replace(old, new), PIN_FILE)


def test_a_new_workflow_level_env_block_fails_the_pins(committed):
    changed = committed.replace("\njobs:\n", "\nenv:\n  A: b\n\njobs:\n", 1)
    with pytest.raises(ValueError, match="workflow block env is new"):
        patcher.check_pins(changed, PIN_FILE)


def test_the_error_names_the_job_the_workflow_file_and_the_pin_file(committed):
    changed = _insert_step(committed, HOST_ANCHOR, _run_step("npx cowsay hi"))
    with pytest.raises(ValueError) as caught:
        patcher.check_pins(changed, PIN_FILE)
    message = str(caught.value)
    assert "job host changed" in message
    assert ".github/workflows/release.yml" in message
    assert "scripts/release_workflow_pins.json" in message
    assert "--update-pins" in message


def test_writing_the_pins_makes_a_changed_workflow_pass(committed, tmp_path):
    changed = _insert_step(committed, HOST_ANCHOR, _run_step("echo reviewed"))
    pins = tmp_path / "pins.json"
    pins.write_text(PIN_FILE.read_text(encoding="utf-8"), encoding="utf-8")
    with pytest.raises(ValueError):
        patcher.check_pins(changed, pins)
    patcher.write_pins(changed, pins)
    patcher.check_pins(changed, pins)
    assert json.loads(pins.read_text(encoding="utf-8"))["jobs"]["host"] != json.loads(
        PIN_FILE.read_text(encoding="utf-8")
    )["jobs"]["host"]


def test_a_changed_header_comment_fails_the_pins(committed):
    first = committed.splitlines(keepends=True)[0]
    assert first.startswith("#")
    changed = committed.replace(first, first.rstrip("\n") + " edited\n", 1)
    with pytest.raises(ValueError, match="workflow block _preamble changed"):
        patcher.check_pins(changed, PIN_FILE)


def test_a_line_between_jobs_and_the_first_job_fails_the_pins(committed):
    changed = committed.replace("\njobs:\n", "\njobs:\n  # smuggled\n", 1)
    assert changed != committed
    with pytest.raises(ValueError, match="workflow block _jobs_header changed"):
        patcher.check_pins(changed, PIN_FILE)


def test_the_regions_add_up_to_every_byte_of_the_workflow(committed):
    assert "".join(text for _, _, text in patcher._regions(committed)) == committed


def test_generation_fails_when_the_regions_do_not_add_up(committed, monkeypatch):
    real = patcher._regions
    monkeypatch.setattr(patcher, "_regions", lambda text: real(text)[1:])
    with pytest.raises(ValueError, match="do not add up"):
        patcher.pin_table(committed)


@pytest.mark.parametrize("earlier", ["jobs: {}\n", '"jobs": {}\n', "'jobs':\n"], ids=["inline", "quoted", "single-quoted"])
def test_a_second_jobs_declaration_fails_generation(raw, generate, earlier):
    assert raw.count("\njobs:\n") == 1
    code, err = generate(raw.replace("\njobs:\n", "\n" + earlier + "jobs:\n", 1))
    assert code == 1
    assert "repeats the region _jobs_header" in err


# --- one writer for the pin file -----------------------------------------------

WRAPPER = ROOT / "scripts" / "dist_generate.sh"


def _wrapper(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", str(WRAPPER), *args], cwd=ROOT, capture_output=True, text=True, check=False
    )


def test_save_raw_refuses_the_pin_file_and_leaves_it_untouched():
    before = PIN_FILE.read_bytes()
    result = _wrapper("--save-raw", "scripts/release_workflow_pins.json")
    assert result.returncode != 0
    assert "must not point at scripts/release_workflow_pins.json" in result.stderr
    assert PIN_FILE.read_bytes() == before


def test_save_raw_refuses_the_workflow_file_and_leaves_it_untouched():
    before = WORKFLOW.read_bytes()
    result = _wrapper("--save-raw", "./.github/../.github/workflows/release.yml")
    assert result.returncode != 0
    assert "must not point at .github/workflows/release.yml" in result.stderr
    assert WORKFLOW.read_bytes() == before


def test_save_raw_refuses_a_symlink_to_the_pin_file(tmp_path):
    link = tmp_path / "raw.yml"
    link.symlink_to(PIN_FILE)
    before = PIN_FILE.read_bytes()
    result = _wrapper("--save-raw", str(link))
    assert result.returncode != 0
    assert PIN_FILE.read_bytes() == before


@pytest.mark.parametrize(
    ("args", "ambient", "seen"),
    [
        ((), "1", "unset"),
        ((), None, "unset"),
        (("--update-pins",), None, "1"),
        (("--update-pins",), "1", "1"),
    ],
    ids=["ambient-ignored", "plain", "update", "update-with-ambient"],
)
def test_the_wrapper_sets_the_update_variable_only_for_update_pins(
    tmp_path, args, ambient, seen
):
    """Run the wrapper in a scratch repo, with `dist` and `python3` replaced by shims."""
    repo = tmp_path / "repo"
    (repo / "scripts").mkdir(parents=True)
    (repo / ".github" / "workflows").mkdir(parents=True)
    shutil.copy(WRAPPER, repo / "scripts" / "dist_generate.sh")
    (repo / "dist-workspace.toml").write_text("allow-dirty = []\n")
    (repo / ".github" / "workflows" / "release.yml").write_text("x\n")
    subprocess.run(["git", "-C", str(repo), "init", "-q"], check=True)
    shims = tmp_path / "shims"
    shims.mkdir()
    record = tmp_path / "seen"
    (shims / "dist").write_text("#!/usr/bin/env bash\ntouch .github/workflows/release.yml\n")
    (shims / "python3").write_text(
        '#!/usr/bin/env bash\nprintf %s "${SLOPVAC_UPDATE_PINS-unset}" > "$RECORD"\n'
    )
    for shim in shims.iterdir():
        shim.chmod(0o755)
    env = {**os.environ, "PATH": f"{shims}{os.pathsep}{os.environ['PATH']}", "RECORD": str(record)}
    env.pop("SLOPVAC_UPDATE_PINS", None)
    if ambient:
        env["SLOPVAC_UPDATE_PINS"] = ambient
    result = subprocess.run(
        ["bash", str(repo / "scripts" / "dist_generate.sh"), *args],
        cwd=repo,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    assert record.read_text() == seen


def test_the_patcher_script_takes_no_arguments_and_has_no_pin_writing_flag():
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--write-pins"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 2
    assert "takes no arguments" in result.stderr


@pytest.mark.parametrize(
    ("env", "wrote", "checked"), [("1", True, False), (None, False, True)], ids=["update", "check"]
)
def test_only_the_update_environment_makes_generation_write_the_pins(
    raw, generate, monkeypatch, env, wrote, checked
):
    calls = []
    monkeypatch.setattr(patcher, "write_pins", lambda text: calls.append("write"))
    monkeypatch.setattr(patcher, "check_pins", lambda text: calls.append("check"))
    if env:
        monkeypatch.setenv(patcher.UPDATE_PINS_ENV, env)
    code, _ = generate(raw)
    assert code == 0
    assert calls == [("write" if wrote else "check")]
    assert (("write" in calls) is wrote) and (("check" in calls) is checked)


# --- the pin table is validated on load --------------------------------------------

GOOD = "a" * 64


@pytest.mark.parametrize(
    "text",
    [
        "[]",
        "not json",
        '{"jobs": {}, "workflow": {}, "extra": 1}',
        '{"jobs": {"a": "%s", "a": "%s"}, "workflow": {}}' % (GOOD, GOOD),
        '{"jobs": {"a": "short"}, "workflow": {}}',
        '{"jobs": {"a": "%s"}, "workflow": {"b": 1}}' % GOOD,
        '{"jobs": {"a": "%s"}}' % GOOD,
        '{"_comment": 1, "jobs": {}, "workflow": {}}',
        '{"jobs": {"a": "%s"}, "workflow": {}, "jobs": {}}' % GOOD,
    ],
    ids=[
        "not-an-object",
        "not-json",
        "unknown-field",
        "duplicate-job",
        "short-hash",
        "non-string-hash",
        "missing-section",
        "comment-not-text",
        "duplicate-section",
    ],
)
def test_an_invalid_pin_table_is_refused(tmp_path, text):
    pins = tmp_path / "pins.json"
    pins.write_text(text, encoding="utf-8")
    with pytest.raises(ValueError, match="invalid pin table"):
        patcher.load_pins(pins)


def test_the_regenerated_workflow_still_has_its_printf_base64_step(raw, workflow):
    assert patcher.harden(raw) == WORKFLOW.read_text(encoding="utf-8")
    run = _push_step(workflow)["run"]
    assert "$(printf 'x-access-token:%s' \"$TAP_TOKEN\" | base64" in run


# --- the signing key is read only in the protected `release` environment -----------


def test_only_the_signing_job_references_the_signing_key_and_it_uses_the_release_environment(
    workflow,
):
    text = WORKFLOW.read_text(encoding="utf-8")
    holders = [
        name
        for name, body in workflow["jobs"].items()
        if "MINISIGN_SECRET_KEY" in str(body)
    ]
    assert holders == ["sign"]
    assert workflow["jobs"]["sign"]["environment"] == "release"
    assert text.count("secrets.MINISIGN_SECRET_KEY") == 1
