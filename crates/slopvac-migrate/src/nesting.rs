//! The nesting bound on the TOML that migrate walks.
//!
//! `toml_edit` limits each construct on its own: an inline table or array may nest 79 deep,
//! and one dotted key or `[a.b.c]` header may name 79 tables. The limits multiply. A key
//! `a.b.c…` with 70 parts inside an inline table inside another one, and so on, builds
//! thousands of nested tables from well under [`slopvac_config::MAX_CONFIG_BYTES`], and the
//! recursive walks in this crate and in `slopvac-config` would overflow the stack on them.
//!
//! [`check`] counts every table and array on the way from the root to a value, whatever
//! construct made it, and refuses a document that nests more than [`MAX_DEPTH`] levels. It
//! walks with an explicit stack, so the check itself uses constant stack. Run it before any
//! recursive walk of a document.
//!
//! # Bound
//!
//! After the check, every recursive walk is at most [`MAX_DEPTH`] calls deep. A real config
//! nests under ten levels (`overrides[0].categories.x.rules.y`), and `extends` chains are
//! bounded at the same number by the config loader.

use frontmatter_spans::LineIndex;
use toml_edit::{Document, Item, Key, TableLike, Value};

/// Most levels of tables and arrays between the root and any value.
pub(crate) const MAX_DEPTH: usize = 32;

/// A document that nests deeper than [`MAX_DEPTH`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TooDeep {
    /// One-based line of the first table or array past the bound.
    pub(crate) line: usize,
}

impl TooDeep {
    /// The message for a diagnostic, without the file and line.
    pub(crate) fn message(self) -> String {
        format!("tables nest more than {MAX_DEPTH} levels deep")
    }
}

enum Node<'a> {
    Table(&'a dyn TableLike),
    Value(&'a Value),
}

/// Parse `text` and check its nesting. Text that is not valid TOML passes: the caller's own
/// parse reports it.
pub(crate) fn check(text: &str) -> Result<(), TooDeep> {
    match Document::parse(text) {
        Ok(doc) => check_document(&doc, text),
        Err(_) => Ok(()),
    }
}

/// Check the nesting of a document parsed from `text`.
pub(crate) fn check_document<S>(doc: &Document<S>, text: &str) -> Result<(), TooDeep> {
    let mut stack = vec![(Node::Table(doc.as_table()), 0usize, 0usize)];
    while let Some((node, depth, at)) = stack.pop() {
        let mut push = |child, span_start: Option<usize>| {
            let (child_depth, child_at) = (depth + 1, span_start.unwrap_or(at));
            if child_depth > MAX_DEPTH {
                return Err(TooDeep {
                    line: LineIndex::new(text).line_col(child_at).line,
                });
            }
            stack.push((child, child_depth, child_at));
            Ok(())
        };
        match node {
            Node::Table(t) => {
                for (name, item) in t.iter() {
                    let key_at = t.key(name).and_then(Key::span).map(|s| s.start);
                    match item {
                        Item::Table(sub) => push(Node::Table(sub), key_at)?,
                        Item::ArrayOfTables(aot) => {
                            for table in aot {
                                push(Node::Table(table), table.span().map(|s| s.start))?;
                            }
                        }
                        Item::Value(v) => descend(v, key_at, &mut push)?,
                        Item::None => {}
                    }
                }
            }
            Node::Value(Value::Array(arr)) => {
                for v in arr {
                    descend(v, v.span().map(|s| s.start), &mut push)?;
                }
            }
            Node::Value(Value::InlineTable(t)) => {
                for (name, v) in t {
                    let key_at = t.get_key_value(name).and_then(|(k, _)| k.span());
                    descend(v, key_at.map(|s| s.start), &mut push)?;
                }
            }
            Node::Value(_) => {}
        }
    }
    Ok(())
}

/// Queue `v` when it holds more values; scalars end a path.
fn descend<'a>(
    v: &'a Value,
    at: Option<usize>,
    push: &mut impl FnMut(Node<'a>, Option<usize>) -> Result<(), TooDeep>,
) -> Result<(), TooDeep> {
    match v {
        Value::Array(_) | Value::InlineTable(_) => push(Node::Value(v), at),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested_headers(levels: usize) -> String {
        format!("x = 1\n[{}]\n", vec!["a"; levels].join("."))
    }

    fn nested_dotted(levels: usize) -> String {
        format!("x = 1\n{} = 1\n", vec!["a"; levels + 1].join("."))
    }

    fn nested_inline(levels: usize) -> String {
        format!(
            "x = 1\na = {}1{}\n",
            "{a = ".repeat(levels),
            "}".repeat(levels)
        )
    }

    fn nested_arrays(levels: usize) -> String {
        format!("x = 1\na = {}1{}\n", "[".repeat(levels), "]".repeat(levels))
    }

    // Each shape at exactly MAX_DEPTH levels passes, and one more is refused on line 2.
    // Levels count tables and arrays between the root and the value: `[a.a]` and `a.a.a = 1`
    // are two, `a = {a = 1}` is one, and `a = [[1]]` is two.
    #[test]
    fn each_shape_is_refused_one_level_past_the_bound() {
        type Shape = fn(usize) -> String;
        let shapes: [(&str, Shape); 4] = [
            ("headers", nested_headers),
            ("dotted keys", nested_dotted),
            ("inline tables", nested_inline),
            ("arrays", nested_arrays),
        ];
        for (name, shape) in shapes {
            assert_eq!(check(&shape(MAX_DEPTH)), Ok(()), "{name} at the bound");
            assert_eq!(
                check(&shape(MAX_DEPTH + 1)),
                Err(TooDeep { line: 2 }),
                "{name} past the bound"
            );
        }
    }

    #[test]
    fn text_that_is_not_toml_passes_for_the_parser_to_report() {
        assert_eq!(check("a = = 1"), Ok(()));
    }
}
