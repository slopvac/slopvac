//! A brace checker for MDX expressions.
//!
//! Without a JavaScript parser, markdown-rs counts every `{` and `}` as code. That breaks
//! `{'{'}` and `{/* } */}`. This checker skips string, template and comment text. It does
//! not validate JavaScript.
//!
//! markdown-rs calls the checker once per candidate closing brace, each time with the whole
//! expression so far. The checker remembers how far it scanned, so a long expression costs
//! one pass and not one pass per brace.

use std::cell::RefCell;

use markdown::{MdxExpressionKind, MdxSignal};

/// How many closing braces an unfinished expression may hold. markdown-rs rebuilds the
/// expression text on every call, so an unbounded count is quadratic.
const MAX_RETRIES: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    #[default]
    Code,
    Str(char),
    LineComment,
    BlockComment,
}

#[derive(Debug, Default)]
struct Scan {
    /// The text scanned so far.
    seen: String,
    mode: Mode,
    depth: usize,
    /// The last character was a `\` in a string, or a `/` or `*` that may start or end
    /// a comment.
    pending: Option<char>,
    /// Calls since the expression began that found it incomplete.
    retries: usize,
}

impl Scan {
    fn feed(&mut self, chunk: &str) {
        for c in chunk.chars() {
            self.step(c);
        }
        self.seen.push_str(chunk);
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
                }
            }
            Mode::LineComment => {
                if c == '\n' {
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
            Mode::Code => match (pending, c) {
                (Some('/'), '/') => self.mode = Mode::LineComment,
                (Some('/'), '*') => self.mode = Mode::BlockComment,
                (_, '/') => self.pending = Some('/'),
                (_, '"' | '\'' | '`') => self.mode = Mode::Str(c),
                (_, '{') => self.depth += 1,
                (_, '}') => self.depth = self.depth.saturating_sub(1),
                _ => {}
            },
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
