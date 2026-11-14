#![allow(missing_docs)]
//! Adversarial and generated HTML: no panics, valid positions, linear time.

use std::fmt::Write;
use std::time::{Duration, Instant};

use proptest::prelude::*;
use slopvac_model::{SourceIndex, Span};
use slopvac_parse::{Document, ParseError, TokenKind, WORK_BASE, WORK_PER_BYTE, parse_html_budget};

/// A budget no input reaches, for tests of the bounds that the budget sits above.
const UNLIMITED: usize = 1_000_000;

/// Parse `source` and check every position claim the contract makes.
fn checked(source: &str, body: Span) -> Document {
    let (doc, stats) = parse_html_budget(source, body, UNLIMITED, UNLIMITED).unwrap();
    assert_eq!(stats.marker_tags_fed, 0, "{source:?}");
    for block in &doc.blocks {
        assert!(body.start() <= block.span.start() && block.span.end() <= body.end());
        assert!(source.is_char_boundary(block.span.start()));
        assert!(source.is_char_boundary(block.span.end()));
        let Some(projection) = &block.projection else {
            continue;
        };
        let mut previous = block.span.start();
        for token in projection.tokens() {
            let (s, e) = (token.src.start(), token.src.end());
            assert!(previous <= s && s <= e, "tokens ascend: {token:?}");
            assert!(block.span.start() <= s && e <= block.span.end());
            let text = source.get(s..e).expect("token range is on char boundaries");
            match token.kind {
                TokenKind::Identity => assert_eq!(token.text, text),
                TokenKind::Entity => assert!(text.starts_with('&')),
                TokenKind::Collapse => {
                    assert_eq!(token.text, " ");
                    assert_ne!(text, "");
                }
                TokenKind::Wrap => assert_eq!(s, e),
                TokenKind::Masked => assert_eq!(token.text, "\0"),
                // The tokenizer's text always lines up with the source.
                TokenKind::Opaque | TokenKind::Backslash => {
                    panic!("unaligned text in {source:?}: {token:?}")
                }
            }
            previous = e;
        }
    }
    for span in &doc.code_spans {
        assert!(source.is_char_boundary(span.start()) && source.is_char_boundary(span.end()));
    }
    doc
}

fn whole(source: &str) -> Span {
    Span::new(0, source.len()).unwrap()
}

/// Run `f` and fail when it takes longer than `limit`. A quadratic scan over these inputs
/// takes minutes, so the limit only has to be generous, not tight.
fn within(limit: Duration, name: &str, f: impl FnOnce()) {
    let start = Instant::now();
    f();
    let took = start.elapsed();
    assert!(took < limit, "{name} took {took:?}");
}

#[test]
fn huge_input_shapes_finish_quickly_and_stay_valid() {
    let n = 200_000;
    let cases: Vec<(&str, String)> = vec![
        ("unclosed inline tags", "<b>x".repeat(n)),
        ("deep nesting of blocks", "<div>".repeat(n) + "text"),
        ("deep nesting of divs and text", "<div>w ".repeat(n)),
        (
            "huge attribute value",
            format!("<a title=\"{}\">link</a>", "x".repeat(2_000_000)),
        ),
        (
            "unterminated attribute",
            format!("<a title=\"{}", "x".repeat(2_000_000)),
        ),
        ("ampersands", "&".repeat(n * 2)),
        ("numeric reference starts", "&#".repeat(n)),
        ("unterminated names", "&amp".repeat(n)),
        ("many entities", "&amp;&#x41;&eacute;".repeat(n)),
        (
            "open code then stray ends",
            "<code>".repeat(n) + &"</pre>".repeat(n),
        ),
        ("open pre then text", "<pre>".repeat(n) + "text"),
        ("bare CR", "a\r".repeat(n)),
        ("CRLF runs", "a\r\n \r\n".repeat(n)),
        ("lt without tag", "< a ".repeat(n)),
        ("comment openers", "<!--".repeat(n)),
        ("many comments", "<!-- x -->y".repeat(n)),
        ("many cells", "<table><tr><td>a".repeat(n)),
        ("many br", "a<br>".repeat(n)),
        ("stray block ends", "</div>".repeat(n) + &"</li>".repeat(n)),
        (
            "open blocks then stray inline ends",
            "<div>".repeat(n) + &"</b>".repeat(n),
        ),
        (
            "open divs, pre, then divs ended",
            "<div>".repeat(n) + "<pre>" + &"</div>".repeat(n),
        ),
        (
            "open divs, code, then divs ended",
            "<div>".repeat(n) + &"<code>".repeat(n) + &"</div>".repeat(n),
        ),
        (
            "pre in div over and over",
            "<div><pre>".repeat(n) + &"</div>".repeat(n),
        ),
        ("nested svg", "<svg>".repeat(n) + "text"),
        (
            "svg titles",
            "<svg><title>".repeat(n) + &"</title>".repeat(n),
        ),
        ("svg with breakouts", "<svg><p>x".repeat(n)),
        ("inline ends in svg", "<b><svg></b>".repeat(n)),
        ("lists restarting", "<li>a<ul><li>b".repeat(n)),
        ("headings restarting", "<h1>a<h2>b".repeat(n)),
        ("tables restarting", "<table><tr><td>a<table>".repeat(n)),
        ("cdata in svg", "<svg><![CDATA[x]]>".repeat(n)),
        (
            "open templates",
            "<template>".repeat(n) + &"</div>".repeat(n),
        ),
    ];
    for (name, source) in cases {
        within(Duration::from_secs(20), name, || {
            checked(&source, whole(&source));
        });
    }
}

#[test]
fn mixed_newlines_keep_every_offset_valid() {
    let source = "<p>a\r\nb\rc\nd &amp;\r\n&lt;\r\n</p>\r\n<p>é\r</p>";
    let doc = checked(source, whole(source));
    assert_eq!(doc.blocks.len(), 2);
}

const PIECES: &[&str] = &[
    "<p>",
    "</p>",
    "<b>",
    "</b>",
    "<code>",
    "</code>",
    "<pre>",
    "</pre>",
    "<script>",
    "</script>",
    "<style>",
    "</style>",
    "<title>",
    "</title>",
    "<textarea>",
    "</textarea>",
    "<head>",
    "</head>",
    "<!--",
    "-->",
    "<!",
    "<?",
    "<![CDATA[",
    "]]>",
    "&amp;",
    "&",
    "&#x41;",
    "&#",
    "&#x",
    "&lt",
    "&notit;",
    "&nbsp;",
    "&bogus;",
    "&#0;",
    "&#xD800;",
    "a",
    "bc",
    "é",
    "日",
    "😀",
    " ",
    "  ",
    "\t",
    "\n",
    "\r\n",
    "\r",
    "<",
    ">",
    "/",
    "=",
    "\"",
    "'",
    "<br/>",
    "<br>",
    "<td>",
    "</td>",
    "<tr>",
    "<table>",
    "</table>",
    "<h2>",
    "</h2>",
    "<li>",
    "<a href=\"",
    "<a href='x&amp;y'>",
    "<div class=a>",
    "</div>",
    "<svg>",
    "<math>",
    "</",
    "<!doctype html>",
    "\u{feff}",
];

fn soup() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(PIECES), 0..80).prop_map(|v| v.concat())
}

proptest! {
    #[test]
    fn tag_soup_never_panics_and_positions_are_valid(source in soup()) {
        checked(&source, whole(&source));
    }

    #[test]
    fn arbitrary_text_never_panics_and_positions_are_valid(source in "\\PC{0,200}") {
        checked(&source, whole(&source));
    }

    #[test]
    fn any_valid_body_range_keeps_positions_inside_it(
        source in soup(),
        a in 0usize..400,
        b in 0usize..400,
    ) {
        // Snap both ends down to character boundaries, then order them.
        let snap = |mut i: usize| {
            i = i.min(source.len());
            while !source.is_char_boundary(i) {
                i -= 1;
            }
            i
        };
        let (x, y) = (snap(a), snap(b));
        let (start, end) = if x <= y { (x, y) } else { (y, x) };
        // Skip offsets the position contract rejects: inside a BOM or between CR and LF.
        let body = Span::new(start, end).unwrap();
        prop_assume!(SourceIndex::disk(&source).check_span(body).is_ok());
        checked(&source, body);
    }
}

/// Parse and return the counters. These bound the work the tree builder did, so they hold
/// the same on a slow machine.
fn stats(source: &str) -> slopvac_parse::Stats {
    let (_, stats) = parse_html_budget(source, whole(source), UNLIMITED, UNLIMITED).unwrap();
    stats
}

#[test]
fn nested_svg_and_group_pairs_cost_a_fixed_amount_each() {
    let n = 100_000;
    let source = "<svg><g>".repeat(n) + "x" + &"</g></svg>".repeat(n);
    let tags = 4 * n;
    let stats = stats(&source);
    // Every tag is one token or is dropped for depth, and each costs a fixed number of
    // changes to the tree.
    assert!(stats.tokens <= tags + 8, "{stats:?}");
    assert!(stats.tree_ops <= 4 * tags, "{stats:?}");
    assert!(stats.nodes <= 4 * tags, "{stats:?}");
    // The cap holds: nothing nests deeper than 512, so the rest are dropped.
    assert!(stats.dropped_deep >= 2 * n - 2 * 512, "{stats:?}");
}

#[test]
fn nested_tables_cost_a_fixed_amount_each() {
    let n = 100_000;
    let source = "<table><tr><td>".repeat(n) + "x";
    let stats = stats(&source);
    assert!(stats.tokens <= 3 * n + 8, "{stats:?}");
    assert!(stats.tree_ops <= 12 * n, "{stats:?}");
    // The table cap stops the nesting long before the depth cap.
    assert!(stats.tables <= 16, "{stats:?}");
    assert!(stats.tables_dropped >= n - 16, "{stats:?}");
}

#[test]
fn counting_the_open_elements_costs_the_cap_per_tag_at_most() {
    // Close one element and open another at the cap, over and over. Each pair makes the
    // parser count the stack once, and a count visits at most the cap plus a few.
    let n = 50_000;
    let source = "<div>".repeat(600) + &"</div><div>".repeat(n);
    let stats = stats(&source);
    assert!(stats.traced <= n * 530, "{stats:?}");
    assert!(stats.tree_ops <= 8 * n + 8 * 600, "{stats:?}");
}

#[test]
fn flat_siblings_never_hit_the_depth_cap() {
    let n = 50_000;
    let stats = stats(&"<li>a".repeat(n));
    assert_eq!(stats.dropped_deep, 0);
    // The stack is counted again once enough elements were created to fill it, and it
    // holds a few elements each time.
    assert!(stats.traced <= n / 8, "{stats:?}");
}

#[test]
fn nested_templates_are_skipped_not_nested() {
    // The scan skips a template and everything in it, however deep and however many are
    // open, so none of it reaches the tree builder or its stack.
    let templates = 50_000;
    let source = "<template>".repeat(templates) + &"<div>".repeat(600) + "text";
    let stats = stats(&source);
    assert_eq!(stats.marker_tags_fed, 0, "{stats:?}");
    assert!(stats.tokens <= 8, "{stats:?}");
    assert_eq!(stats.dropped_deep, 0, "{stats:?}");
}

#[test]
fn text_after_a_template_still_reaches_the_tree_builder() {
    let n = 20_000;
    let source = "<template><p>hidden</p></template><p>x</p>".repeat(n);
    let stats = stats(&source);
    assert_eq!(stats.marker_tags_fed, 0, "{stats:?}");
    assert!(stats.tokens >= 3 * n, "{stats:?}");
    assert!(stats.tree_ops <= 12 * n, "{stats:?}");
}

#[test]
fn attributes_the_parser_ignores_never_reach_the_tree_builder() {
    let n = 50_000;
    let attrs = (0..n).fold(String::new(), |mut out, i| {
        write!(out, " a{i}=x").unwrap();
        out
    });
    let source = format!("<p{attrs}>text</p><font{attrs}>x</font><input{attrs}>");
    let stats = stats(&source);
    // Only attributes that change how the tree builds can matter: `type`, `color`, `face`,
    // `size` and `encoding`.
    assert!(stats.attrs_passed <= 8, "{stats:?}");
}

#[test]
fn a_repeated_attribute_costs_a_fixed_amount() {
    let n = 50_000;
    let attrs: String = " size=1".repeat(n);
    let stats = stats(&format!("<font{attrs}>x</font>"));
    assert_eq!(stats.attrs_passed, 1, "{stats:?}");
}

#[test]
fn looking_for_an_end_tag_stops_after_a_fixed_number_of_tags() {
    // An unclosed `<code>` followed by a long run of unrelated end tags.
    let n = 100_000;
    let source = "<code>x".to_owned() + &"</b>".repeat(n);
    let stats = stats(&source);
    assert!(stats.end_scan > 0 && stats.end_scan <= 300, "{stats:?}");
}

#[test]
fn repeated_adoption_over_a_wide_parent_stays_linear() {
    // A formatting element ends over a block with many children, again and again.
    let n = 20_000;
    let source = "<b><div>".to_owned() + &"<span>x</span>".repeat(n) + &"</b><b>y".repeat(n);
    let stats = stats(&source);
    assert!(stats.tree_ops <= 8 * n, "{stats:?}");
}

#[test]
fn formatting_elements_that_differ_only_in_ignored_attributes_stay_bounded() {
    // The spec keeps at most three identical formatting elements. Ignored attributes must not
    // make them differ, or the list of active elements would grow with the input.
    let n = 20_000;
    let source = (0..n).fold(String::new(), |mut out, i| {
        write!(out, "<p><font color={i} size={i} id=a{i}>x</p>").unwrap();
        out
    });
    let stats = stats(&source);
    assert!(stats.nodes <= 8 * n, "{stats:?}");
    assert!(stats.tree_ops <= 20 * n, "{stats:?}");
}

#[test]
fn a_shallow_insertion_cannot_hide_a_deep_open_stack() {
    // `</body>` leaves the divs open, and the comment goes under the root. The depth cap
    // counts the open elements, not where the last node went.
    let n = 5_000;
    let source = "<div></body><!--c-->".repeat(n);
    let stats = stats(&source);
    assert!(stats.dropped_deep >= n - 600, "{stats:?}");
}

#[test]
fn formatting_elements_with_different_values_of_ignored_attributes_stay_bounded() {
    // `type` only matters on `<input>`. On a `<font>` it must not make elements differ, or
    // each paragraph would reconstruct all the fonts before it.
    let n = 3_000;
    let source = (0..n).fold(String::new(), |mut out, i| {
        write!(out, "<p><font type=u{i}>x<!--c-->").unwrap();
        out
    });
    let stats = stats(&source);
    assert!(stats.nodes <= 12 * n, "{stats:?}");
    assert!(stats.tree_ops <= 40 * n, "{stats:?}");
}

#[test]
fn a_document_over_the_work_budget_is_an_error() {
    let source = "<p>some <b>text</b> and more</p>".repeat(100);
    let body = whole(&source);
    assert!(parse_html_budget(&source, body, WORK_PER_BYTE, WORK_BASE).is_ok());
    // No budget at all, and a budget of one unit a byte, are both too little.
    assert!(matches!(
        parse_html_budget(&source, body, 0, 0),
        Err(ParseError::Budget)
    ));
    assert!(matches!(
        parse_html_budget(&source, body, 1, 0),
        Err(ParseError::Budget)
    ));
}

#[test]
fn the_budget_error_is_the_same_for_a_big_document() {
    // The scan stops feeding the tree builder once the budget is gone, so a hostile
    // document ends early instead of costing its full price.
    let source = "<b><div></b>".repeat(50_000);
    let result = parse_html_budget(&source, whole(&source), 4, 0);
    assert!(matches!(result, Err(ParseError::Budget)));
}

#[test]
fn markers_never_reach_the_tree_builder() {
    // `<template>`, `<object>`, `<applet>` and `<marquee>` push markers onto the tree
    // builder's list of formatting elements, and some pops leave the marker behind. The
    // scan keeps them out, so the list cannot grow without a matching stack.
    let n = 5_000;
    for open in [
        "<template><object></template>",
        "<template><template></template>",
        "<applet><marquee></applet>",
        "<object><b></object>",
    ] {
        let source = open.repeat(n) + &"<b></b>".repeat(n);
        let stats = stats(&source);
        assert_eq!(stats.marker_tags_fed, 0, "{open}: {stats:?}");
    }
}

#[test]
fn nested_tables_stop_at_the_table_cap() {
    let source = "<table><tr><td><b>x".repeat(200);
    let stats = stats(&source);
    assert!(stats.tables <= 16, "{stats:?}");
    assert!(stats.tables_dropped >= 200 - 16, "{stats:?}");
}

#[test]
fn an_input_over_the_size_cap_is_refused() {
    let big = "a".repeat(slopvac_parse::MAX_HTML_BYTES + 1);
    let result = parse_html_budget(&big, whole(&big), UNLIMITED, UNLIMITED);
    assert!(matches!(result, Err(ParseError::TooLarge)));
}

/// Time the worst shapes at the input cap. Run it in release mode:
/// `cargo test --release -p slopvac-parse --test html_robustness -- --ignored --nocapture`.
///
/// On the development machine (Apple M-series, release build) every shape finishes in under
/// 2 seconds, with the default budget and with none (measured: 0.2 to 1.6 seconds).
#[test]
#[ignore = "benchmark"]
fn worst_shapes_at_the_input_cap_finish_in_bounded_time() {
    let cap = slopvac_parse::MAX_HTML_BYTES;
    let fill = |unit: &str| unit.repeat(cap / unit.len());
    let shapes = [
        ("markers", fill("<template><object></template><b></b>")),
        ("depth thrash", fill("<b><div></b>")),
        (
            "tables and formatting",
            fill("<table><tr><td><b><i><u><s>x</table>"),
        ),
        ("cells in cells", fill("<table><td><b>x<table><td><i>y")),
        ("adoption", fill("<a>x<div>y</a>")),
        ("text", fill("word ")),
    ];
    for (name, source) in shapes {
        let body = whole(&source);
        let start = Instant::now();
        let guarded = parse_html_budget(&source, body, WORK_PER_BYTE, WORK_BASE).map(|_| ());
        let guarded_time = start.elapsed();
        let start = Instant::now();
        let free = parse_html_budget(&source, body, UNLIMITED, UNLIMITED).map(|_| ());
        println!(
            "{name:24} {} bytes: with budget {guarded_time:.2?} ({guarded:?}), without {:.2?} ({free:?})",
            source.len(),
            start.elapsed()
        );
        assert!(guarded_time < Duration::from_secs(30), "{name}");
    }
}
#[test]
fn templates_at_a_foreign_integration_point_never_reach_the_tree_builder() {
    // Inside SVG's `<foreignObject>` HTML rules apply, so a template there is a real one,
    // and with a cell it leaves a marker behind. The scan skips a template in every context.
    let n = 20_000;
    let source = "<svg><foreignObject>".to_owned()
        + &"<template><td></template>".repeat(n)
        + &"<b></b>".repeat(n);
    let stats = stats(&source);
    assert_eq!(stats.marker_tags_fed, 0, "{stats:?}");
}

#[test]
fn a_self_closed_svg_title_in_a_template_does_not_swallow_the_rest() {
    let source = "<template><svg><title/></svg></template><p>after</p>";
    let (doc, _) = parse_html_budget(source, whole(source), UNLIMITED, UNLIMITED).unwrap();
    assert_eq!(doc.blocks.len(), 1);
    let projection = doc.blocks[0].projection.as_ref().unwrap();
    assert_eq!(projection.text(), "after");
    let span = projection.span_for_range(0, 5);
    assert_eq!(&source[span.start()..span.end()], "after");
}

/// Pieces that a skipped template can hold without swallowing its own end tag: no raw text,
/// no comments, no templates, and nothing that hides the paragraph after it.
const SAFE: &[&str] = &[
    "x",
    " ",
    "\n",
    "&amp;",
    "é",
    "<b>",
    "</b>",
    "<p>",
    "</p>",
    "<div>",
    "</div>",
    "<td>",
    "<tr>",
    "<table>",
    "</table>",
    "<li>",
    "<br>",
    "<svg>",
    "</svg>",
    "<math>",
    "</math>",
    "<svg><title/>",
    "<template/>",
    "<svg><desc>",
    "</desc>",
    "<template/><td>",
    "<svg><foreignObject>",
    "</foreignObject>",
    "<span>",
    "</span>",
    "<i>",
    "</i>",
    "<h2>",
    "</h2>",
    "<ul>",
    "</ul>",
    "<a href=x>",
    "</a>",
    "<font color=red>",
    "<img>",
    "</>",
    "&bogus;",
    "<?pi>",
    "<!x>",
];

fn safe_soup() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(SAFE), 0..20).prop_map(|v| v.concat())
}

/// Markup without SVG, `MathML` or templates, for what comes before a context under test.
fn html_soup() -> impl Strategy<Value = String> {
    let atoms: Vec<&str> = SAFE
        .iter()
        .copied()
        .filter(|a| !a.contains("svg") && !a.contains("math") && !a.contains("template"))
        .collect();
    prop::collection::vec(prop::sample::select(atoms), 0..20).prop_map(|v| v.concat())
}

/// The same for inside a template. A `<template/>` there opens a nested template, because
/// HTML ignores the slash, so it would swallow the end tag that the test expects.
fn inside_soup() -> impl Strategy<Value = String> {
    let atoms: Vec<&str> = SAFE
        .iter()
        .copied()
        .filter(|a| !a.contains("<template/>"))
        .collect();
    prop::collection::vec(prop::sample::select(atoms), 0..20).prop_map(|v| v.concat())
}

proptest! {
    #[test]
    fn a_paragraph_after_a_skipped_template_keeps_its_text_and_position(
        before in safe_soup(),
        inside in inside_soup(),
        after in safe_soup(),
    ) {
        let source = format!(
            "{before}<template>{inside}</template>{after}</svg></math></table><p>SENTINEL</p>"
        );
        let (doc, _) = parse_html_budget(&source, whole(&source), UNLIMITED, UNLIMITED).unwrap();
        let found = doc
            .blocks
            .iter()
            .filter_map(|b| b.projection.as_ref())
            .find(|p| p.text() == "SENTINEL");
        let projection =
            found.ok_or_else(|| TestCaseError::fail(format!("no sentinel in {source:?}")))?;
        let span = projection.span_for_range(0, 8);
        prop_assert_eq!(&source[span.start()..span.end()], "SENTINEL");
    }
}
#[test]
fn self_closed_templates_at_an_integration_point_never_reach_the_tree_builder() {
    // Inside `<foreignObject>` a browser opens an inert template for `<template/>`, and with
    // a cell it leaves a marker. The scan drops the tag without skipping anything.
    let n = 20_000;
    let source = "<svg><foreignObject>".to_owned()
        + &"<template/><td></template>".repeat(n)
        + &"<b></b>".repeat(n);
    let stats = stats(&source);
    assert_eq!(stats.marker_tags_fed, 0, "{stats:?}");
}

#[test]
fn a_stray_template_end_tag_is_dropped() {
    let source = "</template><p>x</p></template>";
    let stats = stats(source);
    assert_eq!(stats.marker_tags_fed, 0, "{stats:?}");
}

#[test]
fn a_skipped_template_remembers_the_svg_around_it() {
    // The `<title/>` is an SVG element, not an HTML title that waits for its end tag.
    let source = "<svg><template><title/></template></svg><p>after</p>";
    let (doc, _) = parse_html_budget(source, whole(source), UNLIMITED, UNLIMITED).unwrap();
    assert_eq!(doc.blocks.len(), 1);
    let projection = doc.blocks[0].projection.as_ref().unwrap();
    assert_eq!(projection.text(), "after");
    let span = projection.span_for_range(0, 5);
    assert_eq!(&source[span.start()..span.end()], "after");
}

#[test]
fn a_self_closed_template_hides_nothing() {
    let source = "<p>a<template/>b</p>";
    let (doc, _) = parse_html_budget(source, whole(source), UNLIMITED, UNLIMITED).unwrap();
    assert_eq!(doc.blocks[0].projection.as_ref().unwrap().text(), "ab");
}
#[test]
fn a_self_closed_template_in_svg_inside_a_skipped_template_does_not_nest() {
    // In SVG `<template/>` is an empty element, so the template that holds it ends at the
    // first `</template>` and the paragraph after it shows.
    let source = "<template><svg><template/></svg></template><p>after</p>";
    let (doc, _) = parse_html_budget(source, whole(source), UNLIMITED, UNLIMITED).unwrap();
    assert_eq!(doc.blocks.len(), 1);
    let projection = doc.blocks[0].projection.as_ref().unwrap();
    assert_eq!(projection.text(), "after");
    let span = projection.span_for_range(0, 5);
    assert_eq!(&source[span.start()..span.end()], "after");
}

#[test]
fn raw_text_in_a_template_at_an_integration_point_holds_a_literal_end_tag() {
    // A template directly inside `<foreignObject>` holds HTML, so the `<textarea>` is RCDATA
    // and its `</template>` is text. Nothing inside shows.
    let source = "<svg><foreignObject><template><textarea></template><p>x</p></textarea></template><p>after</p>";
    let (doc, _) = parse_html_budget(source, whole(source), UNLIMITED, UNLIMITED).unwrap();
    let texts: Vec<_> = doc
        .blocks
        .iter()
        .filter_map(|b| b.projection.as_ref())
        .map(slopvac_parse::Projection::text)
        .collect();
    assert_eq!(texts, ["after"]);
}

/// Contexts that hold HTML, and so make raw text elements raw, around a template.
const HTML_CONTEXTS: &[&str] = &[
    "",
    "<svg><foreignObject>",
    "<svg><desc>",
    "<svg><title>",
    "<math><mi>",
    "<math><mtext>",
    "<math><annotation-xml encoding=text/html>",
    "<math><annotation-xml encoding=application/xhtml+xml>",
];

/// Raw text elements whose text holds a literal end tag of the template.
const RAW_WITH_END_TAG: &[&str] = &[
    "<textarea></template><p>LEAK</p></textarea>",
    "<title></template><p>LEAK</p></title>",
    "<style></template><p>LEAK</p></style>",
    "<script></template><p>LEAK</p></script>",
    "<xmp></template><p>LEAK</p></xmp>",
    "<iframe></template><p>LEAK</p></iframe>",
];

proptest! {
    #[test]
    fn raw_text_in_a_skipped_template_keeps_a_literal_end_tag(
        context in prop::sample::select(HTML_CONTEXTS),
        raw in prop::sample::select(RAW_WITH_END_TAG),
        before in html_soup(),
    ) {
        let source = format!(
            "{before}{context}<template>{raw}</template></svg></math><p>SENTINEL</p>"
        );
        let (doc, _) = parse_html_budget(&source, whole(&source), UNLIMITED, UNLIMITED).unwrap();
        let texts: Vec<&str> = doc
            .blocks
            .iter()
            .filter_map(|b| b.projection.as_ref())
            .map(slopvac_parse::Projection::text)
            .collect();
        prop_assert!(texts.iter().all(|t| !t.contains("LEAK")), "{source:?}: {texts:?}");
        prop_assert!(texts.contains(&"SENTINEL"), "{source:?}: {texts:?}");
    }
}
