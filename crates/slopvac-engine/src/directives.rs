//! Inline directives: `slopvac-allow`, `slopvac-disable-next-line` and the
//! `slopvac-disable` / `slopvac-enable` pair. A port of `slopvac.suppression`.
//!
//! An `allow` names a rule and one exception from that rule's own closed list:
//!
//! ```text
//! <!-- slopvac-allow: rule=<pack>:<category>.<id> reason=<exception> -->
//! ```
//!
//! The 3.0 rule id carries its pack. A 2.x id has none and is reported with a pointer to
//! `slopvac migrate`; there is no compatibility mode. A directive that names no reason, a
//! reason the rule does not list, or an unknown rule is reported and does not suppress.
//! Directives written inside code blocks, front matter or inline code are examples and do
//! nothing.

use crate::span_between;
use crate::sub;
use std::collections::{BTreeSet, HashMap, HashSet};

use slopvac_model::Span;
use slopvac_parse::{BlockKind, Document};

/// The lines of a source text. Line numbers start at 1.
pub(crate) struct Lines<'a> {
    source: &'a str,
    /// `(start, end of content)` per line.
    spans: Vec<(usize, usize)>,
}

impl<'a> Lines<'a> {
    pub(crate) fn new(source: &'a str) -> Self {
        let bytes = source.as_bytes();
        let mut spans = Vec::new();
        let (mut start, mut i) = (0, 0);
        while let Some(&current) = bytes.get(i) {
            match current {
                b'\n' => {
                    spans.push((start, i));
                    start = i + 1;
                }
                b'\r' => {
                    spans.push((start, i));
                    if bytes.get(i + 1) == Some(&b'\n') {
                        i += 1;
                    }
                    start = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
        if start < bytes.len() {
            spans.push((start, bytes.len()));
        }
        Self { source, spans }
    }

    pub(crate) fn len(&self) -> usize {
        self.spans.len()
    }

    /// The text of one-based `line`, or nothing for a line the document does not have.
    pub(crate) fn text(&self, line: usize) -> &'a str {
        self.span_of(line)
            .map_or("", |&(s, e)| sub(self.source, s..e))
    }

    /// The offset where one-based `line` starts, or `None` for a line the document does not
    /// have, line 0 included.
    pub(crate) fn start(&self, line: usize) -> Option<usize> {
        self.span_of(line).map(|&(s, _)| s)
    }

    fn span_of(&self, line: usize) -> Option<&(usize, usize)> {
        line.checked_sub(1).and_then(|index| self.spans.get(index))
    }

    /// The one-based line holding `offset`.
    pub(crate) fn line_of(&self, offset: usize) -> usize {
        self.spans
            .partition_point(|&(s, _)| s <= offset)
            .max(1)
            .min(self.spans.len().max(1))
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (usize, &'a str)> + '_ {
        (1..=self.len()).map(|n| (n, self.text(n)))
    }
}

/// What the engine knows about the rule a directive names.
pub(crate) enum Target<'a> {
    /// No rule has this id.
    Unknown,
    /// The rule exists, and this run does not use it.
    Inactive,
    /// The rule runs. These are the exception names it lists.
    Active(Vec<&'a str>),
}

/// A directive that is not valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Invalid {
    pub(crate) span: Span,
    pub(crate) message: String,
}

struct Allow {
    rule: String,
    reason: Option<String>,
}

/// The directives of one document.
#[derive(Default)]
pub(crate) struct Directives {
    allows: HashMap<usize, Vec<Allow>>,
    disabled: HashSet<usize>,
    pub(crate) invalid: Vec<Invalid>,
}

impl Directives {
    /// Whether a finding of `rule` on `line` is covered. `exceptions` are the rule's names.
    pub(crate) fn suppressed(&self, rule: &str, exceptions: &[&str], line: usize) -> bool {
        self.disabled.contains(&line)
            || self.allows.get(&line).is_some_and(|entries| {
                entries.iter().any(|e| {
                    e.rule == rule
                        && e.reason
                            .as_deref()
                            .is_some_and(|reason| exceptions.contains(&reason))
                })
            })
    }
}

/// Block extents in lines, for the blocks a directive can cover.
fn covered_blocks(doc: &Document, lines: &Lines<'_>) -> Vec<(usize, usize)> {
    doc.blocks
        .iter()
        .filter(|b| {
            !matches!(
                b.kind,
                BlockKind::CodeBlock { .. } | BlockKind::Admonition(_)
            )
        })
        .map(|b| {
            (
                lines.line_of(b.span.start()),
                lines.line_of(b.span.end().saturating_sub(1).max(b.span.start())),
            )
        })
        .collect()
}

/// Lines that hold quoted text: code blocks and front matter.
fn quoted_lines(doc: &Document, lines: &Lines<'_>, front_matter: Option<Span>) -> HashSet<usize> {
    let mut quoted = HashSet::new();
    let mut add = |span: Span| {
        let first = lines.line_of(span.start());
        let last = lines.line_of(span.end().saturating_sub(1).max(span.start()));
        quoted.extend(first..=last);
    };
    for block in &doc.blocks {
        if matches!(block.kind, BlockKind::CodeBlock { .. }) {
            add(block.span);
        }
    }
    if let Some(span) = front_matter.filter(|s| !s.is_empty()) {
        add(span);
    }
    quoted
}

/// The lines one annotation on `annotation_line` applies to: the next line that can carry a
/// finding, skipping blank lines and further comments. A table start covers every row.
fn annotation_targets(lines: &Lines<'_>, annotation_line: usize) -> Vec<usize> {
    let mut index = annotation_line + 1;
    while index <= lines.len() {
        let stripped = lines.text(index).trim();
        if !stripped.is_empty() && !stripped.starts_with("<!--") {
            break;
        }
        index += 1;
    }
    if index > lines.len() {
        return Vec::new();
    }
    if !lines.text(index).trim_start().starts_with('|') {
        return vec![index];
    }
    let mut last = index;
    while last < lines.len() && lines.text(last + 1).trim_start().starts_with('|') {
        last += 1;
    }
    (index..=last).collect()
}

fn expand_block_targets(
    blocks: &[(usize, usize)],
    targets: Vec<usize>,
    annotation_line: usize,
) -> Vec<usize> {
    let same_line: Vec<_> = blocks.iter().filter(|b| b.0 == annotation_line).collect();
    if !same_line.is_empty() {
        return same_line.iter().flat_map(|b| b.0..=b.1).collect();
    }
    let mut expanded: BTreeSet<usize> = targets.into_iter().collect();
    for &(first, last) in blocks {
        if expanded.contains(&first) {
            expanded.extend(first..=last);
        }
    }
    expanded.into_iter().collect()
}

/// The backtick run lengths still open after `text[..end]`.
fn runs_toggled(open: &BTreeSet<usize>, text: &str, end: usize) -> BTreeSet<usize> {
    let mut state = open.clone();
    let bytes = text.as_bytes().get(..end).unwrap_or_default();
    let mut i = 0;
    while let Some(&current) = bytes.get(i) {
        if current == b'`' {
            let start = i;
            while bytes.get(i) == Some(&b'`') {
                i += 1;
            }
            let len = i - start;
            if !state.remove(&len) {
                state.insert(len);
            }
        } else {
            i += 1;
        }
    }
    state
}

/// Whether the directive at `at` is written in prose and not quoted in a code span.
fn written(open: &BTreeSet<usize>, line: &str, at: usize) -> bool {
    runs_toggled(open, line, at).is_empty()
}

/// The first `<!-- name -->` in `line`, as the byte offset of its `<!--`.
fn find_plain(line: &str, name: &str) -> Option<usize> {
    line.match_indices("<!--").map(|(i, _)| i).find(|&i| {
        sub(line, i + 4..)
            .trim_start()
            .strip_prefix(name)
            .is_some_and(|rest| rest.trim_start().starts_with("-->"))
    })
}

/// Every `<!-- slopvac-allow` in `line`, as the byte offset of its `<!--`.
fn find_attempts(line: &str) -> Vec<usize> {
    line.match_indices("<!--")
        .map(|(i, _)| i)
        .filter(|&i| {
            sub(line, i + 4..)
                .trim_start()
                .strip_prefix("slopvac-allow")
                .is_some_and(|rest| !rest.chars().next().is_some_and(is_word))
        })
        .collect()
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Parse a whole `<!-- slopvac-allow: rule=<id> [reason=<name>] -->`.
fn parse_allow(text: &str) -> Option<(String, Option<String>)> {
    let rest = text
        .strip_prefix("<!--")?
        .trim_start()
        .strip_prefix("slopvac-allow:")?
        .trim_start()
        .strip_prefix("rule=")?;
    let id_len = rest
        .find(|c: char| !(is_word(c) || matches!(c, '.' | ':' | '-')))
        .unwrap_or(rest.len());
    if id_len == 0 {
        return None;
    }
    let (mut rule, mut rest) = rest.split_at(id_len);
    // The id class holds `-`, so `rule=a.b-->` swallowed the arrow. Give it back, as a
    // backtracking match would.
    if rest.starts_with('>') && rule.ends_with("--") && rule.len() > 2 {
        let keep = rule.len() - 2;
        rest = sub(text, text.len() - rest.len() - 2..);
        rule = sub(rule, ..keep);
    }
    let trimmed = rest.trim_start();
    if rest.len() != trimmed.len()
        && let Some(after) = trimmed.strip_prefix("reason=")
    {
        let len = after
            .find(|c: char| !(is_word(c) || c == '-'))
            .unwrap_or(after.len());
        if len == 0 {
            return None;
        }
        let (reason, tail) = after.split_at(len);
        return (tail.trim_start() == "-->").then(|| (rule.to_owned(), Some(reason.to_owned())));
    }
    (trimmed == "-->").then(|| (rule.to_owned(), None))
}

/// Scan `source` for directives.
///
/// `lookup` classifies the rule a directive names. `front_matter` is the span of the front
/// matter, which holds no directives.
pub(crate) fn scan<'a>(
    doc: &Document,
    lines: &Lines<'_>,
    front_matter: Option<Span>,
    lookup: impl Fn(&str) -> Target<'a>,
) -> Directives {
    let mut out = Directives::default();
    let quoted = quoted_lines(doc, lines, front_matter);
    let blocks = covered_blocks(doc, lines);
    let targets_for =
        |number: usize| expand_block_targets(&blocks, annotation_targets(lines, number), number);
    let mut block_disabled = false;
    let mut open: BTreeSet<usize> = BTreeSet::new();
    for (number, line) in lines.iter() {
        if quoted.contains(&number) || line.trim().is_empty() {
            open.clear();
            if block_disabled {
                out.disabled.insert(number);
            }
            continue;
        }
        if find_plain(line, "slopvac-disable").is_some_and(|at| written(&open, line, at)) {
            block_disabled = true;
        }
        if find_plain(line, "slopvac-enable").is_some_and(|at| written(&open, line, at)) {
            block_disabled = false;
        }
        if block_disabled {
            out.disabled.insert(number);
        }
        if find_plain(line, "slopvac-disable-next-line").is_some_and(|at| written(&open, line, at))
        {
            out.disabled.extend(targets_for(number));
        }
        let attempts: Vec<usize> = find_attempts(line)
            .into_iter()
            .filter(|&at| written(&open, line, at))
            .collect();
        open = runs_toggled(&open, line, line.len());
        for at in attempts {
            let end = sub(line, at..)
                .find("-->")
                .map_or(line.len(), |c| at + c + 3);
            let raw = sub(line, at..end).trim();
            let Some(line_start) = lines.start(number) else {
                continue;
            };
            let span = span_between(line_start + at, line_start + at + raw.len());
            let mut invalid = |message: String| out.invalid.push(Invalid { span, message });
            let Some((rule, reason)) = parse_allow(raw) else {
                invalid(
                    "invalid suppression; expected \
                     <!-- slopvac-allow: rule=<pack>:<rule-id> reason=<reason> -->."
                        .to_owned(),
                );
                continue;
            };
            for target in targets_for(number) {
                out.allows.entry(target).or_default().push(Allow {
                    rule: rule.clone(),
                    reason: reason.clone(),
                });
            }
            if !rule.contains(':') {
                invalid(format!(
                    "suppression names {rule}, a 2.x rule id with no pack. Run `slopvac migrate` \
                     to qualify it as <pack>:{rule}."
                ));
                continue;
            }
            let exceptions = match lookup(&rule) {
                Target::Unknown => {
                    invalid(format!("suppression names unknown rule {rule}."));
                    continue;
                }
                Target::Inactive => continue,
                Target::Active(exceptions) => exceptions,
            };
            let valid = if exceptions.is_empty() {
                "none defined".to_owned()
            } else {
                exceptions.join(", ")
            };
            match reason {
                None => invalid(format!(
                    "suppression of {rule} names no reason. Add reason=<one of: {valid}>."
                )),
                Some(reason) if !exceptions.contains(&reason.as_str()) => invalid(format!(
                    "reason \"{reason}\" is not an exception of {rule}. Valid: {valid}."
                )),
                Some(_) => {}
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_allow_reads_a_namespaced_id_and_a_reason() {
        assert_eq!(
            parse_allow(
                "<!-- slopvac-allow: rule=core:orwell.stale-figure reason=dead-metaphor -->"
            ),
            Some((
                "core:orwell.stale-figure".to_owned(),
                Some("dead-metaphor".to_owned())
            ))
        );
    }

    #[test]
    fn parse_allow_reads_a_directive_with_no_reason() {
        assert_eq!(
            parse_allow("<!--slopvac-allow:rule=core:a.b-->"),
            Some(("core:a.b".to_owned(), None))
        );
    }

    #[test]
    fn parse_allow_rejects_trailing_words_and_a_bare_reason_key() {
        for bad in [
            "<!-- slopvac-allow: rule=core:a.b because -->",
            "<!-- slopvac-allow: rule=core:a.b reason= -->",
            "<!-- slopvac-allow: rule= -->",
            "<!-- slopvac-allow: core:a.b -->",
        ] {
            assert_eq!(parse_allow(bad), None, "{bad}");
        }
    }
}
