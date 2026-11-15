from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "set_workspace_version.py"
SPEC = importlib.util.spec_from_file_location("set_workspace_version", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
swv = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = swv
SPEC.loader.exec_module(swv)

MANIFEST = """\
[workspace]
members = ["crates/*"]

[workspace.package]
version = "0.0.0"
edition = "2024"

[workspace.dependencies]
other = { version = "1.0.0" }

[profile.release]
version = "untouched"
"""


def test_only_the_workspace_package_version_changes():
    updated = swv.set_workspace_version(MANIFEST, "3.0.0-rc.1")
    assert updated == MANIFEST.replace('version = "0.0.0"', 'version = "3.0.0-rc.1"')


@pytest.mark.parametrize("version", ["3.0", "v3.0.0", "3.0.0rc1", ""])
def test_non_semver_versions_are_rejected(version):
    with pytest.raises(ValueError, match="not a SemVer version"):
        swv.set_workspace_version(MANIFEST, version)


def test_manifest_without_workspace_package_is_rejected():
    with pytest.raises(ValueError, match="no \\[workspace.package\\]"):
        swv.set_workspace_version('[package]\nversion = "1.0.0"\n', "3.0.0")
