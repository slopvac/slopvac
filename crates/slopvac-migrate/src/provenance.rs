//! The line of every key in a TOML text, behind the lines that migrate's diagnostics report.
//!
//! A diagnostic names a key by its path: `a.b` for key `b` in table `a`, and `a[2]` for the
//! third table of an array, with no quoting. [`KeyLines`] finds the line of such a path
//! without storing the path. It cuts every path before each `.` and `[` and keeps the pieces
//! in a trie, so a table name shared by many keys is stored once, however long it is.
//!
//! # Memory bound
//!
//! For a text of `n` bytes, a [`KeyLines`] stores at most `n` bytes of key names and at most
//! `n + 1` trie nodes. Each node is one fixed-size line slot and one fixed-size map entry. A
//! config is at most [`slopvac_config::MAX_CONFIG_BYTES`] long, so its map holds at most that
//! many name bytes and one node more than that. The bound holds because every key and every
//! table of an array takes bytes of the text of its own: a key cuts into no more pieces than
//! its text has bytes, and its stored pieces are no longer than its decoded name, which is no
//! longer than its text.

use std::collections::HashMap;

use frontmatter_spans::LineIndex;
use toml_edit::{Document, Item, Key, TableLike, Value};

use crate::messages::MigrateText;
use crate::nesting;

/// The node of the empty path.
const ROOT: usize = 0;

/// One cut of a key path: the text up to the next `.` or `[`, tagged with the separator
/// before it, so the separator itself is not stored.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Piece {
    /// The start of a path, with no separator before it.
    Start(Box<str>),
    /// Text after a `.`.
    Dot(Box<str>),
    /// Text after a `[` that does not form an array index.
    Bracket(Box<str>),
    /// `[n]`, with `n` written in decimal without leading zeros.
    Index(usize),
}

impl Piece {
    /// The bytes of key name this piece stores.
    #[cfg(test)]
    fn name_bytes(&self) -> usize {
        match self {
            Self::Start(s) | Self::Dot(s) | Self::Bracket(s) => s.len(),
            Self::Index(_) => 0,
        }
    }
}

/// The index in `[n]`, given the text after the `[`. `None` unless the text is the decimal
/// form of a `usize` with no leading zero, then `]`, which is how a path writes an index.
fn index(body: &str) -> Option<usize> {
    let digits = body.strip_suffix(']')?;
    let canonical = !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'));
    canonical.then(|| digits.parse().ok()).flatten()
}

/// The pieces of `path`, in order. Equal paths give equal pieces, and the pieces of `a` then
/// those of `b` are the pieces of `a + b` whenever `b` starts with `.` or `[`.
fn pieces(path: &str) -> impl Iterator<Item = Piece> + '_ {
    let mut rest = path;
    std::iter::from_fn(move || {
        let (separator, body) = match rest.chars().next()? {
            sep @ ('.' | '[') => (Some(sep), rest.get(1..).unwrap_or_default()),
            _ => (None, rest),
        };
        let end = body.find(['.', '[']).unwrap_or(body.len());
        let (text, tail) = body.split_at(end);
        rest = tail;
        Some(match separator {
            None => Piece::Start(text.into()),
            Some('.') => Piece::Dot(text.into()),
            _ => index(text).map_or_else(|| Piece::Bracket(text.into()), Piece::Index),
        })
    })
}

/// The one-based line of every key path in a TOML text. Array-of-tables elements use
/// `name[index]`. When two keys have the same path, the later one in the walk wins.
#[derive(Debug)]
pub(crate) struct KeyLines {
    /// The line of each node, by node number. A node with no key of its own has none.
    lines: Vec<Option<usize>>,
    /// The child of a node along a piece.
    children: HashMap<(usize, Piece), usize>,
}

impl Default for KeyLines {
    fn default() -> Self {
        Self {
            lines: vec![None],
            children: HashMap::new(),
        }
    }
}

impl KeyLines {
    /// The line of the key at `path`, if the text has one.
    pub(crate) fn line(&self, path: &str) -> Option<usize> {
        let mut at = ROOT;
        for piece in pieces(path) {
            at = *self.children.get(&(at, piece))?;
        }
        self.lines.get(at).copied().flatten()
    }

    /// The child of `at` along `piece`, added if it is new.
    fn child(&mut self, at: usize, piece: Piece) -> usize {
        let lines = &mut self.lines;
        *self.children.entry((at, piece)).or_insert_with(|| {
            lines.push(None);
            lines.len() - 1
        })
    }

    /// The node of the path of `at` followed by `segment`.
    fn descend(&mut self, at: usize, segment: &str) -> usize {
        pieces(segment).fold(at, |node, piece| self.child(node, piece))
    }

    fn set(&mut self, at: usize, offset: usize, index: &LineIndex<'_>) {
        if let Some(slot) = self.lines.get_mut(at) {
            *slot = Some(index.line_col(offset).line);
        }
    }

    /// Record every key below `t`, whose path is the path of `at`.
    fn walk(&mut self, t: &dyn TableLike, at: usize, index: &LineIndex<'_>) {
        for (name, item) in t.iter() {
            // Only the empty path has the root node, and a key below it takes no `.`.
            let node = if at == ROOT {
                self.descend(ROOT, name)
            } else {
                self.descend(at, &format!(".{name}"))
            };
            if let Some(span) = t.key(name).and_then(Key::span) {
                self.set(node, span.start, index);
            }
            match item {
                Item::ArrayOfTables(aot) => {
                    for (i, table) in aot.iter().enumerate() {
                        let element = self.child(node, Piece::Index(i));
                        if let Some(span) = table.span() {
                            self.set(element, span.start, index);
                        }
                        self.walk(table, element, index);
                    }
                }
                Item::Value(Value::Array(arr)) => {
                    for (i, v) in arr.iter().enumerate() {
                        if let Some(it) = v.as_inline_table() {
                            let element = self.child(node, Piece::Index(i));
                            if let Some(span) = v.span() {
                                self.set(element, span.start, index);
                            }
                            self.walk(it, element, index);
                        }
                    }
                }
                other => {
                    if let Some(tl) = other.as_table_like() {
                        self.walk(tl, node, index);
                    }
                }
            }
        }
    }

    /// What this map holds, for the bound in the module docs.
    #[cfg(test)]
    fn held(&self) -> Held {
        Held {
            name_bytes: self.children.keys().map(|(_, p)| p.name_bytes()).sum(),
            records: self.lines.len(),
        }
    }
}

/// Map every key path in `text` to its one-based line.
///
/// # Errors
/// The parser's message when `text` is not valid TOML.
pub(crate) fn key_lines(text: &str) -> Result<KeyLines, String> {
    let doc = Document::parse(text.to_owned()).map_err(|e| e.to_string())?;
    nesting::check_document(&doc, text).map_err(|e| {
        format!(
            "line {}: {}",
            e.line,
            MigrateText::NestingTooDeep {
                limit: nesting::MAX_DEPTH
            }
        )
    })?;
    let index = LineIndex::new(text);
    let mut out = KeyLines::default();
    out.walk(doc.as_table(), ROOT, &index);
    #[cfg(test)]
    record(out.held());
    Ok(out)
}

/// What one provenance map holds: stored key-name bytes and trie nodes.
#[cfg(test)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Held {
    name_bytes: usize,
    records: usize,
}

#[cfg(test)]
thread_local! {
    /// The largest of each measure over the maps this thread built.
    static LARGEST: std::cell::Cell<Held> = const {
        std::cell::Cell::new(Held { name_bytes: 0, records: 0 })
    };
}

#[cfg(test)]
fn record(held: Held) {
    LARGEST.with(|l| {
        let was = l.get();
        l.set(Held {
            name_bytes: was.name_bytes.max(held.name_bytes),
            records: was.records.max(held.records),
        });
    });
}

#[cfg(test)]
fn largest_built() -> Held {
    LARGEST.with(std::cell::Cell::get)
}

#[cfg(test)]
mod tests {
    use slopvac_config::MAX_CONFIG_BYTES;

    use super::largest_built;
    use crate::{Error, Options, RenameTable, migrate_config};

    /// A config just under the cap: one table name of seven eighths of it, then as many short
    /// keys as fit.
    fn long_table_with_many_keys() -> String {
        let cap = usize::try_from(MAX_CONFIG_BYTES).unwrap();
        let mut text = format!("[{}]\n", "a".repeat(cap / 8 * 7));
        for i in 0.. {
            let line = format!("k{i} = 1\n");
            if text.len() + line.len() > cap {
                break;
            }
            text.push_str(&line);
        }
        text
    }

    #[test]
    fn a_long_table_name_shared_by_many_keys_is_stored_once() {
        let text = long_table_with_many_keys();
        let result = migrate_config(
            &text,
            "slopvac.toml",
            &RenameTable::bundled(),
            Options::default(),
        );
        assert!(
            matches!(result, Err(Error::Dropped(_))),
            "the unknown table should block the run"
        );
        let held = largest_built();
        assert!(
            held.name_bytes <= text.len(),
            "{held:?} for {} bytes",
            text.len()
        );
        assert!(
            held.records <= text.len() + 1,
            "{held:?} for {} bytes",
            text.len()
        );
    }
}
