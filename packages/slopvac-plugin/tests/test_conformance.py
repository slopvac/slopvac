"""The conformance suite, run against good and deliberately broken plugins."""

import json
import subprocess
import sys
import tomllib
from pathlib import Path

import pytest
from conftest import EXAMPLE, script
from slopvac_plugin.conformance import SuiteError, load_declaration, run_conformance

ALL_IDS = [f"P{i}" for i in range(1, 20)]
REPO = Path(__file__).resolve().parents[3]
PACK_SCHEMA = REPO / "schemas" / "v3" / "pack-manifest.schema.json"

ALL_HOOKS_INPUTS = {
    "provider": {},
    "tagger": {"invalid_document": {"id": "bad", "text": "a\u0000b"}},
    "extractor": {"invalid_document": {"id": "bad", "text": "a\u0000b"}},
    "fixer": {"invalid_document": {"id": "bad", "text": "a\u0000b", "findings": []}},
    "scorer": {"invalid_document": {"id": "bad", "text": "a\u0000b", "findings": []}},
}


def statuses(report):
    return {r.id: r.status for r in report.results}


def test_example_plugin_passes_every_applicable_case(quiet_cwd):
    command = [sys.executable, str(EXAMPLE / "plugin.py")]
    inputs = json.loads((EXAMPLE / "conformance-inputs.json").read_text("utf-8"))
    report = run_conformance(
        command,
        declaration=load_declaration(EXAMPLE / "slopvac-pack.toml"),
        inputs=inputs,
    )
    assert report.failed == []
    assert report.exit_code == 0
    # The example declares a tagger only, so the provider, recipe and fixer cases skip.
    skipped = {r.id for r in report.results if r.status == "skip"}
    assert skipped == {"P16", "P17", "P18"}
    assert [r.id for r in report.results] == ALL_IDS


def test_plugin_with_every_hook_kind_passes_all_19_cases(tmp_path, quiet_cwd):
    declaration = {
        "network": True,
        "hooks": [
            {"kind": "tagger", "id": "words"},
            {"kind": "extractor", "id": "lines"},
            {"kind": "reporter", "id": "plain"},
            {"kind": "fixer", "id": "upper"},
            {"kind": "scorer", "id": "count"},
            {"kind": "provider", "id": "echo"},
            {"kind": "recipe", "id": "files"},
        ],
        "features": {"required": [], "optional": ["document-path"]},
    }
    path = tmp_path / "declaration.json"
    path.write_text(json.dumps(declaration), encoding="utf-8")
    report = run_conformance(
        script("all_hooks_plugin.py"),
        declaration=load_declaration(path),
        inputs=ALL_HOOKS_INPUTS,
    )
    assert statuses(report) == dict.fromkeys(ALL_IDS, "pass"), report.to_text()


def test_plugin_written_without_the_sdk_passes(quiet_cwd):
    report = run_conformance(script("raw_plugin.py"))
    assert report.failed == [], report.to_text()


@pytest.mark.parametrize(
    ("mode", "failing"),
    [
        ("no-limit", "P13"),  # reads and answers a body over maxMessageBytes
        ("stdout-noise", "P14"),  # writes a stray line to standard output
        ("bad-span", "P15"),  # span ends past the text
        ("split-char-span", "P15"),  # span ends inside a multi-byte character
        ("exit-zero-without-shutdown", "P11"),  # exit status 0 where 1 is required
        ("no-version-check", "P2"),  # accepts a version range without 1
    ],
)
def test_suite_fails_a_broken_plugin_on_the_violated_case(mode, failing, quiet_cwd):
    report = run_conformance(script("raw_plugin.py", mode), timeout=5)
    assert statuses(report)[failing] == "fail", report.to_text()
    assert report.exit_code == 1


def test_a_failure_names_the_reason(quiet_cwd):
    report = run_conformance(script("raw_plugin.py", "bad-span"), only=["P15"])
    assert "outside" in report.failed[0].detail


def test_missing_declaration_skips_p19_with_a_reason(quiet_cwd):
    report = run_conformance(script("raw_plugin.py"), only=["P19"])
    assert report.results[0].status == "skip"
    assert "--manifest" in report.results[0].detail


def test_p19_fails_a_reply_wider_than_the_declaration(quiet_cwd):
    declaration = {
        "network": False,
        "hooks": [],
        "features": {"required": [], "optional": []},
    }
    report = run_conformance(
        script("raw_plugin.py"), declaration=declaration, only=["P19"]
    )
    assert statuses(report) == {"P19": "fail"}
    assert "tagger" in report.failed[0].detail


def test_command_that_does_not_start_is_incomplete_not_failed(quiet_cwd):
    with pytest.raises(SuiteError):
        run_conformance(["/nonexistent/plugin-binary"])


def cli(*args, cwd):
    return subprocess.run(
        [sys.executable, "-m", "slopvac_plugin.conformance", *args],
        check=False,
        capture_output=True,
        text=True,
        cwd=cwd,
        timeout=120,
    )


def test_cli_exit_0_and_text_report_for_a_passing_plugin(tmp_path):
    done = cli("--only", "P1,P3", "--", *script("raw_plugin.py"), cwd=tmp_path)
    assert done.returncode == 0, done.stdout + done.stderr
    assert done.stdout.splitlines()[-1] == "2 passed, 0 failed, 0 skipped"


def test_cli_exit_1_and_json_report_for_a_failing_plugin(tmp_path):
    done = cli(
        "--only",
        "P11",
        "--format",
        "json",
        "--",
        *script("raw_plugin.py", "exit-zero-without-shutdown"),
        cwd=tmp_path,
    )
    assert done.returncode == 1
    report = json.loads(done.stdout)
    assert report["ok"] is False
    assert [(r["id"], r["status"]) for r in report["results"]] == [("P11", "fail")]


def test_cli_exit_2_when_the_command_does_not_start(tmp_path):
    done = cli("--", "/nonexistent/plugin-binary", cwd=tmp_path)
    assert done.returncode == 2
    assert "incomplete" in done.stderr


@pytest.mark.skipif(
    not PACK_SCHEMA.exists(), reason="pack-manifest schema lands with PR #288"
)
def test_example_manifest_validates_against_the_pack_manifest_schema():
    from jsonschema import Draft202012Validator
    from referencing import Registry, Resource

    schemas = [
        json.loads(p.read_text("utf-8"))
        for p in (REPO / "schemas" / "v3").glob("*.schema.json")
    ]
    registry = Registry().with_resources(
        [(s["$id"], Resource.from_contents(s)) for s in schemas]
    )
    schema = json.loads(PACK_SCHEMA.read_text("utf-8"))
    manifest = tomllib.loads((EXAMPLE / "slopvac-pack.toml").read_text("utf-8"))
    errors = list(Draft202012Validator(schema, registry=registry).iter_errors(manifest))
    assert errors == []
