//! Span invariants over a corpus: every node and key slice is a clean, re-parsable fragment.

use frontmatter_spans::{Format, FrontMatter, Node, Value};

/// A canonical, span-free rendering of a value, for comparing a node to its re-parse.
fn shape(node: &Node) -> String {
    match &node.value {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Float(f) => format!("{f:?}"),
        Value::String(s) | Value::Datetime(s) => format!("{s:?}"),
        Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(shape).collect();
            format!("[{}]", inner.join(","))
        }
        Value::Table(entries) => {
            let inner: Vec<String> = entries
                .iter()
                .map(|e| format!("{:?}:{}", e.key, shape(&e.value)))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
    }
}

struct Item<'a> {
    node: &'a Node,
}

fn nodes<'a>(
    node: &'a Node,
    out: &mut Vec<Item<'a>>,
    keys: &mut Vec<(&'a str, std::ops::Range<usize>, usize)>,
) {
    out.push(Item { node });
    match &node.value {
        Value::Table(entries) => {
            for e in entries {
                keys.push((&e.key, e.key_span.range.clone(), e.key_span.start.column));
                nodes(&e.value, out, keys);
            }
        }
        Value::Array(items) => items.iter().for_each(|n| nodes(n, out, keys)),
        _ => {}
    }
}

fn parse_fragment(format: Format, fragment: &str) -> Option<FrontMatter<'static>> {
    let doc: &'static str = Box::leak(
        match format {
            Format::Yaml => format!("---\n{fragment}\n---\n"),
            Format::Toml => format!("+++\nv = {fragment}\n+++\n"),
            Format::Json => format!("{{\"v\": {fragment}}}"),
        }
        .into_boxed_str(),
    );
    FrontMatter::parse(doc).ok().flatten()
}

fn check(doc: &str) {
    let fm = FrontMatter::parse(doc)
        .unwrap_or_else(|e| panic!("{doc:?}: {e}"))
        .unwrap_or_else(|| panic!("{doc:?}: no block"));
    let format = fm.block.format;
    let mut all = Vec::new();
    let mut keys = Vec::new();
    nodes(&fm.root, &mut all, &mut keys);

    for Item { node } in all.iter().skip(1) {
        let slice = &doc[node.span.range.clone()];
        if slice.is_empty() {
            continue;
        }
        assert_eq!(slice, slice.trim(), "padding in {slice:?} of {doc:?}");
        // TOML tables written with headers or dotted keys are not fragments.
        let toml_table_form = format == Format::Toml
            && match &node.value {
                Value::Table(_) => !slice.starts_with('{'),
                Value::Array(items) => {
                    slice.starts_with("[[")
                        && matches!(items.first().map(|n| &n.value), Some(Value::Table(_)))
                }
                _ => false,
            };
        if toml_table_form {
            continue;
        }
        let quoted = slice.starts_with(['"', '\'']);
        if format == Format::Yaml && !quoted {
            let last = slice.rsplit(['\n', '\r']).next().unwrap();
            assert!(!last.contains('#'), "comment text in {slice:?} of {doc:?}");
        }
        let fragment = if format == Format::Yaml {
            // Block collections continue at the column where they began.
            format!("{}{slice}", " ".repeat(node.span.start.column - 1))
        } else {
            slice.to_owned()
        };
        let again = parse_fragment(format, &fragment)
            .unwrap_or_else(|| panic!("{slice:?} of {doc:?} does not re-parse"));
        let root = &again.root;
        let got = match format {
            Format::Yaml => root,
            _ => root.get("v").expect("v"),
        };
        assert_eq!(shape(got), shape(node), "{slice:?} of {doc:?}");
    }

    for (key, range, _) in keys {
        let slice = &doc[range];
        assert_eq!(slice, slice.trim(), "padding in key {slice:?}");
        let fragment = match format {
            Format::Yaml => format!("{slice}: 1"),
            Format::Toml => format!("{slice} = 1"),
            Format::Json => format!("{{{slice}: 1}}"),
        };
        let wrapped = match format {
            Format::Yaml => format!("---\n{fragment}\n---\n"),
            Format::Toml => format!("+++\n{fragment}\n+++\n"),
            Format::Json => fragment,
        };
        // Dotted TOML keys (`a.b`) are paths, not single keys.
        if format == Format::Toml && slice.contains('.') && !slice.starts_with(['"', '\'']) {
            continue;
        }
        let again = FrontMatter::parse(&wrapped)
            .unwrap_or_else(|e| panic!("key {slice:?} of {doc:?}: {e}"))
            .unwrap();
        let Value::Table(entries) = &again.root.value else {
            panic!("not a table");
        };
        assert_eq!(entries[0].key, key, "key {slice:?} of {doc:?}");
    }
}

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
