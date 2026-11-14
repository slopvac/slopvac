"""The default requests the conformance suite sends are valid protocol messages."""

import json
from pathlib import Path

import pytest
from conftest import SDK_ENV, script
from jsonschema import Draft202012Validator
from referencing import Registry, Resource
from slopvac_plugin.conformance import load_declaration, run_conformance
from slopvac_plugin.conformance.samples import params_for, sample_report
from slopvac_plugin.conformance.schemas import Schemas

REPO_SCHEMAS = Path(__file__).resolve().parents[3] / "schemas" / "v3"
PARAMS_DEFS = {
    "tagger": "TagParams",
    "extractor": "ExtractParams",
    "reporter": "RenderParams",
    "fixer": "FixParams",
    "scorer": "ScoreParams",
    "provider": "CompleteParams",
    "recipe": "RecipeParams",
}


@pytest.mark.parametrize("kind", PARAMS_DEFS)
def test_default_sample_validates_against_its_params_schema(kind):
    params = params_for(kind, "hook-id", {})
    assert Schemas().errors(PARAMS_DEFS[kind], params) == []


def test_default_report_validates_against_the_lint_report_schema():
    schema = json.loads((REPO_SCHEMAS / "lint-report.schema.json").read_text("utf-8"))
    common = json.loads((REPO_SCHEMAS / "common.schema.json").read_text("utf-8"))
    registry = Registry().with_resources(
        [(s["$id"], Resource.from_contents(s)) for s in (schema, common)]
    )
    validator = Draft202012Validator(schema, registry=registry)
    assert [e.message for e in validator.iter_errors(sample_report())] == []


def test_reporter_that_reads_required_report_fields_passes_with_default_sample(
    tmp_path, quiet_cwd
):
    declaration = {
        "network": False,
        "hooks": [{"kind": "reporter", "id": "strict"}],
        "features": {"required": [], "optional": []},
    }
    path = tmp_path / "declaration.json"
    path.write_text(json.dumps(declaration), encoding="utf-8")
    report = run_conformance(
        script("strict_reporter_plugin.py"),
        declaration=load_declaration(path),
        pass_env=SDK_ENV,
    )
    assert report.failed == [], report.to_text()
