//! Inline directives: `slopvac: key=value` comments for documents.
//!
//! A directive sets the same fields as the front-matter block, so formats without front matter
//! (reStructuredText, plain text, code) can carry per-file settings. The same `[frontmatter]
//! allow` list decides which fields take effect.
//!
//! | Document | Form |
//! |---|---|
//! | Markdown, HTML | `<!-- slopvac: genre=how-to locale=en-GB -->` |
//! | reStructuredText | `.. slopvac: genre=how-to` |
//! | Plain text and code | `# slopvac: ...`, `// slopvac: ...`, `-- slopvac: ...`, `; slopvac: ...` or `/* slopvac: ... */` |
//!
//! A dotted key sets a nested field: `thresholds.min_score=60`, `rules.core:no-dash=off`.
//! `terms.allow` takes a comma-separated list.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

use crate::comments::{Family, family_of, family_of_name, scan};
use crate::diag::{Code, Diagnostic};
use crate::flatten::{Origin, flatten_with};
use crate::frontmatter::{Policy, Read, denied};
use crate::messages::{ConfigPhrase, ConfigText};
use crate::schema::validate_directive;
use crate::tree::{Entry, LineIndex, Node, Value};
use slopvac_model::ConfigLayer;

/// One directive comment: where it is and the fields it sets.
#[derive(Debug, Clone, PartialEq)]
pub struct Directive {
    /// Byte range of the text after `slopvac:`.
    pub payload: Range<usize>,
    /// The fields as a table. Every span indexes the document.
    pub node: Node,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    /// Source code whose comment syntax is known, so strings can be told from comments.
    Code(Family),
    /// Text with no known comment syntax: a marker at the start of a line counts.
    Plain,
    Html,
    Rst,
}

fn style_of(file: &str) -> Style {
    let path = Path::new(file);
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("md" | "markdown" | "mdx" | "html" | "htm") => Style::Html,
        Some("rst") => Style::Rst,
        Some(other) => family_of(other).map_or(Style::Plain, Style::Code),
        None => path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(family_of_name)
            .map_or(Style::Plain, Style::Code),
    }
}

/// Find the directives in `doc`. `file` picks the comment style by its extension.
///
/// Only real comments count. Lines inside a Markdown or HTML code block, a reStructuredText
/// literal block, or a multi-line string in code are examples or data, not directives.
#[must_use]
pub fn find(doc: &str, file: &str) -> Vec<Directive> {
    let mut unlimited = Budget::unlimited();
    let Ok(payloads) = locate(doc, file, &mut unlimited) else {
        return Vec::new();
    };
    payloads
        .into_iter()
        .filter_map(|payload| {
            let node = parse_pairs(doc, payload.clone(), &mut unlimited).ok()?;
            Some(Directive { payload, node })
        })
        .collect()
}

/// The work a document may cost, charged as it is scanned: one unit for each directive, each
/// pair, each dotted segment and each list item, before the thing is collected or allocated.
/// The refusal comes at the first unit past the cap, so the work and the memory of a refused
/// document stay linear in its size, and a hostile one costs no more than the cap.
struct Budget {
    cap: usize,
    spent: usize,
}

/// The document passed its budget, or nests a key deeper than [`DIRECTIVE_MAX_DEPTH`].
struct Over;

impl Budget {
    fn for_document(doc: &str) -> Self {
        Self {
            cap: directive_cap(doc),
            spent: 0,
        }
    }

    const fn unlimited() -> Self {
        Self {
            cap: usize::MAX,
            spent: 0,
        }
    }

    /// Charge `units`; `Err` when that passes the cap.
    fn charge(&mut self, units: usize) -> Result<(), Over> {
        self.spent = self.spent.saturating_add(units);
        if self.spent > self.cap {
            Err(Over)
        } else {
            Ok(())
        }
    }
}

/// The payload range of every directive in `doc`, found without reading any key. Each is
/// charged to `budget` before it is kept.
fn locate(doc: &str, file: &str, budget: &mut Budget) -> Result<Vec<Range<usize>>, Over> {
    let style = style_of(file);
    if let Style::Code(family) = style {
        return locate_in_code(doc, family, budget);
    }
    let mut literal = Literals::new(style);
    let mut out = Vec::new();
    let mut offset = 0;
    for line in doc.split_inclusive('\n') {
        let in_literal = literal.step(line);
        if !in_literal && let Some(payload) = payload_of(line, style) {
            budget.charge(1)?;
            out.push(offset + payload.start..offset + payload.end);
        }
        offset += line.len();
    }
    Ok(out)
}

/// Directives in the comments of source code, found with a scanner that knows the language's
/// strings. A comment counts when it is the first thing on its line. The scanner hands over
/// one comment at a time and keeps none, so a source file with many ordinary comments costs
/// nothing here.
fn locate_in_code(
    doc: &str,
    family: Family,
    budget: &mut Budget,
) -> Result<Vec<Range<usize>>, Over> {
    let mut out = Vec::new();
    let mut over = false;
    scan(doc, family, |c| {
        let line_start = doc.as_bytes()[..c.marker]
            .iter()
            .rposition(|&b| b == b'\n' || b == b'\r')
            .map_or(0, |p| p + 1);
        if !doc[line_start..c.marker].trim().is_empty() {
            return true;
        }
        let body = &doc[c.body.clone()];
        let text = body.trim_start();
        let Some(rest) = text.strip_prefix("slopvac:") else {
            return true;
        };
        if is_steering_marker(rest) {
            return true;
        }
        if budget.charge(1).is_err() {
            over = true;
            return false;
        }
        let start = c.body.start + (body.len() - rest.len());
        let end = (c.body.start + body.trim_end().len()).max(start);
        out.push(start..end);
        true
    });
    if over { Err(Over) } else { Ok(out) }
}

/// Tracks, line by line, whether the text is inside a code block or string.
struct Literals {
    style: Style,
    fence: Option<(char, usize)>,
    raw_tag: Option<&'static str>,
    rst_pending: bool,
    rst_active: bool,
}

const RAW_TAGS: &[&str] = &["pre", "script", "style", "code", "textarea"];
const RST_LITERAL_DIRECTIVES: &[&str] = &[
    ".. code-block::",
    ".. code::",
    ".. sourcecode::",
    ".. parsed-literal::",
];

impl Literals {
    fn new(style: Style) -> Self {
        Self {
            style,
            fence: None,
            raw_tag: None,
            rst_pending: false,
            rst_active: false,
        }
    }

    /// Feed the next line; true when that line is inside a literal context.
    fn step(&mut self, line: &str) -> bool {
        match self.style {
            Style::Html => self.step_markup(line),
            Style::Rst => self.step_rst(line),
            Style::Code(_) | Style::Plain => false,
        }
    }

    fn step_markup(&mut self, line: &str) -> bool {
        let indent = line.len() - line.trim_start_matches(' ').len();
        let body = line.trim_start_matches(' ');
        if let Some((ch, len)) = self.fence {
            let run = body.chars().take_while(|c| *c == ch).count();
            if run >= len && body[run..].trim().is_empty() {
                self.fence = None;
            }
            return true;
        }
        if indent <= 3
            && let Some(ch) = body.chars().next().filter(|c| *c == '`' || *c == '~')
        {
            let run = body.chars().take_while(|c| *c == ch).count();
            if run >= 3 && !(ch == '`' && body[run..].contains('`')) {
                self.fence = Some((ch, run));
                return true;
            }
        }
        if let Some(tag) = self.raw_tag {
            if line.to_ascii_lowercase().contains(&format!("</{tag}")) {
                self.raw_tag = None;
            }
            return true;
        }
        let lower = line.to_ascii_lowercase();
        for tag in RAW_TAGS {
            if let Some(at) = lower.find(&format!("<{tag}"))
                && !lower[at..].contains(&format!("</{tag}"))
            {
                self.raw_tag = Some(tag);
                return true;
            }
        }
        indent >= 4 || line.starts_with('\t')
    }

    fn step_rst(&mut self, line: &str) -> bool {
        let indented = line.starts_with(' ') || line.starts_with('\t');
        let blank = line.trim().is_empty();
        if self.rst_active {
            if blank || indented {
                return true;
            }
            self.rst_active = false;
        }
        if self.rst_pending {
            if blank {
                return false;
            }
            self.rst_pending = false;
            if indented {
                self.rst_active = true;
                return true;
            }
        }
        let trimmed = line.trim_end();
        let lower = trimmed.to_ascii_lowercase();
        if RST_LITERAL_DIRECTIVES.iter().any(|d| lower.starts_with(d))
            || (trimmed.ends_with("::") && !trimmed.starts_with(".. "))
        {
            self.rst_pending = true;
        }
        false
    }
}

/// Whether the text after `slopvac:` makes the comment a steering marker. A comment whose body
/// is exactly `slopvac:begin` or `slopvac:end`, compared after trimming, is the marker that
/// the 2.x steering block carries, not a directive. Every other `slopvac:` comment is read as
/// a directive, so a mistyped one is reported.
fn is_steering_marker(after_keyword: &str) -> bool {
    matches!(after_keyword.trim_end(), "begin" | "end")
}

/// The byte range of the text after `slopvac:` in a directive comment, if `line` is one.
fn payload_of(line: &str, style: Style) -> Option<Range<usize>> {
    let lead = line.len() - line.trim_start().len();
    let body = &line[lead..];
    let (opens, closes): (&[&str], &[&str]) = match style {
        Style::Html => (&["<!--"], &["-->"]),
        Style::Rst => (&[".."], &[]),
        Style::Plain | Style::Code(_) => (&["//", "#", "--", ";", "/*"], &["*/"]),
    };
    let open = opens.iter().find(|p| body.starts_with(**p))?;
    let after_open = lead + open.len();
    let rest = &line[after_open..];
    let skipped = rest.len() - rest.trim_start().len();
    let key_at = after_open + skipped;
    let payload_at = key_at + "slopvac:".len();
    if !line[key_at..].starts_with("slopvac:") {
        return None;
    }
    let mut end = line.trim_end().len();
    for close in closes {
        if line[..end].ends_with(close) {
            end -= close.len();
            end = line[..end].trim_end().len();
            break;
        }
    }
    if style == Style::Html && !line.trim_end().ends_with("-->") {
        return None;
    }
    let end = end.max(payload_at);
    if is_steering_marker(&line[payload_at..end]) {
        return None;
    }
    Some(payload_at..end)
}

/// Split `key=value` pairs at white space; a value may be in double quotes.
///
/// Each pair, each dotted segment past the first and each list item past the first is charged
/// to `budget` before it is built, a repeated key included, so the work and the memory stay
/// bounded by the cap.
fn parse_pairs(doc: &str, payload: Range<usize>, budget: &mut Budget) -> Result<Node, Over> {
    let text = &doc[payload.clone()];
    let mut entries: Vec<Entry> = Vec::new();
    let mut slots = Slots::default();
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let key_end = i;
        let key = &text[key_start..key_end];
        let key_span = payload.start + key_start..payload.start + key_end;
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            let (value, value_span, next) = read_value(text, i, payload.start);
            i = next;
            insert(
                &mut Target {
                    entries: &mut entries,
                    slots: &mut slots,
                    budget,
                },
                key,
                key_span,
                Rawish::Str(value),
                value_span,
            )?;
        } else {
            insert(
                &mut Target {
                    entries: &mut entries,
                    slots: &mut slots,
                    budget,
                },
                key,
                key_span.clone(),
                Rawish::Value(Value::Bool(true)),
                key_span,
            )?;
        }
    }
    Ok(Node {
        span: payload,
        value: Value::Table(entries),
    })
}

fn read_value(text: &str, from: usize, base: usize) -> (&str, Range<usize>, usize) {
    let bytes = text.as_bytes();
    if bytes.get(from) == Some(&b'"') {
        let close = text[from + 1..]
            .find('"')
            .map_or(text.len(), |p| from + 1 + p);
        let end = (close + 1).min(text.len());
        return (&text[from + 1..close], base + from..base + end, end);
    }
    let mut i = from;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    (&text[from..i], base + from..base + i, i)
}

/// The list items of `raw` for a key that takes a comma-separated list.
fn list_items(raw: &str) -> impl Iterator<Item = &str> {
    raw.split(',').filter(|w| !w.is_empty())
}

fn typed(key: &str, raw: &str, span: &Range<usize>) -> Value {
    if key == "terms.allow" {
        return Value::Array(
            list_items(raw)
                .map(|w| Node {
                    span: span.clone(),
                    value: Value::Str(w.to_owned()),
                })
                .collect(),
        );
    }
    if let Ok(i) = raw.parse::<i64>() {
        Value::Int(i)
    } else if let Ok(f) = raw.parse::<f64>() {
        Value::Float(f)
    } else if raw == "true" || raw == "false" {
        Value::Bool(raw == "true")
    } else {
        Value::Str(raw.to_owned())
    }
}

/// How deep a dotted key may nest. The deepest field of the schema is three segments, so a
/// key past this is no setting, and its tables are never built: a deep tree would also recurse
/// on drop and on every walk over it.
pub const DIRECTIVE_MAX_DEPTH: usize = 8;

/// Where each key of a directive sits, so that a repeated key is found without a scan. A table
/// has an id; the root is 0.
#[derive(Default)]
struct Slots {
    index: HashMap<(usize, String), Slot>,
    tables: usize,
}

#[derive(Clone, Copy)]
struct Slot {
    /// Position in the entries of the table.
    at: usize,
    /// The id of the table the entry holds, when it holds one.
    table: Option<usize>,
}

impl Slots {
    /// The id of a table that no key points at yet.
    fn new_table(&mut self) -> usize {
        self.tables += 1;
        self.tables
    }
}

/// What `insert` adds to: the entries of the directive, the slots that index them, and the
/// budget that every addition is charged to.
struct Target<'a> {
    entries: &'a mut Vec<Entry>,
    slots: &'a mut Slots,
    budget: &'a mut Budget,
}

/// A value as written: text to type by its key, or a value that is already typed.
enum Rawish<'a> {
    Str(&'a str),
    Value(Value),
}

/// Add `dotted.key = value`, creating or extending the nested tables.
///
/// The pair costs one unit. Each segment past the first and each list item past the first
/// costs one more, charged before any table or item is built, and a key nested deeper than
/// [`DIRECTIVE_MAX_DEPTH`] is refused outright.
fn insert(
    target: &mut Target<'_>,
    dotted: &str,
    key_span: Range<usize>,
    raw: Rawish<'_>,
    value_span: Range<usize>,
) -> Result<(), Over> {
    let depth = dotted.bytes().filter(|&b| b == b'.').count() + 1;
    if depth > DIRECTIVE_MAX_DEPTH {
        return Err(Over);
    }
    let mut cost = depth;
    if let Rawish::Str(text) = &raw
        && dotted == "terms.allow"
    {
        cost = cost.saturating_add(list_items(text).count().saturating_sub(1));
    }
    target.budget.charge(cost)?;
    let mut offset = key_span.start;
    let mut cur = &mut *target.entries;
    let mut table = 0;
    let mut segments = dotted.split('.').peekable();
    while let Some(seg) = segments.next() {
        let span = offset..offset + seg.len();
        offset += seg.len() + 1;
        let slot = target.slots.index.get(&(table, seg.to_owned())).copied();
        if segments.peek().is_none() {
            let value = match &raw {
                Rawish::Str(s) => typed(dotted, s, &value_span),
                Rawish::Value(v) => v.clone(),
            };
            let entry = Entry {
                key: seg.to_owned(),
                key_span: span,
                node: Node {
                    span: value_span.clone(),
                    value,
                },
            };
            let at = if let Some(slot) = slot {
                cur[slot.at] = entry;
                slot.at
            } else {
                cur.push(entry);
                cur.len() - 1
            };
            target
                .slots
                .index
                .insert((table, seg.to_owned()), Slot { at, table: None });
            return Ok(());
        }
        let (at, child) = if let Some(Slot {
            at,
            table: Some(child),
        }) = slot
        {
            (at, child)
        } else {
            let child = target.slots.new_table();
            let entry = Entry {
                key: seg.to_owned(),
                key_span: span.clone(),
                node: Node {
                    span,
                    value: Value::Table(Vec::new()),
                },
            };
            let at = if let Some(slot) = slot {
                cur[slot.at] = entry;
                slot.at
            } else {
                cur.push(entry);
                cur.len() - 1
            };
            target.slots.index.insert(
                (table, seg.to_owned()),
                Slot {
                    at,
                    table: Some(child),
                },
            );
            (at, child)
        };
        let Value::Table(next) = &mut cur[at].node.value else {
            return Ok(());
        };
        cur = next;
        table = child;
    }
    Ok(())
}

/// The directives and settings a document may hold before its directives are refused: this
/// many, plus one for each [`DIRECTIVE_BYTES_PER_UNIT`] bytes of the document. The cap grows
/// with the document, so a long document with ordinary directives stays far below it, and the
/// work stays linear in the size of the document.
pub const DIRECTIVE_BASE_UNITS: usize = 1024;

/// Bytes of document that add one unit to the directive cap.
pub const DIRECTIVE_BYTES_PER_UNIT: usize = 32;

fn directive_cap(doc: &str) -> usize {
    DIRECTIVE_BASE_UNITS + doc.len() / DIRECTIVE_BYTES_PER_UNIT
}

/// True for the refusal of a document whose directives pass the cap. Its message holds only
/// the limit, never a value of the document.
#[must_use]
pub fn is_over_cap(diagnostic: &Diagnostic) -> bool {
    diagnostic.code == Code::InvalidValue && diagnostic.path == ["directive"]
}

/// The refusal of a document whose directives pass the cap, at `span`.
fn over_cap(cap: usize, span: Range<usize>) -> Read {
    Read {
        settings: Vec::new(),
        diagnostics: vec![Diagnostic {
            code: Code::InvalidValue,
            message: ConfigText::ListTooLong { max_items: cap }.to_string(),
            label: ConfigPhrase::LabelNotValidHere.text(),
            span,
            help: None,
            path: vec!["directive".into()],
            data: None,
        }],
    }
}

/// Read every directive in `doc`. `file` is the path shown in provenance.
///
/// Directives apply in document order, so a later one wins. A field outside the policy's allow
/// list is an error, and then the document's directives contribute nothing. So does a document
/// whose directives and settings together pass a cap that grows with its size.
#[must_use]
pub fn read(doc: &str, file: &str, policy: &Policy) -> Read {
    if !policy.enabled {
        return Read::default();
    }
    let mut budget = Budget::for_document(doc);
    let cap = budget.cap;
    // The markers, the pairs, the dotted segments and the list items are charged while the
    // document is scanned, so a hostile one is refused before it is collected or built.
    let payloads = match locate(doc, file, &mut budget) {
        Ok(payloads) => payloads,
        Err(Over) => return over_cap(cap, 0..0),
    };
    let mut directives = Vec::with_capacity(payloads.len());
    for payload in payloads {
        let Ok(node) = parse_pairs(doc, payload.clone(), &mut budget) else {
            return over_cap(cap, payload);
        };
        directives.push(Directive { payload, node });
    }
    let mut diagnostics = Vec::new();
    for d in &directives {
        diagnostics.extend(validate_directive(&d.node, doc));
        diagnostics.extend(denied(&d.node, policy, "directive"));
        if let Value::Table(entries) = &d.node.value
            && entries.is_empty()
        {
            diagnostics.push(Diagnostic {
                code: Code::MissingKey,
                message: ConfigPhrase::DirectiveEmpty.text(),
                label: ConfigPhrase::LabelWriteKeyValue.text(),
                span: d.payload.clone(),
                help: None,
                path: vec!["directive".into()],
                data: None,
            });
        }
    }
    if !diagnostics.is_empty() {
        return Read {
            settings: Vec::new(),
            diagnostics,
        };
    }
    let lines = LineIndex::new(doc);
    let mut settings = Vec::new();
    for d in &directives {
        settings.extend(flatten_with(
            &d.node,
            Origin {
                layer: ConfigLayer::Directive,
                file,
                text: doc,
            },
            &[],
            &lines,
        ));
    }
    Read {
        settings,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scan counts work in units, so these tests read the counter and not a clock. A marker
    /// line of 18 bytes holds one directive and no pair.
    const EMPTY_MARKER: &str = "<!-- slopvac: -->\n";

    #[test]
    fn the_scan_of_markers_stops_at_the_first_one_past_the_cap() {
        // 100 000 lines of 18 bytes make 1 800 000 bytes, so the cap is 1024 + 56 250 = 57 274
        // units, and the scan stops at the unit after it: 57 275.
        let doc = EMPTY_MARKER.repeat(100_000);
        let mut budget = Budget::for_document(&doc);
        assert!(locate(&doc, "a.md", &mut budget).is_err());
        assert_eq!((budget.cap, budget.spent), (57_274, 57_275));
    }

    #[test]
    fn the_scan_of_source_comments_stops_at_the_first_marker_past_the_cap() {
        // `// slopvac:` and a newline are 13 bytes, so 100 000 lines make 1 300 000 bytes and
        // the cap is 1024 + 40 625 = 41 649 units.
        let doc = "// slopvac:\n".repeat(100_000);
        let mut budget = Budget::for_document(&doc);
        assert!(locate(&doc, "a.rs", &mut budget).is_err());
        assert_eq!((budget.cap, budget.spent), (41_649, 41_650));
    }

    #[test]
    fn ordinary_comments_of_a_source_file_cost_nothing() {
        // 100 000 comments that are no directive: none is charged, and none is kept.
        let doc = "// note\n".repeat(100_000);
        let mut budget = Budget::for_document(&doc);
        let found = locate(&doc, "a.rs", &mut budget).unwrap_or_default();
        assert_eq!((found.len(), budget.spent), (0, 0));
    }

    #[test]
    fn a_key_nested_past_the_depth_limit_builds_no_table() {
        // Nine segments are one past the limit of eight: refused before the first is built.
        let doc = "<!-- slopvac: a.b.c.d.e.f.g.h.i=1 -->\n";
        let mut budget = Budget::for_document(doc);
        let payloads = locate(doc, "a.md", &mut budget).unwrap_or_default();
        assert_eq!(payloads.len(), 1);
        let parsed = parse_pairs(doc, payloads[0].clone(), &mut budget);
        assert!(parsed.is_err());
    }
}
