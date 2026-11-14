import math
import time

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
        [(3, 3), (3, 3)],  # insertions at one point apply in array order
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


WIDE = Finding("a:b", "warning", Span(0, 10), "m")
WIDE_TEXT = b"0123456789"


def test_an_edit_far_from_an_earlier_wide_edit_still_overlaps_it():
    # [0,10) overlaps (4,5) even though (2,3) sits between them in start order.
    spans = [(0, 10), (2, 2), (4, 5)]
    with pytest.raises(OutputInvalid, match="overlap"):
        check_edits(WIDE_TEXT, [WIDE], [Edit(0, Span(*s), "x") for s in spans])


def test_a_thousand_edits_are_accepted():
    edits = [Edit(0, Span(i, i), "") for i in range(1000)]
    big = Finding("a:b", "warning", Span(0, 1000), "m")
    check_edits(b"x" * 1000, [big], edits)


def test_one_thousand_and_one_edits_are_refused_quickly():
    # 100,000 zero-width edits made the all-pairs check run about 5e9 comparisons.
    edits = [Edit(0, Span(i, i), "") for i in range(100_000)]
    big = Finding("a:b", "warning", Span(0, 100_000), "m")
    started = time.monotonic()
    with pytest.raises(OutputInvalid, match="1000"):
        check_edits(b"x" * 100_000, [big], edits)
    assert time.monotonic() - started < 1


def test_inserted_bytes_over_four_times_the_span_plus_4_kib_are_refused():
    # Span of 2 bytes: the limit is 4 * 2 + 4096 = 4104 bytes.
    finding = Finding("a:b", "warning", Span(0, 2), "m")
    check_edits(b"ab", [finding], [Edit(0, Span(0, 2), "x" * 4104)])
    with pytest.raises(OutputInvalid, match="bytes"):
        check_edits(b"ab", [finding], [Edit(0, Span(0, 2), "x" * 4105)])


def test_inserted_bytes_are_counted_as_utf8():
    finding = Finding("a:b", "warning", Span(0, 2), "m")
    # 1369 three-byte characters are 4107 bytes: over the limit, under it by length.
    with pytest.raises(OutputInvalid, match="bytes"):
        check_edits(b"ab", [finding], [Edit(0, Span(0, 2), "日" * 1369)])


def test_a_reply_with_hundreds_of_thousands_of_invalid_entries_is_checked_quickly(
    schemas,
):
    """Round 3, finding 3: only a few violations are kept, so the rest are not collected."""
    result = {"results": [{"document": 1, "tags": []}] * 400_000}
    started = time.monotonic()
    problems = schemas.errors("TagResult", result)
    assert problems
    assert len(problems) <= 3
    assert time.monotonic() - started < 2
