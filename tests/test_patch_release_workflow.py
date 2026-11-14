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
