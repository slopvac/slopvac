"""Check the MegaLinter and Trunk descriptors under integrations/.

MegaLinter publishes a JSON schema for descriptors. The test downloads it from
oxsecurity/megalinter at a pinned commit (the project is AGPL-3.0, so it is not copied
into this repository) and skips offline. Trunk publishes no schema for plugin.yaml, so
the Trunk checks are structural: they assert the fields that carry the exit code and
output contract. Both descriptors' commands also run against the real CLI. Neither
descriptor can install slopvac until a 3.0 release is on PyPI.
"""

from __future__ import annotations

import json
import os
import re
import shlex
import shutil
import subprocess
import urllib.error
import urllib.request
from pathlib import Path

import pytest
import yaml
from jsonschema import Draft7Validator

ROOT = Path(__file__).resolve().parents[1]
MEGALINTER = (
    ROOT
    / "integrations/megalinter/mega-linter-plugin-slopvac/slopvac.megalinter-descriptor.yml"
)
SCHEMA_URL = (
    "https://raw.githubusercontent.com/oxsecurity/megalinter/"
    "a2eaa251451345c7ec11801f125e154f60b9d30e/"
    "megalinter/descriptors/schemas/megalinter-descriptor.jsonschema.json"
)
TRUNK = ROOT / "integrations/trunk/linters/slopvac/plugin.yaml"
SEMVER_PIN = re.compile(r"slopvac==(\d+\.\d+\.\d+)(?![\w.])")


def _load(path: Path) -> dict:
    return yaml.safe_load(path.read_text(encoding="utf-8"))


def _megalinter_linter() -> dict:
    (linter,) = _load(MEGALINTER)["linters"]
    return linter


def _trunk_linter() -> dict:
    (linter,) = _load(TRUNK)["lint"]["definitions"]
    return linter


def _megalinter_pin() -> str:
    (instruction,) = _megalinter_linter()["install"]["dockerfile"]
    match = SEMVER_PIN.search(instruction)
    assert match, instruction
    return match.group(1)


@pytest.fixture(scope="session")
def megalinter_schema() -> dict:
    """Download the descriptor schema at a pinned upstream commit.

    MegaLinter is AGPL-3.0, so the schema is fetched at test time, not vendored.
    Only a network failure outside CI skips. In CI any download error fails, and a
    response that isn't JSON fails everywhere.
    """
    try:
        with urllib.request.urlopen(SCHEMA_URL, timeout=30) as response:
            body = response.read()
    except urllib.error.HTTPError:
        raise
    except OSError as error:
        if os.environ.get("CI"):
            raise
        pytest.skip(f"offline: cannot download the MegaLinter schema: {error}")
    return json.loads(body)


def test_megalinter_descriptor_matches_upstream_schema(megalinter_schema):
    schema = megalinter_schema
    errors = sorted(
        Draft7Validator(schema).iter_errors(_load(MEGALINTER)),
        key=lambda e: list(e.path),
    )
    assert [e.message for e in errors] == []


def test_megalinter_plugin_location_follows_loader_rules():
    # https://megalinter.io/latest/plugins/: URLs must contain /mega-linter-plugin-
    # and the file must be named *.megalinter-descriptor.yml.
    assert "/mega-linter-plugin-" in MEGALINTER.as_posix()
    assert MEGALINTER.name.endswith(".megalinter-descriptor.yml")


def test_megalinter_linter_name_uses_descriptor_prefix():
    descriptor = _load(MEGALINTER)
    assert _megalinter_linter()["name"] == f"{descriptor['descriptor_id']}_SLOPVAC"


def test_megalinter_install_is_a_single_run_instruction():
    # Plugins may install only through `dockerfile` entries that start with RUN.
    install = _megalinter_linter()["install"]
    assert list(install) == ["dockerfile"]
    assert all(line.startswith("RUN ") for line in install["dockerfile"])


def test_megalinter_install_pins_an_exact_wheel_version():
    (instruction,) = _megalinter_linter()["install"]["dockerfile"]
    assert "latest" not in instruction
    assert "--only-binary=:all:" in instruction
    assert _megalinter_pin()


def test_megalinter_does_not_mask_the_exit_code():
    # MegaLinter fails a file on any nonzero exit, so exit 2 fails with exit 1. Setting
    # these keys could turn a failure into a pass.
    linter = _megalinter_linter()
    assert linter["cli_lint_extra_args"] == ["lint"]
    assert not {"help_command_return_code", "version_command_return_code"} & set(linter)
    assert "cli_lint_fix_arg_name" not in linter


def test_megalinter_sarif_args_write_to_the_report_file():
    linter = _megalinter_linter()
    assert linter["can_output_sarif"] is True
    assert linter["cli_sarif_args"] == [
        "--format",
        "sarif",
        "--out",
        "{{SARIF_OUTPUT_FILE}}",
    ]


def test_trunk_parses_sarif_from_the_output_file():
    command = _trunk_linter()["commands"][0]
    assert command["output"] == "sarif"
    assert command["read_output_from"] == "tmp_file"
    assert command["run"] == "slopvac lint --format sarif --out ${tmpfile} ${target}"


def test_trunk_fails_the_tool_on_an_incomplete_check():
    # Exit 1 means findings. Exit 2 is absent from success_codes, so Trunk reports a failure.
    assert _trunk_linter()["commands"][0]["success_codes"] == [0, 1]


def test_trunk_linter_runs_on_a_file_type_it_defines():
    definition = _load(TRUNK)
    (file_type,) = definition["lint"]["files"]
    assert _trunk_linter()["files"] == [file_type["name"]]


def test_trunk_tool_and_linter_share_one_version():
    definition = _load(TRUNK)
    (tool,) = definition["tools"]["definitions"]
    assert tool["package"] == "slopvac"
    assert _trunk_linter()["tools"] == [tool["name"]]
    assert re.fullmatch(r"\d+\.\d+\.\d+", str(tool["known_good_version"]))
    assert str(tool["known_good_version"]) == str(_trunk_linter()["known_good_version"])


def test_trunk_skips_native_windows():
    # 3.0 has no native Windows build; Windows users run Trunk under WSL.
    assert _trunk_linter()["supported_platforms"] == ["linux", "macos"]


def test_both_integrations_pin_the_same_release():
    (tool,) = _load(TRUNK)["tools"]["definitions"]
    assert str(tool["known_good_version"]) == _megalinter_pin()


@pytest.fixture(scope="session")
def slopvac_binary() -> Path:
    """Build the workspace CLI once; the argv tests run it."""
    cargo = shutil.which("cargo")
    if cargo is None:
        pytest.skip("cargo is not installed")
    subprocess.run([cargo, "build", "--quiet", "-p", "slopvac"], cwd=ROOT, check=True)
    return ROOT / "target" / "debug" / "slopvac"


def _megalinter_argv(report: Path, config: Path, target: Path) -> list[str]:
    linter = _megalinter_linter()
    sarif = [
        arg.replace("{{SARIF_OUTPUT_FILE}}", str(report))
        for arg in linter["cli_sarif_args"]
    ]
    return [
        *linter["cli_lint_extra_args"],
        linter["cli_config_arg_name"],
        str(config),
        *sarif,
        str(target),
    ]


def _trunk_argv(report: Path, config: Path, target: Path) -> list[str]:
    run = _trunk_linter()["commands"][0]["run"]
    run = run.replace("${tmpfile}", str(report)).replace("${target}", str(target))
    words = shlex.split(run)
    assert words[0] == "slopvac"
    return words[1:]


@pytest.mark.parametrize(
    "build_argv", [_megalinter_argv, _trunk_argv], ids=["megalinter", "trunk"]
)
def test_descriptor_command_is_accepted_by_the_cli(
    tmp_path, slopvac_binary, build_argv
):
    config = tmp_path / "slopvac.toml"
    config.write_text("", encoding="utf-8")
    target = tmp_path / "doc.md"
    target.write_text("# Title\n\nA short sentence.\n", encoding="utf-8")
    report = tmp_path / "report.sarif"
    done = subprocess.run(
        [str(slopvac_binary), *build_argv(report, config, target)],
        capture_output=True,
        text=True,
        check=False,
        cwd=tmp_path,
    )
    # Exit 2 covers a clap usage error and an incomplete check; both fail the integration.
    assert done.returncode in (0, 1), done.stderr
    assert '"version"' in report.read_text(encoding="utf-8")


def test_lint_help_lists_every_descriptor_flag(slopvac_binary):
    done = subprocess.run(
        [str(slopvac_binary), "lint", "--help"],
        capture_output=True,
        text=True,
        check=True,
    )
    for flag in ("--format", "--out", "--config"):
        assert flag in done.stdout
