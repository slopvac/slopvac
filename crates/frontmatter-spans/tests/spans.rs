//! Span invariants over a corpus, plus named regressions.

mod common;

use common::check;
use frontmatter_spans::{FrontMatter, Node, Value};

const CORPUS: &[&str] = &[
    // YAML: nested flow, immediately nested, with trailing comments.
    "---\na: [[1], [2]]\nb: {c: {d: 1}, e: [x, y]}\n---\n",
    "---\na: [[1],[2]] # note\nb: {c: {d: 1}} # note\n---\n",
    "---\na: [1, # inner\n  2]\nb: 3\n---\n",
    // YAML: block collections with comments and blank lines.
    "---\na:\n  - 1\n  - 2 # two\n\n  # tail\nb:\n  c: 1 # one\n  d: 2   # two\n\n# end\ne: 3\n---\n",
    "---\nlist:\n  - name: a\n    tags: [x, y]\n  - name: b # second\n    tags: []\n---\n",
    "---\n- a\n- b # c\n- [d, e]\n- {f: g}\n---\n",
    // YAML: quotes, multi-byte text, empty values.
    "---\nq: \"x # not a comment\" # comment\ns: 'it''s' # c\nu: ünï-ćödé\nv:\nw: [\"é\", 'ü']\n---\n",
    // TOML.
    "+++\ntitle = \"x\" # comment\nlist = [1, 2, [3]] # note\ninline = {a = {b = 1}, c = [1]}\n[sec]\nk = 1\nm = \"ü\"\n[[t]]\nq = 1\n[[t]]\nq = 2\n+++\n",
    "+++\na.b = 1\n'quoted key' = 2\nd = 2024-05-06T07:08:09Z\n+++\n",
    // JSON.
    "{\"a\": [1, [2, 3], {\"b\": null}], \"c\": {\"d\": \"é\"}, \"e\": 1.5e3}\n",
    "{\n  \"a\": [\n    1,\n    {\"b\": [true, false]}\n  ],\n  \"c\": \"ü\"\n}\nbody",
];

#[test]
fn every_span_is_a_clean_reparsable_fragment() {
    for doc in CORPUS {
        check(doc);
        check(&doc.replace('\n', "\r\n"));
        check(&format!("\u{feff}{doc}"));
    }
}

// ---- named regressions ----------------------------------------------------------------

fn text<'a>(doc: &'a str, node: &Node) -> &'a str {
    &doc[node.span.range.clone()]
}

#[test]
fn immediately_nested_flow_collections_end_at_their_own_delimiter() {
    let doc = "---\na: [[1],[2]]\nb: {c: {d: 1}}\n---\n";
    let fm = FrontMatter::parse(doc).unwrap().unwrap();
    let a = fm.get("a").unwrap();
    assert_eq!(text(doc, a), "[[1],[2]]");
    let Value::Array(items) = &a.value else {
        panic!()
    };
    assert_eq!(text(doc, &items[0]), "[1]");
    assert_eq!(text(doc, &items[1]), "[2]");
    let b = fm.get("b").unwrap();
    assert_eq!(text(doc, b), "{c: {d: 1}}");
    assert_eq!(text(doc, b.get("c").unwrap()), "{d: 1}");
}

#[test]
fn collection_spans_exclude_trailing_comments_and_blank_lines() {
    let cases = [
        ("a: [1, 2] # note", "a", "[1, 2]"),
        ("a: {b: 1} # note", "a", "{b: 1}"),
        ("a: [1, # inner\n  2] # outer", "a", "[1, # inner\n  2]"),
        ("a:\n  - 1\n  - 2 # c\n\n  # tail\nb: 3", "a", "- 1\n  - 2"),
        (
            "a:\n  b: 1 # c\n  c: 2   # d\n\n# x\nd: 3",
            "a",
            "b: 1 # c\n  c: 2",
        ),
    ];
    for (body, key, want) in cases {
        let doc = format!("---\n{body}\n---\n");
        let fm = FrontMatter::parse(&doc).unwrap().unwrap();
        assert_eq!(text(&doc, fm.get(key).unwrap()), want, "{body:?}");
    }
}

#[test]
fn the_root_mapping_ends_at_its_last_value() {
    let doc = "---\na: 1\nb: [2] # c\n\n# tail\n---\n";
    let fm = FrontMatter::parse(doc).unwrap().unwrap();
    assert_eq!(text(doc, &fm.root), "a: 1\nb: [2]");
}

#[test]
fn a_bare_alias_error_covers_the_whole_alias_token() {
    let cases = [
        ("b: *x", "*x"),
        ("b: *long-name # c", "*long-name"),
        ("b: [*x, 2]", "*x"),
        ("b: {c: *é1}", "*é1"),
        ("- *list_1\n- 2", "*list_1"),
    ];
    for (body, token) in cases {
        let doc = format!("---\n{body}\n---\n");
        let err = FrontMatter::parse(&doc).unwrap_err();
        let span = err.span.clone().expect("span");
        assert_eq!(&doc[span.range], token, "{body:?}: {err}");
    }
}

#[test]
fn flow_collections_with_a_trailing_comma_keep_their_closing_delimiter() {
    let cases = [
        ("a: {b: 1,}", "{b: 1,}"),
        ("a: [1,]", "[1,]"),
        ("a: [1, 2 , ]", "[1, 2 , ]"),
        ("a: {\n  b: 1,\n}", "{\n  b: 1,\n}"),
        ("a: [1, # c\n  2, # d\n] # e", "[1, # c\n  2, # d\n]"),
        ("a: [[1,],]", "[[1,],]"),
    ];
    for (body, want) in cases {
        let doc = format!("---\n{body}\n---\n");
        let fm = FrontMatter::parse(&doc).unwrap().unwrap();
        assert_eq!(text(&doc, fm.get("a").unwrap()), want, "{body:?}");
    }
}

#[test]
fn an_empty_value_is_zero_width_right_after_its_colon_or_dash() {
    let cases = [
        ("a:", "a", 2),
        ("a: # note\nb: 1", "a", 2),
        ("a:\n  draft:", "a", 2),
    ];
    for (body, key, colon) in cases {
        let doc = format!("---\n{body}\n---\n");
        let fm = FrontMatter::parse(&doc).unwrap().unwrap();
        let node = fm.get(key).unwrap();
        let at = 4 + colon - 1 + 1; // after `---\n` and the one-byte key
        if matches!(node.value, Value::Null) {
            assert_eq!(node.span.range, at..at, "{body:?}");
        }
    }
    let doc = "---\n- a\n-\n- b\n---\n";
    let fm = FrontMatter::parse(doc).unwrap().unwrap();
    let Value::Array(items) = &fm.root.value else {
        panic!()
    };
    assert_eq!(items[1].span.range, 9..9);
    assert_eq!(&doc[..9], "---\n- a\n-");
}

#[test]
fn block_collections_with_an_empty_final_value_keep_the_colon_or_dash() {
    let doc = "---\na:\n  draft: # note\n\n# tail\n---\n";
    let fm = FrontMatter::parse(doc).unwrap().unwrap();
    assert_eq!(text(doc, fm.get("a").unwrap()), "draft:");
    assert_eq!(text(doc, &fm.root), "a:\n  draft:");

    let doc = "---\nx: 1\ndraft:\n---\n";
    let fm = FrontMatter::parse(doc).unwrap().unwrap();
    assert_eq!(text(doc, &fm.root), "x: 1\ndraft:");

    let doc = "---\n- a\n- # n\n---\n";
    let fm = FrontMatter::parse(doc).unwrap().unwrap();
    assert_eq!(text(doc, &fm.root), "- a\n-");
}
