"""An installed SDK loads its vendored schemas, never the checkout's.

The tests copy the package into a ``site`` directory inside the checkout, so the
module resolves under the checkout but outside the SDK's source tree. That is the
layout of a virtual environment kept in a working copy.
"""

import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

import pytest

REPO = Path(__file__).parents[3]
PACKAGE = REPO / "packages" / "slopvac-plugin" / "src" / "slopvac_plugin"
SCHEMAS = REPO / "schemas" / "v3"

# The vendored copy gives each pattern a marker that the checkout's schemas lack.
CONTEXT_TAG = "^cx[a-z0-9-]*(?![\\s\\S])"
HOOK_ID = "^hk[a-z0-9-]*(?![\\s\\S])"


def run(code, site):
    """Run ``code`` in an isolated interpreter that sees ``site`` first."""
    prelude = f"import sys; sys.path.insert(0, {str(site)!r})\n"
    return subprocess.run(
        [sys.executable, "-I", "-c", prelude + code],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=False,
        timeout=120,
    )


def vendor(directory):
    """Copy the checkout's schemas to ``directory`` with the marker patterns."""
    shutil.copytree(SCHEMAS, directory)
    common = directory / "common.schema.json"
    document = json.loads(common.read_text("utf-8"))
    document["$defs"]["ContextTag"]["pattern"] = CONTEXT_TAG
    common.write_text(json.dumps(document))
    protocol = directory / "plugin-protocol.schema.json"
    document = json.loads(protocol.read_text("utf-8"))
    document["$defs"]["HookId"]["pattern"] = HOOK_ID
    protocol.write_text(json.dumps(document))


@pytest.fixture
def site():
    """An installed layout inside the checkout, removed afterwards."""
    root = Path(tempfile.mkdtemp(prefix=".pytest-sdk-site-", dir=REPO))
    try:
        installed = root / "site"
        shutil.copytree(
            PACKAGE,
            installed / "slopvac_plugin",
            ignore=shutil.ignore_patterns("__pycache__"),
        )
        vendor(installed / "slopvac_plugin" / "schemas")
        yield installed
    finally:
        shutil.rmtree(root, ignore_errors=True)


PROBE = """
import slopvac_plugin
from slopvac_plugin import Plugin
from slopvac_plugin.validate import KEBAB

print(slopvac_plugin.__file__)
plugin = Plugin("p", "1")
plugin.hook("tagger", "hk1")
try:
    plugin.hook("tagger", "ab1")
except ValueError:
    print(KEBAB.pattern)
else:
    raise SystemExit("hook ids are not checked against HookId")
"""


def test_an_installed_package_in_a_checkout_loads_its_vendored_patterns(site):
    result = run(PROBE, site)
    assert result.returncode == 0, result.stderr
    where, pattern = result.stdout.splitlines()
    assert Path(where).resolve().is_relative_to(site.resolve())
    assert pattern == CONTEXT_TAG.replace("(?![\\s\\S])", r"\Z")


def test_an_installed_package_without_its_schemas_refuses_to_import(site):
    found = run("import slopvac_plugin; print(slopvac_plugin.__file__)", site)
    assert found.returncode == 0, found.stderr
    assert Path(found.stdout.strip()).resolve().is_relative_to(site.resolve())
    vendored = site / "slopvac_plugin" / "schemas"
    assert vendored.is_dir()
    shutil.rmtree(vendored)
    result = run("import slopvac_plugin", site)
    assert result.returncode != 0
    assert "plugin-protocol.schema.json" in result.stderr
    assert "vendored" in result.stderr


@pytest.mark.parametrize(
    ("directory", "falls_back"), [("slopvac_plugin", True), ("other", False)]
)
def test_only_the_sdk_source_tree_may_use_the_repository_schemas(
    tmp_path, directory, falls_back
):
    """``…/src/slopvac_plugin`` exactly may fall back; a sibling directory may not."""
    source = tmp_path / "packages" / "slopvac-plugin" / "src"
    package = source / directory
    package.mkdir(parents=True)
    (package / "__init__.py").write_text("")
    shutil.copy(PACKAGE / "_schema.py", package / "_schema.py")
    shutil.copytree(SCHEMAS, tmp_path / "schemas" / "v3")
    result = run(
        f"from {directory}._schema import schema_dir; print(schema_dir())", source
    )
    if falls_back:
        assert result.returncode == 0, result.stderr
        assert (
            Path(result.stdout.strip()).resolve()
            == (tmp_path / "schemas" / "v3").resolve()
        )
    else:
        assert result.returncode != 0
        assert "vendored" in result.stderr
