"""The candidate workflow and the stable gate must agree on what a candidate is."""

from __future__ import annotations

import fnmatch
import importlib.util
import json
import os
import re
import subprocess
import sys
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "candidate.yml"
SCRIPT = ROOT / "scripts" / "release_provenance.py"
SPEC = importlib.util.spec_from_file_location("release_provenance", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
provenance = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = provenance
SPEC.loader.exec_module(provenance)

# BaseLoader keeps every scalar a string, so `on` stays a key and not a boolean.
DOC = yaml.load(WORKFLOW.read_text(), Loader=yaml.BaseLoader)
TRIGGERS = DOC["on"]
JOBS = DOC["jobs"]
CANDIDATE_TAG = "rc/v3.0.0-rc.2"
SHA = "a" * 40


def step(job: str, name: str) -> dict:
    return next(s for s in JOBS[job]["steps"] if s.get("name") == name)


def test_pushing_a_candidate_tag_is_the_only_automatic_trigger():
    assert set(TRIGGERS) == {"push", "workflow_dispatch"}
    assert TRIGGERS["push"] == {"tags": ["rc/v*"]}


@pytest.mark.parametrize("tag", ["rc/v3.0.0-rc.1", "rc/v10.20.30-rc.12"])
def test_the_tag_filter_catches_every_candidate_tag(tag):
    provenance.parse_candidate_tag(tag)
    assert fnmatch.fnmatchcase(tag, TRIGGERS["push"]["tags"][0])


@pytest.mark.parametrize("tag", ["v3.0.0", "v3.0.0-rc.1", "release/v3.0.0-rc.1"])
def test_the_tag_filter_ignores_stable_and_foreign_tags(tag):
    assert not fnmatch.fnmatchcase(tag, TRIGGERS["push"]["tags"][0])


def test_dispatch_defaults_to_a_dry_run():
    inputs = TRIGGERS["workflow_dispatch"]["inputs"]
    assert inputs["dry-run"]["default"] == "true"
    assert inputs["tag"]["required"] == "true"


def test_no_run_script_interpolates_an_expression():
    scripts = [
        (job, s.get("name", "?"), s["run"])
        for job, body in JOBS.items()
        for s in body.get("steps", [])
        if "run" in s
    ]
    assert scripts, "expected run steps to check"
    assert [(job, name) for job, name, run in scripts if "${{" in run] == []


def test_the_tag_is_validated_first_and_only_through_the_environment():
    parse = JOBS["validate"]["steps"][1]
    assert parse["id"] == "parse"
    assert parse["env"] == {"TAG": "${{ inputs.tag || github.ref_name }}"}
    assert "validate" not in JOBS["validate"].get("needs", [])
    for job, body in JOBS.items():
        if job != "validate":
            assert "validate" in body["needs"], job


def test_workflow_default_permissions_grant_nothing():
    assert DOC["permissions"] == {}


def test_each_job_asks_for_the_least_it_needs():
    granted = {job: body["permissions"] for job, body in JOBS.items()}
    assert granted == {
        "validate": {"contents": "read"},
        "evals": {"contents": "read"},
        "wheels": {"contents": "read"},
        "archives": {"contents": "read"},
        "sign": {"contents": "read"},
        "publish": {"id-token": "write"},
        "prerelease": {
            "contents": "write",
            "id-token": "write",
            "attestations": "write",
        },
    }


def test_publishing_waits_for_the_pypi_environment_and_every_build():
    publish = JOBS["publish"]
    assert publish["environment"]["name"] == "pypi"
    assert set(publish["needs"]) == {
        "validate",
        "evals",
        "wheels",
        "archives",
        "sign",
    }
    assert publish["if"] == "${{ needs.validate.outputs.publish == 'true' }}"
    assert set(JOBS["prerelease"]["needs"]) >= {"publish", "wheels", "archives"}


def test_every_external_action_is_pinned_by_commit():
    refs = [
        s["uses"]
        for body in JOBS.values()
        for s in body.get("steps", [])
        if "uses" in s
    ]
    assert refs
    unpinned = [r for r in refs if not re.fullmatch(r"[\w./-]+@[0-9a-f]{40}", r)]
    assert unpinned == []


def run_publish_decision(
    tmp_path: Path, **env: str
) -> subprocess.CompletedProcess[str]:
    script = step("validate", "Decide whether this run may publish")["run"]
    output = tmp_path / "github_output"
    output.write_text("")
    done = subprocess.run(
        ["bash", "-c", script],
        env={"PATH": os.environ["PATH"], "GITHUB_OUTPUT": str(output), **env},
        capture_output=True,
        text=True,
        check=False,
    )
    done.stdout = output.read_text().strip()
    return done


@pytest.mark.parametrize(
    ("env", "published"),
    [
        ({"EVENT": "push"}, "publish=true"),
        ({"EVENT": "workflow_dispatch", "DRY_RUN": "true"}, "publish=false"),
        (
            {
                "EVENT": "workflow_dispatch",
                "DRY_RUN": "false",
                "INPUT_TAG": CANDIDATE_TAG,
            },
            "publish=true",
        ),
    ],
)
def test_a_tag_push_or_a_real_dispatch_on_the_tag_publishes(tmp_path, env, published):
    base = {
        "DRY_RUN": "",
        "INPUT_TAG": "",
        "REF_TYPE": "tag",
        "REF_NAME": CANDIDATE_TAG,
    }
    done = run_publish_decision(tmp_path, **{**base, **env})
    assert done.returncode == 0, done.stderr
    assert done.stdout == published


@pytest.mark.parametrize(
    ("ref_type", "ref_name"),
    [("branch", "main"), ("branch", CANDIDATE_TAG), ("tag", "rc/v3.0.0-rc.9")],
)
def test_a_real_dispatch_run_must_start_on_the_input_tag(tmp_path, ref_type, ref_name):
    done = run_publish_decision(
        tmp_path,
        EVENT="workflow_dispatch",
        DRY_RUN="false",
        INPUT_TAG=CANDIDATE_TAG,
        REF_TYPE=ref_type,
        REF_NAME=ref_name,
    )
    assert done.returncode == 1
    assert "dry-run is off" in done.stderr
    assert done.stdout == ""


# The contract with scripts/release_provenance.py (check-stable).


def run_the_workflow_would_record(event: str = "push") -> dict:
    """The fields of the API run that a pushed candidate tag produces."""
    return {
        "event": event,
        "status": "completed",
        "conclusion": "success",
        "head_sha": SHA,
        "head_branch": CANDIDATE_TAG,
        "path": WORKFLOW.relative_to(ROOT).as_posix(),
    }


def gate_runner(runs: list[dict]):
    def runner(argv):
        if argv[:2] == ["git", "tag"]:
            return CANDIDATE_TAG
        if argv[:2] == ["git", "rev-parse"]:
            return "tree1" if argv[2].endswith("^{tree}") else SHA
        assert argv[:2] == ["gh", "api"]
        return json.dumps({"workflow_runs": runs})

    return runner


def test_the_gate_accepts_a_passed_push_run_of_this_workflow_file():
    runner = gate_runner([run_the_workflow_would_record()])
    assert provenance.check_stable("v3.0.0", "o/r", SHA, runner) == CANDIDATE_TAG


def test_the_gate_does_not_count_a_dispatch_run():
    runner = gate_runner([run_the_workflow_would_record(event="workflow_dispatch")])
    with pytest.raises(ValueError, match="no passed candidate"):
        provenance.check_stable("v3.0.0", "o/r", SHA, runner)


def test_the_parse_step_emits_the_outputs_the_jobs_read():
    lines = subprocess.run(
        [sys.executable, str(SCRIPT), "parse-tag", CANDIDATE_TAG],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.splitlines()
    emitted = {line.split("=", 1)[0] for line in lines}
    read = {
        name
        for key in ("cargo-version", "pypi-version")
        for name in re.findall(
            r"steps\.parse\.outputs\.(\w+)", JOBS["validate"]["outputs"][key]
        )
    }
    assert read == emitted == {"cargo", "pypi"}


# Behaviour of the guard steps, run as the workflow runs them.


def run_step(job: str, name: str, cwd: Path, env: dict[str, str]):
    script = step(job, name)["run"]
    return subprocess.run(
        ["bash", "-c", script],
        cwd=cwd,
        env={"PATH": os.environ["PATH"], **env},
        capture_output=True,
        text=True,
        check=False,
    )


@pytest.mark.parametrize(
    ("tag", "cargo", "pypi"),
    [
        ("rc/v3.0.0-rc.1", "3.0.0-rc.1", "3.0.0rc1"),
        ("rc/v3.1.20-rc.12", "3.1.20-rc.12", "3.1.20rc12"),
    ],
)
def test_the_parse_step_writes_both_versions_for_a_candidate_tag(
    tmp_path, tag, cargo, pypi
):
    output = tmp_path / "out"
    output.write_text("")
    done = run_step(
        "validate", "Parse the tag", ROOT, {"TAG": tag, "GITHUB_OUTPUT": str(output)}
    )
    assert done.returncode == 0, done.stderr
    assert output.read_text().splitlines() == [f"cargo={cargo}", f"pypi={pypi}"]


@pytest.mark.parametrize(
    "tag",
    [
        "v3.0.0",
        "3.0.0-rc.1",
        "rc/3.0.0-rc.1",
        "rc/v3.0.0",
        "rc/v3.0.0-rc.0",
        "rc/v3.0.0-rc.01",
        "rc/v03.0.0-rc.1",
        "rc/v3.00.0-rc.1",
        "rc/v3.0.00-rc.1",
        "rc/v3.0.0-rc.1\u0663",
        "rc/v3.0-rc.1",
        "x/rc/v3.0.0-rc.1",
        "rc/v3.0.0-rc.1/extra",
        "rc/v3.0.0-rc.1 ",
        "rc/v3.0.0-rc.1$(id)",
        "rc/v3.0.0-rc.1`id`",
        "rc/v3.0.0-rc.1;id",
        "",
    ],
)
def test_the_parse_step_rejects_anything_but_a_candidate_tag(tmp_path, tag):
    output = tmp_path / "out"
    output.write_text("")
    done = run_step(
        "validate", "Parse the tag", ROOT, {"TAG": tag, "GITHUB_OUTPUT": str(output)}
    )
    assert done.returncode != 0
    assert output.read_text() == ""


GIT_ENV = {
    "GIT_AUTHOR_NAME": "t",
    "GIT_AUTHOR_EMAIL": "t@example.com",
    "GIT_COMMITTER_NAME": "t",
    "GIT_COMMITTER_EMAIL": "t@example.com",
    "GIT_CONFIG_GLOBAL": os.devnull,
    "GIT_CONFIG_SYSTEM": os.devnull,
}


def git(cwd: Path, *args: str) -> str:
    return subprocess.run(
        ["git", *args],
        cwd=cwd,
        env={"PATH": os.environ["PATH"], **GIT_ENV},
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()


@pytest.fixture
def clone(tmp_path):
    """A checkout whose origin has one commit per branch: main, main-foo, feature,
    release-please--branches--main, and release-please--x."""
    origin = tmp_path / "origin.git"
    seed = tmp_path / "seed"
    git(tmp_path, "init", "-q", "--bare", "-b", "main", str(origin))
    git(tmp_path, "init", "-q", "-b", "main", str(seed))
    git(seed, "remote", "add", "origin", str(origin))
    shas = {}
    for branch in (
        "main",
        "main-foo",
        "feature",
        "release-please--branches--main",
        "release-please--x",
    ):
        git(seed, "checkout", "-q", "--orphan", branch)
        (seed / "f").write_text(branch)
        git(seed, "add", "f")
        git(seed, "commit", "-q", "-m", branch)
        git(seed, "push", "-q", "origin", branch)
        shas[branch] = git(seed, "rev-parse", "HEAD")
    work = tmp_path / "work"
    git(tmp_path, "clone", "-q", str(origin), str(work))
    return work, shas


@pytest.mark.parametrize(
    ("branch", "passes"),
    [
        ("main", True),
        ("release-please--branches--main", True),
        ("main-foo", False),
        ("feature", False),
        ("release-please--x", False),
    ],
)
def test_the_ancestry_guard_accepts_only_main_and_the_release_please_main_branch(
    clone, branch, passes
):
    work, shas = clone
    done = run_step(
        "validate",
        "Require the commit to be on main or the release PR branch",
        work,
        {**GIT_ENV, "GITHUB_SHA": shas[branch]},
    )
    assert (done.returncode == 0) is passes, done.stderr
    if not passes:
        assert "not on main" in done.stderr


def make_wheels(directory: Path, names: list[str]) -> None:
    directory.mkdir()
    for name in names:
        (directory / name).write_text("")


WHEEL_STEP = "Check the wheel versions"
TARGET_TAGS = [
    "macosx_11_0_arm64",
    "macosx_10_12_x86_64",
    "manylinux_2_17_x86_64",
    "manylinux_2_17_aarch64",
    "musllinux_1_2_x86_64",
]


def wheel_names(version: str) -> list[str]:
    return [f"slopvac-{version}-py3-none-{t}.whl" for t in TARGET_TAGS]


def run_wheel_guard(tmp_path: Path, names: list[str] | None, expected="3.0.0rc2"):
    if names is not None:
        make_wheels(tmp_path / "dist", names)
    return run_step("publish", WHEEL_STEP, tmp_path, {"EXPECTED": expected})


def test_the_wheel_guard_accepts_a_full_set_of_the_expected_version(tmp_path):
    done = run_wheel_guard(tmp_path, wheel_names("3.0.0rc2"))
    assert done.returncode == 0, done.stderr


@pytest.mark.parametrize(
    "names",
    [
        wheel_names("3.0.0rc1"),
        wheel_names("3.0.0rc20"),
        wheel_names("3.0.0"),
        wheel_names("3.0.0rc2")[:-1] + wheel_names("2.18.1")[:1],
    ],
    ids=["older-rc", "longer-rc", "stable", "one-stray"],
)
def test_the_wheel_guard_rejects_a_wheel_of_another_version(tmp_path, names):
    done = run_wheel_guard(tmp_path, names)
    assert done.returncode == 1
    assert "unexpected wheel" in done.stderr


def test_the_wheel_guard_rejects_an_empty_directory(tmp_path):
    done = run_wheel_guard(tmp_path, [])
    assert done.returncode == 1
    assert "no wheels downloaded" in done.stderr


def test_the_wheel_guard_rejects_a_missing_directory(tmp_path):
    done = run_wheel_guard(tmp_path, None)
    assert done.returncode == 1
    assert "no wheels downloaded" in done.stderr


# --- minisign signing of the candidate assets -----------------------------------


def test_candidate_assets_are_signed_in_a_job_that_gates_the_pypi_upload():
    names = [s.get("name") for s in JOBS["sign"]["steps"]]
    order = [
        names.index(n)
        for n in (
            "Download archives and wheels",
            "Install minisign",
            "Sign the release artifacts",
            "Upload signatures",
        )
    ]
    assert order == sorted(order)
    signing = step("sign", "Sign the release artifacts")
    assert signing["run"] == "python3 scripts/release_signing.py sign artifacts"
    # Signing must finish before the irreversible PyPI upload and the prerelease.
    assert "sign" in JOBS["publish"]["needs"]
    assert "sign" in JOBS["prerelease"]["needs"]
    assert JOBS["sign"]["if"] == "${{ needs.validate.outputs.publish == 'true' }}"


def test_the_prerelease_attaches_the_signatures_without_the_secret():
    download = step("prerelease", "Download archives and wheels")
    assert download["with"]["pattern"] == "{archive,wheels,signatures}-*"
    assert "artifacts/*" in step("prerelease", "Create the prerelease")["run"]
    assert "MINISIGN" not in str(JOBS["prerelease"])


def test_the_signing_secret_is_in_the_candidate_signing_step_env_only():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert text.count("secrets.MINISIGN_SECRET_KEY") == 1
    holders = [
        s["name"] for s in JOBS["sign"]["steps"] if "MINISIGN_SECRET_KEY" in str(s)
    ]
    assert holders == ["Sign the release artifacts"]
    assert "MINISIGN_PASSWORD" not in text


def test_a_publishing_run_requires_the_public_key_before_anything_is_built():
    guard = step("validate", "Require the release public key")
    assert guard["if"] == "${{ steps.mode.outputs.publish == 'true' }}"
    assert guard["run"] == "python3 scripts/release_signing.py require-key"
