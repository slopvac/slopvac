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
use toml_edit::{ArrayOfTables, Document, Item, Key, TableLike, Value};

use crate::Dropped;
use crate::messages::MigrateText;

/// Most levels of tables and arrays between the root and any value.
pub(crate) const MAX_DEPTH: usize = 32;

/// A document that nests deeper than [`MAX_DEPTH`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TooDeep {
    /// One-based line of the first table or array past the bound.
    pub(crate) line: usize,
}

/// The key a record names when the refusal is about the document as a whole.
pub(crate) const TABLES_KEY: &str = "tables";

impl TooDeep {
    /// The refusal as a blocking record: `config-legacy-dropped`, exit 2. The content parses
    /// but migrate cannot carry it over, so it is not a syntax failure. `key` is the key the
    /// record names.
    pub(crate) fn dropped(self, file: &str, key: &str) -> Dropped {
        Dropped::new(
            file,
            key,
            self.line,
            MigrateText::NestingTooDeep { limit: MAX_DEPTH }.to_string(),
        )
    }
}

enum Node<'a> {
    Table(&'a dyn TableLike),
    Tables(&'a ArrayOfTables),
    Value(&'a Value),
}

/// Stack for the thread that parses, checks and drops a document in [`check`].
///
/// The parser takes about 80 dotted parts in each of about 80 nested inline tables, so a
/// tree it accepts can nest some 6,500 levels. Building the tree and dropping it both
/// recurse once per level, and an unoptimized build spends a few hundred bytes a level, more
/// than a default 2 MiB thread stack holds. 16 MiB leaves several times that worst case.
/// The pages are reserved, not committed, until the recursion reaches them.
const CHECK_STACK_BYTES: usize = 16 << 20;

/// Parse `text` and check its nesting. Text that is not valid TOML passes: the caller's own
/// parse reports it.
///
/// The parse, the check and the drop of the parsed tree run on a thread of their own with
/// [`CHECK_STACK_BYTES`] of stack, so a refused document cannot overflow the caller's stack.
pub(crate) fn check(text: &str) -> Result<(), TooDeep> {
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("slopvac-migrate-nesting".to_owned())
            .stack_size(CHECK_STACK_BYTES)
            .spawn_scoped(scope, || match Document::parse(text) {
                Ok(doc) => check_document(&doc, text),
                Err(_) => Ok(()),
            });
        match worker {
            Ok(handle) => handle
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
            // No thread, no check: refuse rather than walk an unchecked document.
            Err(_) => Err(TooDeep { line: 1 }),
        }
    })
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
                        Item::ArrayOfTables(aot) => push(Node::Tables(aot), key_at)?,
                        Item::Value(v) => descend(v, key_at, &mut push)?,
                        Item::None => {}
                    }
                }
            }
            Node::Tables(aot) => {
                for table in aot {
                    push(Node::Table(table), table.span().map(|s| s.start))?;
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
    use rstest::rstest;

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

    /// `[[a.a…]]` with `levels - 1` parts: the tables, the array, and its element.
    fn nested_array_of_tables(levels: usize) -> String {
        format!("x = 1\n[[{}]]\n", vec!["a"; levels - 1].join("."))
    }

    /// An array of tables whose element holds alternating inline arrays and inline tables.
    fn mixed(levels: usize) -> String {
        let rest = levels - 2;
        let (pairs, odd) = (rest / 2, rest % 2 == 1);
        format!(
            "x = 1\n[[a]]\nb = {}{}1{}{}\n",
            "[{c = ".repeat(pairs),
            if odd { "[" } else { "" },
            if odd { "]" } else { "" },
            "}]".repeat(pairs)
        )
    }

    // Each shape at exactly MAX_DEPTH levels passes, and one more is refused.
    // Levels count tables and arrays between the root and the value: `[a.a]` and `a.a.a = 1`
    // are two, `a = {a = 1}` is one, `a = [[1]]` is two, and `[[a]]` is two (the array and
    // its element).
    #[rstest]
    #[case::headers(nested_headers, 2)]
    #[case::dotted_keys(nested_dotted, 2)]
    #[case::inline_tables(nested_inline, 2)]
    #[case::arrays(nested_arrays, 2)]
    #[case::arrays_of_tables(nested_array_of_tables, 2)]
    #[case::arrays_and_tables_mixed(mixed, 3)]
    fn a_shape_is_refused_one_level_past_the_bound(
        #[case] shape: fn(usize) -> String,
        #[case] line: usize,
    ) {
        assert_eq!(check(&shape(MAX_DEPTH)), Ok(()), "at the bound");
        assert_eq!(
            check(&shape(MAX_DEPTH + 1)),
            Err(TooDeep { line }),
            "past the bound"
        );
    }

    #[test]
    fn text_that_is_not_toml_passes_for_the_parser_to_report() {
        assert_eq!(check("a = = 1"), Ok(()));
    }
}
