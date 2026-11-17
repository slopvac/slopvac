from __future__ import annotations

import importlib.util
import re
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
    injected = raw.replace(marker, marker + "\n          echo ${{ github.ref_name }}")
    with pytest.raises(ValueError, match="tag interpolated"):
        patcher.harden(injected)


def test_host_requires_the_provenance_job(workflow):
    host = workflow["jobs"]["host"]
    assert "custom-release-provenance" in host["needs"]
    assert "needs.custom-release-provenance.result == 'success'" in host["if"]


def test_the_provenance_job_gets_no_secrets_and_read_permissions(workflow):
    job = workflow["jobs"]["custom-release-provenance"]
    assert job["uses"] == "./.github/workflows/release-provenance.yml"
    # Only the one optional token reaches the called workflow; never `secrets: inherit`.
    assert job["secrets"] == {"RULESET_READ_TOKEN": "${{ secrets.RULESET_READ_TOKEN }}"}
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
    runs = [s["run"].strip() for s in _legacy_steps(workflow) if "run" in s]
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


def test_the_ruleset_token_reaches_only_the_check_steps(workflow):
    expected = "${{ secrets.RULESET_READ_TOKEN || secrets.GITHUB_TOKEN }}"
    steps = workflow["jobs"]["publish-legacy-release"]["steps"]
    with_token = [s for s in steps if "RULESET_READ_TOKEN" in str(s)]
    assert [s["run"].split()[1] for s in with_token] == ["scripts/release_ruleset.py"]
    assert with_token[0]["env"] == {"GH_TOKEN": expected}
    for step in steps:
        assert "RULESET_READ_TOKEN" not in step.get("run", "")


def test_the_pypi_gate_passes_only_the_ruleset_token():
    provenance = _publish_jobs()["provenance"]
    assert provenance["secrets"] == {
        "RULESET_READ_TOKEN": "${{ secrets.RULESET_READ_TOKEN }}"
    }


# --- the reusable provenance workflow ------------------------------------------

PROVENANCE = ROOT / ".github" / "workflows" / "release-provenance.yml"


@pytest.fixture(scope="module")
def provenance() -> dict:
    return yaml.safe_load(PROVENANCE.read_text(encoding="utf-8"))


def test_the_ruleset_token_is_an_optional_workflow_call_secret(provenance):
    # PyYAML reads the bare `on` key as boolean True.
    secret = provenance[True]["workflow_call"]["secrets"]["RULESET_READ_TOKEN"]
    assert secret["required"] is False


def test_only_the_ruleset_check_step_reads_the_ruleset_token(provenance):
    steps = provenance["jobs"]["provenance"]["steps"]
    holders = [s for s in steps if "RULESET_READ_TOKEN" in str(s)]
    assert [s["name"] for s in holders] == ["Require an immutable tag ruleset"]
    assert holders[0]["env"]["GH_TOKEN"] == (
        "${{ secrets.RULESET_READ_TOKEN || github.token }}"
    )
    assert "scripts/release_ruleset.py" in holders[0]["run"]


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
        ("matrix.install_dist.run", 0),
        ("matrix.install_cargo_auditable.run", 0),
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


# --- minisign signing before the draft is published ----------------------------


def _host_step_index(workflow, name: str) -> int:
    steps = workflow["jobs"]["host"]["steps"]
    return next(i for i, s in enumerate(steps) if s.get("name") == name)


def test_artifacts_are_signed_after_cleanup_and_before_the_upload(workflow):
    names = [s.get("name") for s in workflow["jobs"]["host"]["steps"]]
    order = [
        names.index(n)
        for n in (
            "Cleanup",
            "Install minisign",
            "Sign the release artifacts",
            "Upload the artifacts to the draft release",
            "Publish the release",
        )
    ]
    assert order == sorted(order)


def test_the_signed_directory_is_the_one_uploaded_to_the_release(workflow):
    steps = workflow["jobs"]["host"]["steps"]
    sign = steps[_host_step_index(workflow, "Sign the release artifacts")]
    upload = steps[
        _host_step_index(workflow, "Upload the artifacts to the draft release")
    ]
    assert (
        sign["run"].strip()
        == 'python3 scripts/release_signing.py sign artifacts --version "${RELEASE_TAG#v}"'
    )
    # The plan job validated the tag as vX.Y.Z; the step reads it from the environment.
    assert sign["env"]["RELEASE_TAG"] == "${{ needs.plan.outputs.tag }}"
    # The glob also takes the .minisig files written next to the artifacts.
    assert 'gh release upload "$RELEASE_TAG" artifacts/*' in upload["run"]


def test_the_signing_secret_is_in_the_signing_step_env_only(workflow):
    text = WORKFLOW.read_text(encoding="utf-8")
    assert text.count("MINISIGN_SECRET_KEY") == 2  # the env name and its secret
    assert text.count("secrets.MINISIGN_SECRET_KEY") == 1
    steps = workflow["jobs"]["host"]["steps"]
    holders = [s["name"] for s in steps if "MINISIGN_SECRET_KEY" in str(s)]
    assert holders == ["Sign the release artifacts"]
    assert "MINISIGN_PASSWORD" not in text


def test_minisign_is_installed_by_the_pinned_script(workflow):
    step = workflow["jobs"]["host"]["steps"][
        _host_step_index(workflow, "Install minisign")
    ]
    assert step["run"] == "bash scripts/install_minisign.sh"


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
