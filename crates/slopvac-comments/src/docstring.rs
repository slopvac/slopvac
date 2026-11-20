//! Python docstrings: the string statement that opens a module, class or function body.
//!
//! The lexer already knows where every comment and string is. This pass reads only the
//! code between them, as a few token kinds, to decide which strings open a body. It makes
//! one pass over the source, so its cost is linear.
//!
//! A string is a docstring when it is the first statement of the module, or the first
//! statement after the colon that ends a `def` or `class` header, and the statement is
//! nothing but that string (or adjacent strings, which Python joins). Comments, blank
//! lines and an encoding or shebang line are not statements. Byte strings and f-strings
//! are not documentation in Python, so they are not extracted.

use std::ops::Range;

/// Find the docstring literals, from the first prefix letter or quote to the closing quote.
///
/// `comments` and `strings` are the sorted, non-overlapping ranges the lexer found.
pub(crate) fn find(
    source: &str,
    comments: &[Range<usize>],
    strings: &[Range<usize>],
) -> Vec<Range<usize>> {
    let bytes = source.as_bytes();
    let mut walker = Walker::new();
    let mut cursor = if source.starts_with('\u{feff}') { 3 } else { 0 };
    let mut comments = comments.iter().peekable();
    let mut strings = strings.iter().peekable();
    loop {
        let comment_first = match (comments.peek(), strings.peek()) {
            (None, None) => break,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (Some(comment), Some(string)) => comment.start < string.start,
        };
        if comment_first {
            if let Some(range) = comments.next() {
                walker.code(bytes, cursor..range.start);
                cursor = cursor.max(range.end);
            }
        } else if let Some(range) = strings.next() {
            let (start, eligible) = prefix(bytes, cursor, range.start);
            walker.code(bytes, cursor..start);
            walker.string(start..range.end, eligible);
            cursor = cursor.max(range.end);
        }
    }
    walker.code(bytes, cursor..bytes.len());
    walker.finish()
}

/// Where the string that opens at `quote` really starts, counting a prefix such as `r` or
/// `rb` that sits right before it (no earlier than `floor`), and whether the prefix leaves
/// it a plain text string.
fn prefix(bytes: &[u8], floor: usize, quote: usize) -> (usize, bool) {
    let mut start = quote;
    while start > floor && quote - start < 2 && bytes[start - 1].is_ascii_alphabetic() {
        start -= 1;
    }
    let letters = bytes[start..quote].to_ascii_lowercase();
    // A longer run, or letters that follow an identifier character, are a name.
    let glued = start > floor && is_word(bytes[start - 1]);
    let valid = matches!(
        letters.as_slice(),
        b"" | b"r" | b"u" | b"b" | b"f" | b"t" | b"br" | b"rb" | b"fr" | b"rf" | b"tr" | b"rt"
    );
    if glued || !valid {
        return (quote, true);
    }
    let text = matches!(letters.as_slice(), b"" | b"r" | b"u");
    (start, text)
}

/// Whether `byte` can be part of an identifier or number. Every non-ASCII byte counts, as
/// most non-ASCII characters are identifier characters in Python.
const fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
}

/// What the walker has seen of the current `def` or `class` header.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Header {
    /// Not in a header.
    None,
    /// After `async` at the start of a line: a `def` must follow.
    Async,
    /// After `def` or `class`, before the colon that ends it.
    Open,
}

/// What the last token inside a parenthesised string statement was.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Last {
    Open,
    Str,
    Close,
}

struct Walker {
    /// Bracket nesting. Line ends and colons count only at depth zero.
    depth: usize,
    /// Nothing but blanks, comments and strings precede this point on its logical line.
    line_start: bool,
    header: Header,
    /// The bracket depth of each `lambda` in the header that still waits for its colon.
    lambdas: Vec<usize>,
    /// Set while reading a parenthesised statement that so far holds only strings and
    /// parentheses, as in `("doc")`. Python drops the parentheses, so it is a docstring.
    paren: Option<Last>,
    /// The outermost parenthesis of that statement has closed. Nothing but the end of the
    /// statement may follow, so `("a") "b"` is no docstring.
    closed: bool,
    /// The next statement is the first of a module, class or function body.
    body: bool,
    /// Adjacent plain strings that make up that statement so far.
    run: Vec<Range<usize>>,
    found: Vec<Range<usize>>,
}

impl Walker {
    const fn new() -> Self {
        Self {
            depth: 0,
            line_start: true,
            header: Header::None,
            lambdas: Vec::new(),
            paren: None,
            closed: false,
            body: true,
            run: Vec::new(),
            found: Vec::new(),
        }
    }

    fn finish(mut self) -> Vec<Range<usize>> {
        self.end_statement();
        self.found
    }

    /// The statement ended. The strings collected for it are docstrings.
    fn end_statement(&mut self) {
        if self.body {
            self.found.append(&mut self.run);
        }
        self.run.clear();
        self.closed = false;
        self.body = false;
    }

    /// Something other than a string statement came first, so there is no docstring.
    fn other(&mut self) {
        self.paren = None;
        self.closed = false;
        self.run.clear();
        self.body = false;
        self.line_start = false;
    }

    fn string(&mut self, range: Range<usize>, eligible: bool) {
        let allowed = match self.paren {
            Some(last) => last != Last::Close,
            None => self.depth == 0 && !self.closed,
        };
        if self.body && allowed && eligible {
            self.run.push(range);
            if self.paren.is_some() {
                self.paren = Some(Last::Str);
            }
            self.line_start = false;
        } else {
            self.other();
        }
    }

    fn code(&mut self, bytes: &[u8], range: Range<usize>) {
        let mut at = range.start;
        while at < range.end {
            match bytes[at] {
                b' ' | b'\t' | b'\x0c' => at += 1,
                b'\n' | b'\r' => {
                    at += if bytes[at] == b'\r' && bytes.get(at + 1) == Some(&b'\n') {
                        2
                    } else {
                        1
                    };
                    self.newline();
                }
                b'\\' if matches!(bytes.get(at + 1), Some(b'\n' | b'\r')) => {
                    // A backslash joins the next line to this one.
                    at += if bytes[at + 1] == b'\r' && bytes.get(at + 2) == Some(&b'\n') {
                        3
                    } else {
                        2
                    };
                }
                b'(' => {
                    self.open_paren();
                    at += 1;
                }
                b')' => {
                    self.close_paren();
                    at += 1;
                }
                b'[' | b'{' => {
                    self.depth += 1;
                    self.other();
                    at += 1;
                }
                b']' | b'}' => {
                    self.depth = self.depth.saturating_sub(1);
                    self.other();
                    at += 1;
                }
                b':' if self.header == Header::Open => {
                    self.header_colon();
                    at += 1;
                }
                b';' if self.depth == 0 => {
                    self.end_statement();
                    self.line_start = false;
                    at += 1;
                }
                byte if is_word(byte) => {
                    let end = at
                        + bytes[at..range.end]
                            .iter()
                            .take_while(|&&byte| is_word(byte))
                            .count();
                    self.word(&bytes[at..end]);
                    at = end;
                }
                _ => {
                    self.other();
                    at += 1;
                }
            }
        }
    }

    fn open_paren(&mut self) {
        let starts_statement = self.body && self.depth == 0 && self.run.is_empty() && !self.closed;
        match self.paren {
            Some(Last::Open) => {}
            None if starts_statement => self.paren = Some(Last::Open),
            Some(_) | None => self.other(),
        }
        self.depth += 1;
    }

    fn close_paren(&mut self) {
        self.depth = self.depth.saturating_sub(1);
        match self.paren {
            Some(Last::Str | Last::Close) => {
                // The statement is complete once the outermost parenthesis closes.
                if self.depth == 0 {
                    self.paren = None;
                    self.closed = true;
                } else {
                    self.paren = Some(Last::Close);
                }
            }
            _ => self.other(),
        }
    }

    /// A colon in a `def` or `class` header ends it, unless it belongs to a `lambda`.
    fn header_colon(&mut self) {
        if self.lambdas.last() == Some(&self.depth) {
            self.lambdas.pop();
            self.other();
        } else if self.depth == 0 {
            self.header = Header::None;
            self.lambdas.clear();
            self.line_start = false;
            self.body = true;
        } else {
            self.other();
        }
    }

    fn newline(&mut self) {
        if self.depth > 0 {
            return;
        }
        if self.header != Header::None {
            self.header = Header::None;
            self.lambdas.clear();
        }
        if !self.run.is_empty() {
            self.end_statement();
        }
        self.line_start = true;
    }

    fn word(&mut self, word: &[u8]) {
        let starts_header = match word {
            b"def" | b"class" => self.line_start || self.header == Header::Async,
            _ => false,
        };
        if word == b"lambda" && self.header == Header::Open {
            self.lambdas.push(self.depth);
        }
        let async_def = word == b"async" && self.line_start;
        self.other();
        if starts_header {
            self.header = Header::Open;
        } else if async_def {
            self.header = Header::Async;
        } else if self.header == Header::Async {
            self.header = Header::None;
        }
    }
}
