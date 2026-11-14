//! A brace checker for MDX expressions.
//!
//! Without a JavaScript parser, markdown-rs counts every `{` and `}` as code. That breaks
//! `{'{'}`, `{/* } */}` and `{/}/.test(x)}`. This checker skips string, template, comment
//! and regular expression text. It does not validate JavaScript.
//!
//! markdown-rs calls the checker once per candidate closing brace, each time with the whole
//! expression so far. The checker remembers how far it scanned, so a long expression costs
//! one pass and not one pass per brace.

use std::cell::RefCell;

use markdown::{MdxExpressionKind, MdxSignal};

/// How many closing braces an unfinished expression may hold. markdown-rs rebuilds the
/// expression text on every call, so an unbounded count is quadratic.
const MAX_RETRIES: usize = 1000;

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
    Regex {
        in_class: bool,
    },
}

/// The last significant token, as far as it decides what a `/` means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Prev {
    /// Start of the expression, an operator or an opening bracket: a `/` starts a regex.
    #[default]
    Other,
    /// An identifier, number, string, or a closing bracket: a `/` divides.
    Operand,
}

#[derive(Debug, Default)]
struct Scan {
    /// The text scanned so far.
    seen: String,
    mode: Mode,
    depth: usize,
    /// The last character was a `\` in a string or regex, or a `/` or `*` that may start
    /// or end a comment.
    pending: Option<char>,
    prev: Prev,
    /// Whether a pending `/` would start a regex.
    slash_is_regex: bool,
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
            Mode::Regex { in_class } => {
                if pending == Some('\\') {
                    // An escaped character: nothing to do.
                } else if c == '\\' {
                    self.pending = Some('\\');
                } else if is_line_terminator(c) {
                    // Not a regex after all.
                    self.mode = Mode::Code;
                } else if in_class {
                    if c == ']' {
                        self.mode = Mode::Regex { in_class: false };
                    }
                } else if c == '[' {
                    self.mode = Mode::Regex { in_class: true };
                } else if c == '/' {
                    self.mode = Mode::Code;
                    self.prev = Prev::Operand;
                }
            }
            Mode::Code => {
                if pending == Some('/') {
                    if c == '/' {
                        self.mode = Mode::LineComment;
                        return;
                    }
                    if c == '*' {
                        self.mode = Mode::BlockComment;
                        return;
                    }
                    if self.slash_is_regex {
                        self.mode = Mode::Regex { in_class: false };
                        self.step(c);
                        return;
                    }
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
                self.slash_is_regex = self.prev == Prev::Other;
            }
            '"' | '\'' | '`' => self.mode = Mode::Str(c),
            '{' => {
                self.depth += 1;
                self.prev = Prev::Other;
            }
            '}' => {
                self.depth = self.depth.saturating_sub(1);
                self.prev = Prev::Operand;
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
            ')' | ']' => self.prev = Prev::Operand,
            _ if c.is_whitespace() => {}
            _ => self.prev = Prev::Other,
        }
    }

    fn complete(&self) -> bool {
        self.mode == Mode::Code && self.depth == 0
    }
}

/// The checker to pass as `mdx_expression_parse`.
#[derive(Debug, Default)]
pub(crate) struct ExpressionChecker {
    scan: RefCell<Scan>,
}

impl ExpressionChecker {
    pub(crate) fn check(&self, code: &str, _kind: &MdxExpressionKind) -> MdxSignal {
        let mut scan = self.scan.borrow_mut();
        if !code.starts_with(&scan.seen) {
            *scan = Scan::default();
        }
        let done = scan.seen.len();
        scan.feed(&code[done..]);
        if scan.complete() {
            *scan = Scan::default();
            MdxSignal::Ok
        } else if scan.retries >= MAX_RETRIES {
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
