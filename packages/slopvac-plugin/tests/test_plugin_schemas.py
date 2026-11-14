"""The schema check bounds work on arrays whose uniqueness it must test."""

import pytest

NAMES = "capabilities/features/optional"


def reply(optional):
    return {
        "protocolVersion": 1,
        "pluginInfo": {"name": "p", "version": "1"},
        "capabilities": {
            "hooks": [],
            "features": {"required": [], "optional": optional},
            "requiresNetwork": False,
        },
        "limits": {"maxMessageBytes": 65536, "maxBatchDocuments": 1, "maxInFlight": 1},
    }


def problems(schemas, optional):
    return schemas.errors("InitializeResult", reply(optional))


def test_unique_features_up_to_the_cap_are_accepted(schemas):
    assert problems(schemas, [f"f{n}" for n in range(1024)]) == []


def test_more_features_than_the_cap_are_rejected(schemas):
    found = problems(schemas, [f"f{n}" for n in range(1025)])
    assert found and "1024" in found[0]


@pytest.mark.parametrize(
    "optional", [["a", "b", "a"], ["a", 1, "a"], [1, "a", "a"]], ids=str
)
def test_a_repeated_feature_is_rejected_even_beside_other_types(schemas, optional):
    assert any("unique" in p for p in problems(schemas, optional))


def test_a_feature_that_is_not_a_string_is_rejected(schemas):
    assert problems(schemas, ["a", 1])
