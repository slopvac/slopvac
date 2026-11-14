from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "release_provenance.py"
SPEC = importlib.util.spec_from_file_location("release_provenance", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
prov = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = prov
SPEC.loader.exec_module(prov)


@pytest.mark.parametrize(
    ("tag", "cargo", "pypi"),
    [
        ("rc/v3.0.0-rc.1", "3.0.0-rc.1", "3.0.0rc1"),
        ("rc/v3.10.2-rc.12", "3.10.2-rc.12", "3.10.2rc12"),
    ],
)
def test_candidate_tag_maps_to_cargo_and_pypi_versions(tag, cargo, pypi):
    assert prov.parse_candidate_tag(tag) == prov.Candidate(cargo, pypi)


@pytest.mark.parametrize(
    "tag",
    [
        "v3.0.0-rc.1",
        "rc/v3.0.0",
        "rc/v3.0-rc.1",
        "rc/v3.0.0-rc.0",
        "rc/v3.0.0-rc.1x",
        "rc/v3.0.0-beta.1",
    ],
)
def test_bad_candidate_tags_are_rejected(tag):
    with pytest.raises(ValueError, match="not a candidate tag"):
        prov.parse_candidate_tag(tag)


def good_run(candidate: str, **overrides):
    run = {
        "conclusion": "success",
        "status": "completed",
        "event": "push",
        "head_sha": f"sha-{candidate}",
        "head_branch": candidate,
        "path": ".github/workflows/candidate.yml",
    }
    return {**run, **overrides}


def make_runner(trees: dict[str, str], tags: list[str], runs: dict[str, list[dict]]):
    """Fake git and gh. ``runs`` maps a candidate commit sha to workflow run objects."""

    def runner(argv):
        if argv[:2] == ["git", "rev-parse"]:
            ref = argv[2]
            name, kind = ref.rsplit("^", 1)
            return (trees[name] if kind == "{tree}" else f"sha-{name}") + "\n"
        if argv[:2] == ["git", "tag"]:
            return "\n".join(tags)
        assert argv[:2] == ["gh", "api"]
        sha = argv[2].split("head_sha=")[1].split("&")[0]
        return json.dumps({"workflow_runs": runs.get(sha, [])})

    return runner


RC1 = "rc/v3.0.0-rc.1"
RC2 = "rc/v3.0.0-rc.2"


def test_stable_passes_with_a_successful_candidate_on_the_same_tree():
    runner = make_runner(
        {"v3.0.0": "T1", RC1: "T1"}, [RC1], {f"sha-{RC1}": [good_run(RC1)]}
    )
    assert prov.check_stable("v3.0.0", "o/r", runner) == RC1


def test_stable_uses_a_later_candidate_when_the_first_failed():
    runner = make_runner(
        {"v3.0.0": "T2", RC1: "T1", RC2: "T2"},
        [RC1, RC2],
        {f"sha-{RC2}": [good_run(RC2, conclusion="failure"), good_run(RC2)]},
    )
    assert prov.check_stable("v3.0.0", "o/r", runner) == RC2


@pytest.mark.parametrize(
    ("trees", "tags", "runs", "message"),
    [
        ({"v3.0.0": "T1"}, [], {}, "no candidate tag"),
        ({"v3.0.0": "T1", RC1: "T0"}, [RC1], {}, "tree differs"),
        ({"v3.0.0": "T1", RC1: "T1"}, [RC1], {}, "no successful"),
    ],
)
def test_stable_is_blocked_without_a_passed_candidate(trees, tags, runs, message):
    with pytest.raises(ValueError, match=message):
        prov.check_stable("v3.0.0", "o/r", make_runner(trees, tags, runs))


@pytest.mark.parametrize(
    "override",
    [
        {"conclusion": "failure"},
        {"conclusion": "cancelled"},
        {"status": "in_progress"},
        {"event": "workflow_dispatch"},
        {"head_branch": "main"},
        {"head_branch": RC2},
        {"head_sha": "sha-other"},
        {"path": ".github/workflows/publish-lint.yml"},
    ],
)
def test_a_run_must_be_the_candidate_workflow_push_run_of_that_tag_and_commit(override):
    runner = make_runner(
        {"v3.0.0": "T1", RC1: "T1"},
        [RC1],
        {f"sha-{RC1}": [good_run(RC1, **override)]},
    )
    with pytest.raises(ValueError, match="no successful"):
        prov.check_stable("v3.0.0", "o/r", runner)


def test_a_prerelease_tag_is_not_a_stable_tag():
    with pytest.raises(ValueError, match="not a stable tag"):
        prov.check_stable("v3.0.0-rc.1", "o/r", make_runner({}, [], {}))


def test_candidate_tags_of_other_versions_do_not_count():
    runner = make_runner({"v3.0.0": "T1"}, [], {})
    with pytest.raises(ValueError, match="no candidate tag rc/v3.0.0-rc.N"):
        prov.check_stable("v3.0.0", "o/r", runner)
