//! Shared span invariants: every node and key slice is a clean, re-parsable fragment.

#![allow(dead_code)]

use frontmatter_spans::{Format, FrontMatter, Node, Value};

/// A canonical, span-free rendering of a value, for comparing a node to its re-parse.
pub(crate) fn shape(node: &Node) -> String {
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Parent {
    Root,
    MapValue,
    SeqItem,
}

struct Visited<'a> {
    node: &'a Node,
    parent: Parent,
}

fn walk<'a>(
    node: &'a Node,
    parent: Parent,
    out: &mut Vec<Visited<'a>>,
    keys: &mut Vec<(&'a str, std::ops::Range<usize>)>,
) {
    out.push(Visited { node, parent });
    match &node.value {
        Value::Table(entries) => {
            for e in entries {
                keys.push((&e.key, e.key_span.range.clone()));
                walk(&e.value, Parent::MapValue, out, keys);
            }
        }
        Value::Array(items) => items
            .iter()
            .for_each(|n| walk(n, Parent::SeqItem, out, keys)),
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

/// Whether `line` carries comment text: a `#` outside quotes at the start or after a blank.
fn has_comment(line: &str) -> bool {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'\'' => {
                i += 1;
                while i < b.len() {
                    if b[i] == b'\'' {
                        if b.get(i + 1) == Some(&b'\'') {
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    i += 1;
                }
            }
            b'#' if i == 0 || b[i - 1] == b' ' || b[i - 1] == b'\t' => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

/// Parse `doc` and assert the span invariants for every node and key. Returns the root shape.
pub(crate) fn check(doc: &str) -> String {
    let fm = FrontMatter::parse(doc)
        .unwrap_or_else(|e| panic!("{doc:?}: {e}"))
        .unwrap_or_else(|| panic!("{doc:?}: no block"));
    let format = fm.block.format;
    let mut all = Vec::new();
    let mut keys = Vec::new();
    walk(&fm.root, Parent::Root, &mut all, &mut keys);

    for Visited { node, parent } in all.iter().skip(1) {
        let slice = &doc[node.span.range.clone()];
        if slice.is_empty() {
            // An empty value is zero-width, right after the `:` or `-` that introduces it.
            if format == Format::Yaml {
                let at = node.span.range.start;
                let want = match parent {
                    Parent::MapValue => b':',
                    Parent::SeqItem => b'-',
                    Parent::Root => unreachable!(),
                };
                assert!(
                    at > 0 && doc.as_bytes()[at - 1] == want,
                    "empty value at {at} is not right after `{}` in {doc:?}",
                    want as char
                );
            }
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
        if format == Format::Yaml {
            let last = slice.rsplit(['\n', '\r']).next().unwrap();
            assert!(!has_comment(last), "comment text in {slice:?} of {doc:?}");
        }
        let fragment = if format == Format::Yaml {
            // Block collections continue at the column where they began.
            format!("{}{slice}", " ".repeat(node.span.start.column - 1))
        } else {
            slice.to_owned()
        };
        let again = parse_fragment(format, &fragment)
            .unwrap_or_else(|| panic!("{slice:?} of {doc:?} does not re-parse"));
        let got = match format {
            Format::Yaml => &again.root,
            _ => again.root.get("v").expect("v"),
        };
        assert_eq!(shape(got), shape(node), "{slice:?} of {doc:?}");
    }

    for (key, range) in keys {
        let slice = &doc[range];
        assert_eq!(slice, slice.trim(), "padding in key {slice:?}");
        // Dotted TOML keys (`a.b`) are paths, not single keys.
        if format == Format::Toml && slice.contains('.') && !slice.starts_with(['"', '\'']) {
            continue;
        }
        let wrapped = match format {
            Format::Yaml => format!("---\n{slice}: 1\n---\n"),
            Format::Toml => format!("+++\n{slice} = 1\n+++\n"),
            Format::Json => format!("{{{slice}: 1}}"),
        };
        let again = FrontMatter::parse(&wrapped)
            .unwrap_or_else(|e| panic!("key {slice:?} of {doc:?}: {e}"))
            .unwrap();
        let Value::Table(entries) = &again.root.value else {
            panic!("not a table");
        };
        assert_eq!(entries[0].key, key, "key {slice:?} of {doc:?}");
    }
    shape(&fm.root)
}
