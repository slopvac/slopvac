from __future__ import annotations

import hashlib
import importlib.util
import sys
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "release_smoke.py"
SPEC = importlib.util.spec_from_file_location("release_smoke", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
smoke = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = smoke
SPEC.loader.exec_module(smoke)


@pytest.mark.parametrize(
    ("tag", "version", "stable"),
    [
        ("v3.0.0", "3.0.0", True),
        ("v10.20.30", "10.20.30", True),
        ("v3.0.0-rc.2", "3.0.0-rc.2", False),
    ],
)
def test_valid_tags_yield_a_version_and_stability(tag, version, stable):
    assert smoke.parse_tag(tag) == smoke.ReleaseTag(version, stable)


@pytest.mark.parametrize(
    "tag",
    [
        "3.0.0",
        "v3.0",
        "v3.0.0-rc.0",
        "v3.0.0-beta.1",
        "rc/v3.0.0-rc.1",
        "v3.0.0$(id)",
        "v3.0.0`id`",
        "v3.0.0\nx",
        "v3.0.0 ",
        "",
    ],
)
def test_malformed_tags_are_rejected(tag):
    with pytest.raises(ValueError, match="not a release tag"):
        smoke.parse_tag(tag)


@pytest.mark.parametrize(
    ("tag", "output"),
    [
        ("v3.0.0", "slopvac 3.0.0\n"),
        ("v3.0.0", "slopvac 3.0.0\r\n"),
        ("v3.0.0-rc.1", "slopvac 3.0.0-rc.1"),
    ],
)
def test_the_version_output_must_match_the_tag(tag, output):
    smoke.expect_version(tag, output)


@pytest.mark.parametrize(
    "output",
    [
        "slopvac 2.9.1",
        "slopvac 3.0.0-rc.1",
        "slopvac 3.0.01",
        "3.0.0",
        "slopvac, version 3.0.0",
    ],
)
def test_a_different_version_output_fails(output):
    with pytest.raises(ValueError, match="expected 'slopvac 3.0.0'"):
        smoke.expect_version("v3.0.0", output)


# sha256("abc"), a published test vector.
ABC_DIGEST = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"


@pytest.fixture
def archive(tmp_path):
    path = tmp_path / "slopvac-x.tar.xz"
    path.write_bytes(b"abc")
    return path


@pytest.mark.parametrize(
    "line",
    [
        f"{ABC_DIGEST}  slopvac-x.tar.xz\n",
        f"{ABC_DIGEST.upper()} *slopvac-x.tar.xz\n",
        ABC_DIGEST,
    ],
)
def test_a_matching_checksum_passes(archive, tmp_path, line):
    (tmp_path / "sum").write_text(line)
    smoke.verify_sha256(archive, tmp_path / "sum")


def test_a_wrong_checksum_fails(archive, tmp_path):
    (tmp_path / "sum").write_text(hashlib.sha256(b"abd").hexdigest() + "  x\n")
    with pytest.raises(ValueError, match="does not match"):
        smoke.verify_sha256(archive, tmp_path / "sum")


@pytest.mark.parametrize("content", ["", "not-a-digest  x\n", "abc123\n"])
def test_a_checksum_file_without_a_digest_fails(archive, tmp_path, content):
    (tmp_path / "sum").write_text(content)
    with pytest.raises(ValueError, match="no sha256 digest"):
        smoke.verify_sha256(archive, tmp_path / "sum")


def test_the_smoke_workflow_passes_the_tag_only_through_the_environment():
    text = (ROOT / ".github" / "workflows" / "release-smoke.yml").read_text(
        encoding="utf-8"
    )
    workflow = yaml.safe_load(text)
    assert workflow["permissions"] == {"contents": "read"}
    for job in workflow["jobs"].values():
        for step in job.get("steps", []):
            script = step.get("run", "")
            assert "${{" not in script, script
