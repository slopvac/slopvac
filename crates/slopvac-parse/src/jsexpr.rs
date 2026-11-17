//! A brace checker for MDX expressions.
//!
//! Without a JavaScript parser, markdown-rs counts every `{` and `}` as code. That breaks
//! `{'{'}` and `{/* } */}`. This checker skips string, template and comment text. It does
//! not validate JavaScript and does not read regular expression literals: a `/` that is
//! not a division after an operand is a doubt, and plain counting takes over.
//!
//! The checker is conservative. It overrides plain counting only while it is sure that it
//! reads a literal or a comment. It falls back to plain counting for the rest of the
//! expression at the first doubt: a `<` that may open or close a JSX tag, a string that a
//! line break cuts off, a template substitution, or a `/` that may start a regex. Plain counting is what markdown-rs does with no hook.
//!
//! markdown-rs calls the checker once per candidate closing brace, each time with the whole
//! expression so far. The checker remembers how far it scanned, so a long expression costs
//! one pass and not one pass per brace.

use crate::sub;
use std::cell::{Cell, RefCell};

use markdown::{MdxExpressionKind, MdxSignal};

/// How many closing braces an unfinished expression may hold before the checker stops
/// reading it and counts plain braces for the rest of it.
const MAX_RETRIES: usize = 1000;

/// Bytes the checker may ask markdown-rs to rebuild across unfinished expressions.
/// markdown-rs copies the whole expression text on every call, so a huge expression with
/// many closing braces is quadratic. Past this budget the read fails and `parse_mdx` reads
/// the document again with plain counting.
const MAX_WORK: usize = 64 << 20;

/// Words after which a `/` starts a regular expression and not a division.
const KEYWORDS: &[&str] = &[
    "return",
    "typeof",
    "instanceof",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "throw",
    "case",
    "do",
    "else",
    "yield",
    "await",
];

/// Longest keyword, plus one so that longer words are not mistaken for it.
const WORD_CAP: usize = 11;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    #[default]
    Code,
    Str(char),
    LineComment,
    BlockComment,
}

/// The last significant token, as far as it decides what a `/` means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Prev {
    /// Start of the expression, an operator or an opening bracket: a `/` starts a regex.
    #[default]
    Other,
    /// An identifier, number, string or `]`: a `/` divides.
    Operand,
    /// A `)` or `}`: a `/` may divide or start a regex, as in `if (x) /re/.test(y)`.
    Closing,
}

#[derive(Debug, Default)]
struct Scan {
    /// The text scanned so far.
    seen: String,
    mode: Mode,
    depth: usize,
    /// Braces counted the way markdown-rs counts them with no hook: all of them.
    plain_depth: usize,
    /// Set at the first doubt. From then on only `plain_depth` decides.
    doubtful: bool,
    /// The last character was a `\` in a string or regex, or a `/` or `*` that may start
    /// or end a comment.
    pending: Option<char>,
    prev: Prev,
    /// The identifier being read, up to `WORD_CAP` characters.
    word: String,
    word_len: usize,
    /// The `+` or `-` just read, and what came before it. `x++` ends in an operand.
    last_sign: Option<(char, Prev)>,
    /// Calls since the expression began that found it incomplete.
    retries: usize,
}

/// JavaScript line terminators.
const fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

impl Scan {
    fn feed(&mut self, chunk: &str) {
        for c in chunk.chars() {
            #[cfg(test)]
            crate::work::add(&crate::work::CHECKER_STEPS, 1);
            match c {
                '{' => self.plain_depth += 1,
                '}' => self.plain_depth = self.plain_depth.saturating_sub(1),
                _ => {}
            }
            self.step(c);
        }
        self.seen.push_str(chunk);
    }

    fn end_word(&mut self) {
        if self.word_len > 0 {
            let keyword = self.word_len <= WORD_CAP && KEYWORDS.contains(&self.word.as_str());
            self.prev = if keyword { Prev::Other } else { Prev::Operand };
            self.word.clear();
            self.word_len = 0;
        }
    }

    fn step(&mut self, c: char) {
        let pending = self.pending.take();
        match self.mode {
            Mode::Str(q) => {
                if pending == Some('\\') {
                    // An escaped character: nothing to do.
                } else if c == '\\' {
                    self.pending = Some('\\');
                } else if c == q {
                    self.mode = Mode::Code;
                    self.prev = Prev::Operand;
                } else if q == '`' {
                    if c == '$' {
                        self.pending = Some('$');
                    } else if c == '{' && pending == Some('$') {
                        self.doubtful = true;
                    }
                } else if is_line_terminator(c) {
                    // A string cannot span lines: this is not what it looked like.
                    self.doubtful = true;
                    self.mode = Mode::Code;
                }
            }
            Mode::LineComment => {
                if is_line_terminator(c) {
                    self.mode = Mode::Code;
                }
            }
            Mode::BlockComment => {
                if pending == Some('*') && c == '/' {
                    self.mode = Mode::Code;
                } else if c == '*' {
                    self.pending = Some('*');
                }
            }
            Mode::Code => {
                if pending == Some('<') && (c.is_alphabetic() || matches!(c, '_' | '$' | '>' | '/'))
                {
                    // A JSX tag, or a comparison that looks like one.
                    self.doubtful = true;
                }
                if pending == Some('/') {
                    if c == '/' {
                        self.mode = Mode::LineComment;
                        return;
                    }
                    if c == '*' {
                        self.mode = Mode::BlockComment;
                        return;
                    }
                    // A division after an operand is certain. Anything else may be a
                    // regex literal, whose braces and quotes this checker does not read.
                    self.doubtful |= self.prev != Prev::Operand;
                    self.prev = Prev::Other;
                }
                self.code_char(c);
            }
        }
    }

    fn code_char(&mut self, c: char) {
        let last_sign = self.last_sign.take();
        if c.is_alphanumeric() || c == '_' || c == '$' {
            if self.word_len < WORD_CAP {
                self.word.push(c);
            }
            self.word_len += 1;
            return;
        }
        self.end_word();
        match c {
            '/' => {
                self.pending = Some('/');
            }
            '<' => {
                self.pending = Some('<');
                self.prev = Prev::Other;
            }
            '"' | '\'' | '`' => self.mode = Mode::Str(c),
            '{' => {
                self.depth += 1;
                self.prev = Prev::Other;
            }
            '}' => {
                self.depth = self.depth.saturating_sub(1);
                self.prev = Prev::Closing;
            }
            '+' | '-' => {
                if let Some((sign, before)) = last_sign
                    && sign == c
                    && before == Prev::Operand
                {
                    self.prev = Prev::Operand;
                } else {
                    self.last_sign = Some((c, self.prev));
                    self.prev = Prev::Other;
                }
            }
            ')' => self.prev = Prev::Closing,
            ']' => self.prev = Prev::Operand,
            _ if c.is_whitespace() => {}
            _ => self.prev = Prev::Other,
        }
    }

    fn complete(&self) -> bool {
        if self.doubtful {
            self.plain_depth == 0
        } else {
            self.mode == Mode::Code && self.depth == 0
        }
    }
}

/// The checker to pass as `mdx_expression_parse`.
#[derive(Debug, Default)]
pub(crate) struct ExpressionChecker {
    scan: RefCell<Scan>,
    work: Cell<usize>,
}

impl ExpressionChecker {
    pub(crate) fn check(&self, code: &str, _kind: &MdxExpressionKind) -> MdxSignal {
        let mut scan = self.scan.borrow_mut();
        if !code.starts_with(&scan.seen) {
            *scan = Scan::default();
        }
        let done = scan.seen.len();
        scan.feed(sub(code, done..));
        if scan.retries >= MAX_RETRIES {
            scan.doubtful = true;
        }
        if scan.complete() {
            *scan = Scan::default();
            return MdxSignal::Ok;
        }
        let spent = self.work.get() + code.len();
        self.work.set(spent);
        if spent > MAX_WORK {
            *scan = Scan::default();
            MdxSignal::Error(
                "Too many closing braces inside an unfinished expression".into(),
                code.len(),
                Box::new("slopvac-parse".into()),
                Box::new("expression-too-complex".into()),
            )
        } else {
            scan.retries += 1;
            MdxSignal::Eof(
                "Unexpected end of file in expression".into(),
                Box::new("slopvac-parse".into()),
                Box::new("unexpected-eof".into()),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::sub;
    use markdown::mdast::Node;
    use markdown::{Constructs, ParseOptions, to_mdast};
    use markdown::{MdxExpressionParse, MdxSignal};
    use proptest::prelude::*;

    use super::{ExpressionChecker, MAX_WORK};
    use crate::work::{CHECKER_STEPS, take};

    /// Pieces with no brace inside a string, template, comment or regex, so plain counting
    /// and the checker must agree.
    fn piece() -> impl Strategy<Value = &'static str> {
        prop_oneof![
            Just("<p>text</p>"),
            Just("<p>don't</p>"),
            Just("<br/>"),
            Just("<b>{x}</b>"),
            Just("a < b"),
            Just("/a+/g"),
            Just("/[/']/"),
            Just("n / 2"),
            Just("(n) / 2"),
            Just("x++ / 2"),
            Just("'a'"),
            Just("\"b\""),
            Just("'it\\'s'"),
            Just("`t ${x} u`"),
            Just("/* c */"),
            Just("/* it's */"),
            Just("// c\n"),
            Just("{ a: 1 }"),
            Just("x"),
            Just("return"),
        ]
    }

    fn options(hooked: bool) -> ParseOptions {
        let checker = ExpressionChecker::default();
        ParseOptions {
            constructs: Constructs::mdx(),
            mdx_expression_parse: hooked.then(|| {
                let parse: Box<MdxExpressionParse> =
                    Box::new(move |code: &str, kind: &_| checker.check(code, kind));
                parse
            }),
            ..ParseOptions::mdx()
        }
    }

    fn expressions(node: &Node, out: &mut Vec<(usize, usize)>) {
        if matches!(
            node,
            Node::MdxTextExpression(_) | Node::MdxFlowExpression(_)
        ) && let Some(p) = node.position()
        {
            out.push((p.start.offset, p.end.offset));
        }
        for child in node.children().into_iter().flatten() {
            expressions(child, out);
        }
    }

    fn ranges(source: &str, hooked: bool) -> Option<Vec<(usize, usize)>> {
        let tree = to_mdast(source, &options(hooked)).ok()?;
        let mut out = Vec::new();
        expressions(&tree, &mut out);
        Some(out)
    }

    proptest! {
        #[test]
        fn the_checker_and_plain_counting_find_the_same_expressions(
            pieces in prop::collection::vec(piece(), 0..8),
            more in prop::collection::vec(piece(), 0..4),
        ) {
            let source = format!("a {{{}}} b {{{}}} c\n", pieces.join(" "), more.join(" "));
            let plain = ranges(&source, false);
            let checked = ranges(&source, true);
            // When the checker's read fails, `parse_mdx` reads again with plain counting,
            // so the only way to differ is to succeed with other expressions.
            if plain.is_some() && checked.is_some() {
                prop_assert_eq!(checked, plain, "{:?}", source);
            }
        }
    }

    #[test]
    fn a_brace_in_a_string_is_the_one_intended_difference() {
        let source = "a {'{'} b\n";
        assert!(ranges(source, false).is_none());
        assert_eq!(ranges(source, true), Some(vec![(2, 7)]));
    }

    /// Calls the checker as markdown-rs does: once per closing brace, with all of the
    /// expression so far. Returns the signals.
    fn call_per_brace(content: &str, mut on_signal: impl FnMut(&MdxSignal) -> bool) {
        let checker = ExpressionChecker::default();
        let kind = markdown::MdxExpressionKind::Expression;
        for (i, c) in content.char_indices() {
            if c == '}' && !on_signal(&checker.check(sub(content, ..i), &kind)) {
                break;
            }
        }
    }

    #[test]
    fn the_checker_reads_each_character_once_however_many_braces_come() {
        for n in [500_usize, 2_000] {
            let content = format!("{}{}", "{x".repeat(n), "}".repeat(n));
            take(&CHECKER_STEPS);
            call_per_brace(&content, |_| true);
            let steps = take(&CHECKER_STEPS);
            // Re-reading the expression on each call would cost about n * len / 2.
            assert!(
                steps <= content.len(),
                "{steps} steps for {} bytes",
                content.len()
            );
        }
    }

    #[test]
    fn the_work_budget_ends_a_huge_unfinished_expression() {
        let content = format!("{}{}", "{x".repeat(60_000), "}".repeat(60_000));
        let mut asked = 0_usize;
        let mut failed = false;
        let kind = markdown::MdxExpressionKind::Expression;
        let checker = ExpressionChecker::default();
        for (i, c) in content.char_indices() {
            if c != '}' {
                continue;
            }
            asked += i;
            if matches!(
                checker.check(sub(&content, ..i), &kind),
                MdxSignal::Error(..)
            ) {
                failed = true;
                break;
            }
        }
        assert!(failed, "no error after asking for {asked} bytes");
        // The error comes as soon as the budget is spent, one call late at most.
        assert!(asked <= MAX_WORK + content.len(), "{asked}");
    }
}
