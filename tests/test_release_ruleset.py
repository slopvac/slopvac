from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "release_ruleset.py"
SPEC = importlib.util.spec_from_file_location("release_ruleset", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
rs = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = rs
SPEC.loader.exec_module(rs)

ALL_RULES = ["update", "deletion", "non_fast_forward"]


def ruleset(
    id_=1,
    *,
    target="tag",
    enforcement="active",
    include=("refs/tags/v*",),
    exclude=(),
    rules=ALL_RULES,
):
    return {
        "id": id_,
        "target": target,
        "enforcement": enforcement,
        "conditions": {
            "ref_name": {"include": list(include), "exclude": list(exclude)}
        },
        "rules": [{"type": r} for r in rules],
    }


def fake_api(
    rulesets: list[dict], *, summary_only_fields=("id", "target", "enforcement")
):
    """List endpoint returns summaries without rules; the item endpoint returns everything."""

    def runner(argv):
        assert argv[:2] == ["gh", "api"]
        path = argv[-1]
        if "/rulesets?" in path:
            query = dict(p.split("=") for p in path.split("?")[1].split("&"))
            page, per_page = int(query["page"]), int(query["per_page"])
            assert query["targets"] == "tag"
            batch = rulesets[(page - 1) * per_page : page * per_page]
            return json.dumps([{k: r[k] for k in summary_only_fields} for r in batch])
        ruleset_id = int(path.rsplit("/", 1)[1])
        return json.dumps(next(r for r in rulesets if r["id"] == ruleset_id))

    return runner


def check(rulesets, tag="v3.0.0"):
    rs.check_tag_protection(tag, "o/r", fake_api(rulesets))


def test_an_active_tag_ruleset_with_all_three_rules_passes():
    check([ruleset()])


def test_rules_may_come_from_several_active_rulesets():
    check(
        [
            ruleset(1, rules=["update"]),
            ruleset(2, rules=["deletion", "non_fast_forward"]),
        ]
    )


@pytest.mark.parametrize(
    "include",
    [
        ["~ALL"],
        ["refs/tags/*"],
        ["refs/tags/**"],
        ["refs/tags/**/*"],
        ["refs/tags/v*"],
        ["refs/tags/v3.0.0"],
    ],
)
def test_other_patterns_that_cover_the_tag_pass(include):
    check([ruleset(include=include)])


@pytest.mark.parametrize(
    ("rulesets", "message"),
    [
        ([], "deletion, non_fast_forward, update"),
        ([ruleset(enforcement="evaluate")], "update"),
        ([ruleset(enforcement="disabled")], "update"),
        ([ruleset(target="branch")], "update"),
        ([ruleset(include=["refs/tags/release-*"])], "update"),
        ([ruleset(include=["refs/heads/v*"])], "update"),
        ([ruleset(include=["refs/*"])], "update"),
        ([ruleset(include=["refs/**"])], "update"),
        ([ruleset(include=["~DEFAULT_BRANCH"])], "update"),
        ([ruleset(exclude=["refs/tags/**/v*"])], "update"),
        ([ruleset(exclude=["refs/tags/v?.0.0"])], "update"),
        ([ruleset(exclude=["refs/tags/[v]3.0.0"])], "update"),
        ([ruleset(exclude=["~ALL"])], "update"),
        ([ruleset(exclude=["refs/tags/v3.0.0"])], "update"),
        ([ruleset(exclude=["refs/tags/v3.*"])], "update"),
        ([ruleset(rules=["update", "deletion"])], "non_fast_forward"),
        ([ruleset(rules=["deletion", "non_fast_forward"])], "update"),
        ([ruleset(rules=["creation", "update", "non_fast_forward"])], "deletion"),
        (
            [
                ruleset(1, rules=["update"]),
                ruleset(
                    2, enforcement="disabled", rules=["deletion", "non_fast_forward"]
                ),
            ],
            "deletion",
        ),
    ],
)
def test_missing_or_inactive_protection_fails(rulesets, message):
    with pytest.raises(ValueError, match=message):
        check(rulesets)


@pytest.mark.parametrize(
    "exclude",
    [
        ["refs/heads/**"],
        ["refs/tags/release-*"],
        ["~DEFAULT_BRANCH"],
        ["refs/tags/v2.0.0"],
        ["refs/tags/legacy/**"],
    ],
)
def test_excludes_that_cannot_match_the_tag_do_not_exclude_it(exclude):
    check([ruleset(exclude=exclude)])


def test_a_tag_that_is_not_a_release_tag_is_rejected():
    with pytest.raises(ValueError, match="not a release tag"):
        check([ruleset()], tag="v3.0.0$(id)")


def test_an_api_error_fails_closed():
    def runner(argv):
        raise RuntimeError(
            "gh api failed: HTTP 403: Resource not accessible by integration"
        )

    with pytest.raises(RuntimeError, match="403"):
        rs.check_tag_protection("v3.0.0", "o/r", runner)


def test_a_non_list_response_fails_closed():
    with pytest.raises(TypeError, match="not a list"):
        rs.check_tag_protection(
            "v3.0.0", "o/r", lambda argv: json.dumps({"message": "Not Found"})
        )


def test_a_ruleset_on_the_second_page_counts():
    filler = [ruleset(i, enforcement="disabled") for i in range(1, rs.PER_PAGE + 1)]
    check([*filler, ruleset(rs.PER_PAGE + 1)])


def test_more_pages_than_the_cap_fail_closed():
    filler = [
        ruleset(i, enforcement="disabled")
        for i in range(1, rs.PER_PAGE * rs.MAX_PAGES + 1)
    ]
    with pytest.raises(ValueError, match="refusing to guess"):
        check([*filler, ruleset(rs.PER_PAGE * rs.MAX_PAGES + 1)])


def test_the_command_exits_one_with_the_hint_when_the_api_fails(capsys, monkeypatch):
    def boom(argv):
        raise RuntimeError("gh api failed: HTTP 403")

    monkeypatch.setattr(rs, "run_command", boom)
    assert rs.main(["check", "v3.0.0", "--repo", "o/r"]) == 1
    err = capsys.readouterr().err
    assert "HTTP 403" in err
    assert "refs/tags/v*" in err
