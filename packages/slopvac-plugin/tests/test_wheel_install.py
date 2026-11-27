"""An installed SDK loads its vendored schemas, never the checkout's."""

import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

import pytest

REPO = Path(__file__).parents[3]
PACKAGE = REPO / "packages" / "slopvac-plugin"
UV = shutil.which("uv")

pytestmark = pytest.mark.skipif(UV is None, reason="uv is needed to build a wheel")

# The scratch build gives each pattern a marker that the checkout's schemas lack.
CONTEXT_TAG = "^cx[a-z0-9-]*(?![\\s\\S])"
HOOK_ID = "^hk[a-z0-9-]*(?![\\s\\S])"


def run(*command, cwd=REPO):
    return subprocess.run(
        command, cwd=cwd, capture_output=True, text=True, check=False, timeout=300
    )


@pytest.fixture(scope="module")
def wheel(tmp_path_factory):
    """A wheel built from a scratch tree whose vendored patterns differ."""
    tree = tmp_path_factory.mktemp("tree")
    shutil.copytree(
        PACKAGE,
        tree / "packages" / "slopvac-plugin",
        ignore=shutil.ignore_patterns("__pycache__", "tests", ".venv", "dist"),
    )
    schemas = tree / "schemas" / "v3"
    shutil.copytree(REPO / "schemas" / "v3", schemas)
    common = json.loads((schemas / "common.schema.json").read_text("utf-8"))
    common["$defs"]["ContextTag"]["pattern"] = CONTEXT_TAG
    (schemas / "common.schema.json").write_text(json.dumps(common))
    protocol = json.loads((schemas / "plugin-protocol.schema.json").read_text("utf-8"))
    protocol["$defs"]["HookId"]["pattern"] = HOOK_ID
    (schemas / "plugin-protocol.schema.json").write_text(json.dumps(protocol))
    out = tree / "dist"
    built = run(
        UV,
        "build",
        "--wheel",
        "--out-dir",
        str(out),
        str(tree / "packages" / "slopvac-plugin"),
    )
    assert built.returncode == 0, built.stderr
    return next(out.glob("*.whl"))


@pytest.fixture
def venv(wheel):
    """A venv inside the checkout, so an upward search would find ``schemas/v3``."""
    path = Path(tempfile.mkdtemp(prefix=".wheel-venv-", dir=REPO))
    try:
        made = run(UV, "venv", "--python", sys.executable, str(path))
        assert made.returncode == 0, made.stderr
        python = path / "bin" / "python"
        installed = run(UV, "pip", "install", "--python", str(python), str(wheel))
        assert installed.returncode == 0, installed.stderr
        yield python
    finally:
        shutil.rmtree(path, ignore_errors=True)


def site_schemas(python):
    found = run(
        str(python),
        "-I",
        "-c",
        "import importlib.util as u, pathlib;"
        "print(pathlib.Path(u.find_spec('slopvac_plugin').origin).parent / 'schemas')",
    )
    return Path(found.stdout.strip())


PROBE = """
from slopvac_plugin import Plugin
from slopvac_plugin.validate import KEBAB

plugin = Plugin("p", "1")
plugin.hook("tagger", "hk1")
try:
    plugin.hook("tagger", "ab1")
except ValueError:
    print(KEBAB.pattern)
else:
    raise SystemExit("hook ids are not checked against HookId")
"""


def test_an_installed_wheel_in_a_checkout_loads_its_vendored_patterns(venv):
    result = run(str(venv), "-I", "-c", PROBE)
    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == CONTEXT_TAG.replace("(?![\\s\\S])", r"\Z")


def test_an_installed_wheel_without_its_schemas_refuses_to_import(venv):
    shutil.rmtree(site_schemas(venv))
    result = run(str(venv), "-I", "-c", "import slopvac_plugin")
    assert result.returncode != 0
    assert "plugin-protocol.schema.json" in result.stderr
    assert "vendored" in result.stderr
