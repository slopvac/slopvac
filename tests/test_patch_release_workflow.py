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
    ],
)
def test_the_plan_job_validates_the_tag(tag, accepted):
    assert bool(_validation_regex().fullmatch(tag)) is accepted


def test_validation_is_the_first_plan_step(workflow):
    first = workflow["jobs"]["plan"]["steps"][0]
    assert first["name"] == "Validate the tag"
    assert first["env"] == {"REF_NAME": "${{ github.ref_name }}"}


def test_no_run_script_interpolates_the_tag():
    patcher._assert_clean(WORKFLOW.read_text(encoding="utf-8"))


def test_only_the_host_job_can_write_contents(workflow):
    assert workflow["permissions"] == {"contents": "read"}
    writers = {
        name
        for name, job in workflow["jobs"].items()
        if (job.get("permissions") or {}).get("contents") == "write"
    }
    assert writers == {"host"}


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


def test_the_checker_rejects_tag_interpolation_in_a_script():
    bad = "jobs:\n  a:\n    steps:\n      - run: |\n          echo ${{ github.ref_name }}\n"
    with pytest.raises(ValueError, match="tag interpolated"):
        patcher._assert_clean(bad)


def test_host_requires_the_provenance_job(workflow):
    host = workflow["jobs"]["host"]
    assert "custom-release-provenance" in host["needs"]
    assert "needs.custom-release-provenance.result == 'success'" in host["if"]


def test_the_provenance_job_gets_no_secrets_and_read_permissions(workflow):
    job = workflow["jobs"]["custom-release-provenance"]
    assert job["uses"] == "./.github/workflows/release-provenance.yml"
    assert "secrets" not in job
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
    "dist ${{ (!github.event.pull_request && format('host --steps=create --tag={0}', github.ref_name)) || 'plan' }} --output-format=json",
    'gh release create "${{ needs.plan.outputs.tag }}"',
    '          PRERELEASE_FLAG: "${{ fromJson(steps.host.outputs.manifest)',
    'permissions:\n  "contents": "write"\n',
    "  custom-release-provenance:\n    uses: $/.github/workflows/release-provenance.yml\n    secrets: inherit\n",
    "  host:\n    needs:\n      - plan\n",
    "      - name: Build artifacts\n        run: |\n",
    "steps.cargo-cyclonedx.output.paths",
    "      - id: host\n",
    "      - name: Create GitHub Release\n",
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
    create = next(
        i for i, s in enumerate(steps) if s.get("name") == "Create GitHub Release"
    )
    assert checks == [host - 1, create - 1]
    for index in checks:
        step = steps[index]
        assert step["env"] == {
            "RELEASE_TAG": "${{ needs.plan.outputs.tag }}",
            "RELEASE_COMMIT": "${{ github.sha }}",
        }
        assert '"$RELEASE_TAG" "$RELEASE_COMMIT"' in step["run"]
