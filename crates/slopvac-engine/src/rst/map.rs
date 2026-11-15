//! Place the prose of a docutils tree in the RST source.
//!
//! docutils records the source lines of every block, and keeps the source text of every
//! inline construct (`rawsource`). So the mapper never searches the whole file. Each node is
//! placed inside its own lines, one piece after the other:
//!
//! - Text is matched character by character. A `\x` escape becomes a `backslash` token, an
//!   escaped space is a gap, and a run of whitespace becomes a `collapse` token that covers
//!   the newline and the indentation.
//! - An inline construct (emphasis, a link, a role) must start where the text before it
//!   ended. Its words are matched inside its own source text, so the markup
//!   around them stays a gap.
//! - Inline literals become `masked` tokens over the literal with its backquotes.
//!
//! The first piece of a node may follow a list marker, a directive header or a table
//! border, so the mapper looks for its first word inside the node's lines only.
//!
//! When a node cannot be placed word by word, its prose is kept and maps to the whole
//! trimmed lines of the node (an `opaque` token): the position is line-precise. A first
//! attempt allows one line of slack on each side, because docutils 0.21 puts the line of
//! some directive output (`rubric`) one line late. A node with no usable lines is reported
//! in a notice. No prose is dropped.
//!
//! The work is bounded. Every character looked at is charged to one budget that grows with
//! the size of the source and the text. Past it, nodes fall back to line precision without
//! any more matching, so the cost stays linear.

use slopvac_model::Span;
use slopvac_parse::{
    Admonition, AdmonitionSyntax, Block, BlockKind, Container as ParseContainer, Document,
    Projection, TokenKind,
};

use super::model::{Container, Node, Piece};
use super::{RstNotice, span};

/// Characters looked at per byte of source and text, plus a base for small documents.
pub(super) const WORK_PER_BYTE: usize = 64;
/// See [`WORK_PER_BYTE`].
pub(super) const WORK_BASE: usize = 1 << 20;

/// A projection token before it is pushed.
type Tok = (TokenKind, String, Span);

/// The result of mapping one document.
pub(super) struct Mapped {
    pub(super) document: Document,
    pub(super) notices: Vec<RstNotice>,
    /// Characters charged to the budget.
    pub(super) work: usize,
}

/// Map `nodes` into `source`. `bom` is the byte length of a leading byte order mark.
pub(super) fn project(source: &str, bom: usize, nodes: &[Node]) -> Mapped {
    let text_bytes: usize = nodes
        .iter()
        .map(|n| {
            n.text.len()
                + n.pieces
                    .iter()
                    .map(|p| p.t.len() + p.raw.len())
                    .sum::<usize>()
        })
        .sum();
    let mut mapper = Mapper {
        src: source,
        starts: line_starts(source, bom),
        work: 0,
        budget: WORK_PER_BYTE
            .saturating_mul(source.len().saturating_add(text_bytes))
            .saturating_add(WORK_BASE),
    };
    let mut blocks = Vec::new();
    let mut code_spans = Vec::new();
    let mut unplaced: Vec<&str> = Vec::new();
    let mut line_precise = 0_usize;
    for node in nodes {
        match mapper.block(node) {
            Outcome::Unplaced => unplaced.push(&node.tag),
            Outcome::Empty => {}
            Outcome::Block {
                block,
                codes,
                precise,
            } => {
                line_precise += usize::from(precise);
                code_spans.extend(codes);
                blocks.push(block);
            }
        }
    }
    blocks.sort_by_key(|b| (b.span.start(), std::cmp::Reverse(b.span.end())));
    code_spans.sort_by_key(Span::start);
    let mut notices = Vec::new();
    if !unplaced.is_empty() {
        notices.push(RstNotice {
            code: "rst-prose-unplaced",
            warning: true,
            message: format!(
                "{} RST node(s) have no usable source lines, so their prose was not checked \
                 (first: {})",
                unplaced.len(),
                unplaced[0]
            ),
        });
    }
    if line_precise > 0 {
        notices.push(RstNotice {
            code: "rst-line-precise",
            warning: false,
            message: format!(
                "{line_precise} RST block(s) could not be matched word by word, so findings in \
                 them point at the whole block"
            ),
        });
    }
    Mapped {
        document: Document { blocks, code_spans },
        notices,
        work: mapper.work,
    }
}

/// Start offsets of the lines: LF, CRLF and a lone CR end a line.
fn line_starts(source: &str, bom: usize) -> Vec<usize> {
    let bytes = source.as_bytes();
    let mut starts = vec![bom];
    let mut i = bom;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => starts.push(i + 1),
            b'\r' => {
                if bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                starts.push(i + 1);
            }
            _ => {}
        }
        i += 1;
    }
    starts
}

/// What became of a node.
enum Outcome {
    /// Its lines are missing or outside the file.
    Unplaced,
    /// It holds no prose.
    Empty,
    /// A block, with the spans of its inline code and whether it is line-precise.
    Block {
        block: Block,
        codes: Vec<Span>,
        precise: bool,
    },
}

/// A node placed in the source.
struct Placed {
    projection: Projection,
    span: Span,
    codes: Vec<Span>,
    line_precise: bool,
}

/// Tokens of one attempt.
struct Attempt {
    toks: Vec<Tok>,
    codes: Vec<Span>,
}

struct Mapper<'a> {
    src: &'a str,
    starts: Vec<usize>,
    work: usize,
    budget: usize,
}

impl Mapper<'_> {
    /// The block for `node`.
    fn block(&mut self, node: &Node) -> Outcome {
        let Some((lo, hi)) = node
            .start
            .zip(node.end)
            .and_then(|(s, e)| self.byte_range(s, e))
        else {
            return Outcome::Unplaced;
        };
        let containers: Vec<ParseContainer> = node
            .containers
            .iter()
            .map(|c| match *c {
                Container::Item { ordered, depth } => ParseContainer::ListItem { ordered, depth },
                Container::Quote => ParseContainer::BlockQuote,
            })
            .collect();
        let structural = |kind| Outcome::Block {
            block: Block {
                kind,
                span: span(lo, hi),
                projection: None,
                containers: containers.clone(),
            },
            codes: Vec::new(),
            precise: false,
        };
        if node.code {
            return structural(BlockKind::CodeBlock {
                lang: node.lang.clone(),
                info: None,
                body: span(lo, hi),
                fenced: false,
            });
        }
        if let Some(kind) = &node.adm {
            return structural(BlockKind::Admonition(Admonition {
                kind: kind.clone(),
                title: node.title.clone(),
                title_span: None,
                syntax: AdmonitionSyntax::Directive,
            }));
        }
        let Some(placed) = self.place(node, lo, hi) else {
            return Outcome::Empty;
        };
        let kind = match (node.level, node.cell) {
            (Some(level), _) => BlockKind::Heading { level },
            (None, Some((row, column))) => BlockKind::TableCell { row, column },
            _ => BlockKind::Paragraph,
        };
        Outcome::Block {
            block: Block {
                kind,
                span: placed.span,
                projection: Some(placed.projection),
                containers,
            },
            codes: placed.codes,
            precise: placed.line_precise,
        }
    }

    /// Byte range of lines `first..=last` (1-based), without the final terminator.
    fn byte_range(&self, first: usize, last: usize) -> Option<(usize, usize)> {
        let lines = self.starts.len();
        if first == 0 || first > lines || last < first {
            return None;
        }
        let last = last.min(lines);
        Some((self.starts[first - 1], self.line_end(last)))
    }

    /// End of line `line` (1-based) before its terminator.
    fn line_end(&self, line: usize) -> usize {
        if line >= self.starts.len() {
            return self.src.len();
        }
        let next = self.starts[line];
        let bytes = self.src.as_bytes();
        if next >= 2 && bytes[next - 2] == b'\r' && bytes[next - 1] == b'\n' {
            next - 2
        } else {
            next - 1
        }
    }

    /// Charge `units` of work. False once the budget is spent.
    fn charge(&mut self, units: usize) -> bool {
        self.work = self.work.saturating_add(units);
        self.work <= self.budget
    }

    const fn exhausted(&self) -> bool {
        self.work > self.budget
    }

    /// Place a prose node inside lines `lo..hi`.
    fn place(&mut self, node: &Node, lo: usize, hi: usize) -> Option<Placed> {
        if !self.exhausted() {
            let (first, last) = (node.start?, node.end?);
            let wider = self.byte_range(first.saturating_sub(1).max(1), last + 1);
            for range in [Some((lo, hi)), wider] {
                let Some((a, b)) = range else { continue };
                if let Some(done) = self.align(&node.pieces, a, b) {
                    return Self::finish(done);
                }
            }
        }
        self.fallback(&node.pieces, lo, hi)
    }

    fn finish(done: Attempt) -> Option<Placed> {
        let Attempt { mut toks, codes } = done;
        while toks
            .last()
            .is_some_and(|t| t.1.ends_with(char::is_whitespace))
        {
            toks.pop();
        }
        let lo = toks
            .iter()
            .filter(|t| t.0.is_real())
            .map(|t| t.2.start())
            .min()?;
        let hi = toks
            .iter()
            .filter(|t| t.0.is_real())
            .map(|t| t.2.end())
            .max()?;
        let mut projection = Projection::new();
        for (kind, text, at) in &toks {
            projection.push(*kind, text, *at);
        }
        Some(Placed {
            projection,
            span: span(lo, hi),
            codes,
            line_precise: false,
        })
    }

    /// The prose of the node as one token over its lines.
    fn fallback(&self, pieces: &[Piece], lo: usize, hi: usize) -> Option<Placed> {
        let mut text = String::new();
        for piece in pieces {
            match piece.k.as_str() {
                "t" | "i" => text.push_str(&piece.t),
                "c" => text.push('\0'),
                _ => {}
            }
        }
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            return None;
        }
        let slice = &self.src[lo..hi];
        let start = lo + (slice.len() - slice.trim_start().len());
        let end = (lo + slice.trim_end().len()).max(start);
        let at = span(start, end);
        let mut projection = Projection::new();
        projection.push(TokenKind::Opaque, &text, at);
        Some(Placed {
            projection,
            span: at,
            codes: Vec::new(),
            line_precise: true,
        })
    }

    /// Match all pieces of a node inside `lo..hi`.
    fn align(&mut self, pieces: &[Piece], lo: usize, hi: usize) -> Option<Attempt> {
        let mut out = Attempt {
            toks: Vec::new(),
            codes: Vec::new(),
        };
        let mut cursor = lo;
        let mut first = true;
        for piece in pieces {
            match piece.k.as_str() {
                "t" => {
                    let text = if first {
                        piece.t.trim_start()
                    } else {
                        &piece.t
                    };
                    if text.is_empty() {
                        continue;
                    }
                    if first {
                        cursor = self.find_word_start(cursor, hi, text)?;
                    }
                    cursor = self.text(text, cursor, hi, &mut out.toks)?;
                    first = false;
                }
                "c" | "i" | "o" => {
                    if piece.raw.is_empty() {
                        // Nothing to anchor on: treat the text as plain text.
                        if piece.k == "o" || piece.t.is_empty() {
                            continue;
                        }
                        if first {
                            cursor = self.find_word_start(cursor, hi, &piece.t)?;
                        }
                        cursor = self.text(&piece.t, cursor, hi, &mut out.toks)?;
                        first = false;
                        continue;
                    }
                    let begin = if first {
                        self.find_raw(cursor, hi, &piece.raw)?
                    } else {
                        self.skip_gap(cursor, hi)
                    };
                    let end = self.eat_raw(begin, hi, &piece.raw)?;
                    first = false;
                    match piece.k.as_str() {
                        "c" => {
                            let at = span(begin, end);
                            push(&mut out.toks, (TokenKind::Masked, "\0".to_owned(), at));
                            out.codes.push(at);
                        }
                        "i" => self.inline(piece, begin, end, &mut out.toks),
                        _ => {}
                    }
                    cursor = end;
                }
                _ => return None,
            }
        }
        Some(out)
    }

    /// Place the words of inline markup inside its own source text.
    fn inline(&mut self, piece: &Piece, begin: usize, end: usize, toks: &mut Vec<Tok>) {
        let raw = &self.src[begin..end];
        let mut skip = 0;
        if raw.starts_with(':')
            && let Some(i) = raw[1..].find(':')
        {
            skip = i + 2;
        }
        skip += raw[skip..].len() - raw[skip..].trim_start_matches(['*', '`', '_']).len();
        let mut inner = Vec::new();
        if self.text(&piece.t, begin + skip, end, &mut inner).is_some() {
            for tok in inner {
                push(toks, tok);
            }
        } else {
            push(toks, (TokenKind::Opaque, piece.t.clone(), span(begin, end)));
        }
    }

    /// Skip `\ ` and `\` + newline pairs, which docutils removes.
    fn skip_gap(&mut self, mut at: usize, hi: usize) -> usize {
        let bytes = self.src.as_bytes();
        while at + 1 < hi && bytes[at] == b'\\' {
            let Some(next) = self.src[at + 1..hi].chars().next() else {
                break;
            };
            if !next.is_whitespace() {
                break;
            }
            at += 1 + next.len_utf8();
        }
        at
    }

    /// The first position in `from..hi` where `raw` starts.
    fn find_raw(&mut self, from: usize, hi: usize, raw: &str) -> Option<usize> {
        let first = raw.chars().next()?;
        let mut at = from;
        while at < hi {
            let off = self.src[at..hi].find(first)?;
            if !self.charge(off + 1) {
                return None;
            }
            at += off;
            if self.eat_raw(at, hi, raw).is_some() {
                return Some(at);
            }
            at += first.len_utf8();
        }
        None
    }

    /// Match `raw` at `at`. Whitespace in `raw` matches any whitespace in the source,
    /// because docutils strips the indentation of continuation lines. Returns the end.
    fn eat_raw(&mut self, at: usize, hi: usize, raw: &str) -> Option<usize> {
        let mut p = at;
        let mut chars = raw.chars().peekable();
        while let Some(ch) = chars.next() {
            if !self.charge(1) {
                return None;
            }
            let c = self.src[p..hi].chars().next()?;
            if ch.is_whitespace() {
                if !c.is_whitespace() {
                    return None;
                }
                while chars.peek().is_some_and(|n| n.is_whitespace()) {
                    chars.next();
                }
                p += self.src[p..hi].len() - self.src[p..hi].trim_start().len();
            } else if c == ch {
                p += c.len_utf8();
            } else {
                return None;
            }
        }
        Some(p)
    }

    /// The first position in `from..hi` where the first word of `text` starts as a word.
    fn find_word_start(&mut self, from: usize, hi: usize, text: &str) -> Option<usize> {
        let word: String = text.chars().take_while(|c| !c.is_whitespace()).collect();
        let first = word.chars().next()?;
        let mut at = from;
        while at < hi {
            let (off, ch) = self.src[at..hi]
                .char_indices()
                .find(|&(_, c)| c == first || c == '\\')?;
            if !self.charge(off + 1) {
                return None;
            }
            at += off;
            let before_ok = self.src[..at]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphanumeric() || at == from);
            if before_ok {
                let mut scratch = Vec::new();
                if let Some(end) = self.text(&word, at, hi, &mut scratch) {
                    let after_ok = !word.chars().next_back().is_some_and(char::is_alphanumeric)
                        || !self.src[end..]
                            .chars()
                            .next()
                            .is_some_and(char::is_alphanumeric);
                    if after_ok {
                        return Some(at);
                    }
                }
            }
            at += ch.len_utf8();
        }
        None
    }

    /// Match `text` at `at`, inside `..hi`. Pushes tokens and returns the end. Source left
    /// over after the last character is a gap.
    fn text(&mut self, text: &str, at: usize, hi: usize, toks: &mut Vec<Tok>) -> Option<usize> {
        let mut p = at;
        let mut run = String::new();
        let mut run_start = at;
        let mut chars = text.chars().peekable();
        let flush = |toks: &mut Vec<Tok>, run: &mut String, start: usize, end: usize| {
            if !run.is_empty() {
                push(
                    toks,
                    (TokenKind::Identity, std::mem::take(run), span(start, end)),
                );
            }
        };
        while let Some(ch) = chars.next() {
            if !self.charge(1) {
                return None;
            }
            if ch.is_whitespace() {
                let mut next = None;
                while let Some(&n) = chars.peek() {
                    if n.is_whitespace() {
                        chars.next();
                    } else {
                        next = Some(n);
                        break;
                    }
                }
                flush(toks, &mut run, run_start, p);
                let begin = p;
                // Table borders sit between the words of a cell.
                while let Some(c) = self.src[p..hi].chars().next() {
                    if c.is_whitespace()
                        || (next.is_some() && matches!(c, '|' | '+') && next != Some(c))
                    {
                        p += c.len_utf8();
                    } else {
                        break;
                    }
                }
                if p > begin {
                    push(toks, (TokenKind::Collapse, " ".to_owned(), span(begin, p)));
                } else {
                    let end = toks.last().map_or(begin, |t| t.2.end());
                    push(toks, (TokenKind::Wrap, " ".to_owned(), Span::empty(end)));
                }
                run_start = p;
                continue;
            }
            // `\ ` and `\` + newline vanish from the text.
            let mut q = p;
            loop {
                let c = self.src[q..hi].chars().next()?;
                if c != '\\' {
                    break;
                }
                let next = self.src[q + 1..hi].chars().next()?;
                if next == ch {
                    break;
                }
                if !next.is_whitespace() {
                    return None;
                }
                q += 1 + next.len_utf8();
            }
            if q > p {
                flush(toks, &mut run, run_start, p);
                p = q;
                run_start = p;
            }
            let c = self.src[p..hi].chars().next()?;
            if c == '\\' {
                // An escape: the backslash and the character it protects.
                let next = self.src[p + 1..hi].chars().next()?;
                if next != ch {
                    return None;
                }
                flush(toks, &mut run, run_start, p);
                let end = p + 1 + next.len_utf8();
                push(toks, (TokenKind::Backslash, ch.to_string(), span(p, end)));
                p = end;
                run_start = p;
            } else if c == ch {
                if run.is_empty() {
                    run_start = p;
                }
                run.push(c);
                p += c.len_utf8();
            } else {
                return None;
            }
        }
        flush(toks, &mut run, run_start, p);
        Some(p)
    }
}

/// Push a token. Whitespace that follows whitespace, or opens the list, is dropped.
fn push(toks: &mut Vec<Tok>, tok: Tok) {
    let blank = tok.1.chars().all(char::is_whitespace);
    if blank
        && toks
            .last()
            .is_none_or(|t| t.1.ends_with(char::is_whitespace))
    {
        return;
    }
    toks.push(tok);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::assert_is_empty)]

    use super::*;
    use crate::rst::model::Output;

    fn nodes(json: &str) -> Vec<Node> {
        serde_json::from_str::<Output>(&format!(r#"{{"docutils":"0.23","nodes":{json}}}"#))
            .unwrap()
            .nodes
    }

    fn paragraph(start: usize, end: usize, text: &str) -> String {
        format!(
            r#"{{"tag":"paragraph","start":{start},"end":{end},"pieces":[{{"k":"t","t":{}}}]}}"#,
            serde_json::to_string(text).unwrap()
        )
    }

    fn first_prose(mapped: &Mapped) -> (String, Span) {
        let block = &mapped.document.blocks[0];
        (
            block.projection.as_ref().unwrap().text().to_owned(),
            block.span,
        )
    }

    #[test]
    fn a_line_off_by_one_is_found_with_slack() {
        // docutils 0.21 reports a rubric one line late: the text is on line 2, not 3.
        let source = "Intro\n\nA rubric\n\nAfter.\n";
        let line_of_rubric = 3;
        let mapped = project(
            source,
            0,
            &nodes(&format!(
                "[{}]",
                paragraph(line_of_rubric + 1, line_of_rubric + 1, "A rubric")
            )),
        );
        let (text, at) = first_prose(&mapped);
        assert_eq!(text, "A rubric");
        assert_eq!(&source[at.start()..at.end()], "A rubric");
        assert!(mapped.notices.is_empty());
    }

    #[test]
    fn text_that_cannot_be_matched_keeps_its_prose_at_line_precision() {
        let source = "One\n\n  Words that docutils changed.  \n\nTwo\n";
        let mapped = project(
            source,
            0,
            &nodes(&format!(
                "[{}]",
                paragraph(3, 3, "Words that docutils rewrote.")
            )),
        );
        let (text, at) = first_prose(&mapped);
        assert_eq!(text, "Words that docutils rewrote.");
        assert_eq!(
            &source[at.start()..at.end()],
            "Words that docutils changed."
        );
        let codes: Vec<_> = mapped.notices.iter().map(|n| (n.code, n.warning)).collect();
        assert_eq!(codes, [("rst-line-precise", false)]);
    }

    #[test]
    fn a_node_without_lines_is_reported_not_dropped_silently() {
        let mapped = project(
            "Text\n",
            0,
            &nodes(
                r#"[{"tag":"classifier","start":null,"end":null,"pieces":[{"k":"t","t":"Text"}]}]"#,
            ),
        );
        assert!(mapped.document.blocks.is_empty());
        assert_eq!(mapped.notices.len(), 1);
        assert_eq!(mapped.notices[0].code, "rst-prose-unplaced");
        assert!(mapped.notices[0].warning);
        assert!(mapped.notices[0].message.contains("classifier"));
    }

    #[test]
    fn a_line_past_the_end_is_reported() {
        let mapped = project(
            "Text\n",
            0,
            &nodes(&format!("[{}]", paragraph(9, 9, "Text"))),
        );
        assert!(mapped.document.blocks.is_empty());
        assert_eq!(mapped.notices[0].code, "rst-prose-unplaced");
    }

    #[test]
    fn an_escaped_space_is_a_gap_inside_a_word() {
        let source = "foo\\ bar\n";
        let mapped = project(
            source,
            0,
            &nodes(&format!("[{}]", paragraph(1, 1, "foobar"))),
        );
        let (text, at) = first_prose(&mapped);
        assert_eq!(text, "foobar");
        assert_eq!(&source[at.start()..at.end()], "foo\\ bar");
        assert!(mapped.notices.is_empty());
    }

    #[test]
    fn crlf_and_lone_cr_end_lines() {
        let source = "one\r\ntwo\rthree\n";
        let mapped = project(
            source,
            0,
            &nodes(&format!(
                "[{},{}]",
                paragraph(2, 2, "two"),
                paragraph(3, 3, "three")
            )),
        );
        let got: Vec<_> = mapped
            .document
            .blocks
            .iter()
            .map(|b| &source[b.span.start()..b.span.end()])
            .collect();
        assert_eq!(got, ["two", "three"]);
    }

    #[test]
    fn a_hostile_prefix_costs_bounded_deterministic_work() {
        // The first word is almost the text on its line: every start position matches for
        // thousands of characters before it fails.
        let run = 4_000;
        let source = format!("{}b\n", "a".repeat(run));
        let word = format!("{}c", "a".repeat(run));
        let mapped = project(&source, 0, &nodes(&format!("[{}]", paragraph(1, 1, &word))));
        let budget = WORK_PER_BYTE * (source.len() + word.len()) + WORK_BASE;
        assert!(
            mapped.work <= budget + run,
            "work {} over budget {budget}",
            mapped.work
        );
        // The prose is kept, at line precision.
        let (text, at) = first_prose(&mapped);
        assert_eq!(text, word);
        assert_eq!(
            &source[at.start()..at.end()],
            format!("{}b", "a".repeat(run))
        );
    }

    #[test]
    fn ordinary_prose_costs_a_few_characters_of_work_per_byte() {
        let mut source = String::new();
        let mut json = Vec::new();
        for i in 0..2_000 {
            let line = format!("Paragraph number {i} has some ordinary words in it.");
            source.push_str(&line);
            source.push_str("\n\n");
            json.push(paragraph(2 * i + 1, 2 * i + 1, &line));
        }
        let mapped = project(&source, 0, &nodes(&format!("[{}]", json.join(","))));
        assert_eq!(mapped.document.blocks.len(), 2_000);
        assert!(mapped.notices.is_empty());
        // Each character is compared once: the work is about twice the text, never more
        // than four times.
        assert!(
            mapped.work <= 4 * source.len(),
            "{} for {}",
            mapped.work,
            source.len()
        );
    }
}
