import math

import pytest
from slopvac_plugin.errors import OutputInvalid
from slopvac_plugin.types import Edit, Finding, RecipeFile, Segment, Span, Tag
from slopvac_plugin.validate import (
    check_edits,
    check_recipe,
    check_score,
    check_segments,
    check_span,
    check_tag,
)

# "aé日" is 1 + 2 + 3 bytes: characters start at byte 0, 1 and 3, and it ends at 6.
MULTIBYTE = "aé日".encode()


@pytest.mark.parametrize("offset", [0, 1, 3, 6])
def test_span_on_character_boundaries_is_accepted(offset):
    check_span(MULTIBYTE, Span(offset, offset))


@pytest.mark.parametrize("offset", [2, 4, 5])
def test_span_inside_a_character_is_rejected(offset):
    with pytest.raises(OutputInvalid, match="boundary"):
        check_span(MULTIBYTE, Span(0, offset))
    with pytest.raises(OutputInvalid, match="boundary"):
        check_span(MULTIBYTE, Span(offset, 6))


@pytest.mark.parametrize(
    "span", [Span(0, 7), Span(3, 2), Span(-1, 2), Span(0, 2.0), Span(True, 2)]
)
def test_span_out_of_range_or_not_an_integer_is_rejected(span):
    with pytest.raises(OutputInvalid):
        check_span(MULTIBYTE, span)


def test_span_between_cr_and_lf_is_rejected():
    data = b"a\r\nb"
    check_span(data, Span(1, 3))
    with pytest.raises(OutputInvalid, match="boundary"):
        check_span(data, Span(0, 2))


@pytest.mark.parametrize("confidence", [-0.1, 1.1, True, "1", math.nan])
def test_tag_confidence_must_be_a_number_from_0_to_1(confidence):
    with pytest.raises(OutputInvalid):
        check_tag(b"abc", Tag("prose", Span(0, 1), confidence, "e"))


@pytest.mark.parametrize("confidence", [0, 1, 0.5])
def test_tag_confidence_bounds_are_accepted(confidence):
    check_tag(b"abc", Tag("prose", Span(0, 1), confidence, "e"))


def test_segments_must_be_in_source_order_without_overlap():
    first, second = Segment(Span(0, 3), "prose"), Segment(Span(3, 6), "code")
    check_segments(b"abcdef", [first, second])
    with pytest.raises(OutputInvalid, match="order"):
        check_segments(b"abcdef", [second, first])
    with pytest.raises(OutputInvalid, match="order"):
        check_segments(b"abcdef", [first, Segment(Span(2, 4), "code")])


FINDING = Finding("a:b", "warning", Span(2, 5), "m")
TEXT = b"0123456789"


@pytest.mark.parametrize(
    "spans",
    [
        [(2, 2)],  # zero-width at the start of the finding
        [(5, 5)],  # zero-width at the end
        [(3, 4)],
        [(2, 5)],
        [(2, 3), (3, 5)],  # adjacent
        [(2, 2), (2, 3)],  # insertion before an edit that starts there
    ],
)
def test_edits_inside_the_finding_are_accepted(spans):
    check_edits(TEXT, [FINDING], [Edit(0, Span(*s), "x") for s in spans])


@pytest.mark.parametrize(
    "spans",
    [
        [(1, 2)],  # one byte before
        [(5, 6)],  # one byte after
        [(0, 10)],  # whole document
        [(2, 4), (3, 5)],  # overlap
        [(3, 3), (3, 3)],  # two insertions at one point
        [(2, 5), (3, 3)],  # insertion inside another edit
    ],
)
def test_edits_outside_the_finding_or_overlapping_are_rejected(spans):
    with pytest.raises(OutputInvalid):
        check_edits(TEXT, [FINDING], [Edit(0, Span(*s), "x") for s in spans])


@pytest.mark.parametrize("finding", [1, -1, True])
def test_edit_must_name_a_selected_finding(finding):
    with pytest.raises(OutputInvalid):
        check_edits(TEXT, [FINDING], [Edit(finding, Span(2, 3), "x")])


@pytest.mark.parametrize(
    "path", ["a/b.md", ".github/x.yml", "..hidden", "a..b/c", "file"]
)
def test_recipe_accepts_relative_paths(path):
    check_recipe([RecipeFile(path, "write", "c")])


@pytest.mark.parametrize(
    "path", ["/abs", "../x", "a/../b", "a/..", "C:\\x", "C:/x", "a\\b", ""]
)
def test_recipe_rejects_unsafe_paths(path):
    with pytest.raises(OutputInvalid):
        check_recipe([RecipeFile(path, "write", "c")])


def test_recipe_write_and_merge_need_content_and_remove_does_not():
    check_recipe([RecipeFile("a", "remove")])
    for action in ("write", "merge"):
        with pytest.raises(OutputInvalid):
            check_recipe([RecipeFile("a", action)])


def test_recipe_rejects_an_unknown_action():
    with pytest.raises(OutputInvalid):
        check_recipe([RecipeFile("a", "chmod", "c")])


@pytest.mark.parametrize("score", [math.nan, math.inf, -0.5, True, "1"])
def test_score_must_be_finite_and_not_negative(score):
    with pytest.raises(OutputInvalid):
        check_score(score)


def test_score_zero_and_integers_are_accepted():
    assert check_score(0) == 0.0
    assert check_score(3) == 3.0
