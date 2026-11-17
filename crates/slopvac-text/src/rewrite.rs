//! Rewrite the word-class escapes of a PCRE2 pattern to the Python `regex` meaning.
//!
//! Rule data is written for Python's `regex` module, where `\w` follows UTS #18. PCRE2's own
//! `\w` differs (see [`WORD`]), so the pattern is tokenised first and the escapes are
//! rewritten on tokens. Tokenising comes first because the escapes mean nothing inside a
//! `\Q..\E` quote, a `(?#..)` or extended-mode comment, or after `\c`, and because a class
//! changes what `\b` and `\W` are.

use crate::pattern::sub;

/// The Python `regex` module's `\w`, which follows UTS #18: Alphabetic, marks (M), decimal
/// digits (Nd), connector punctuation (Pc) and `Join_Control`. PCRE2's UCP `\w` is L, N,
/// Mn and Pc, so the pattern spells the class out. The last member is a class escape so a
/// `-` that follows cannot read as a range.
pub(crate) const WORD: &str = r"\p{Alphabetic}\p{M}\p{Nd}\x{200C}\x{200D}\p{Pc}";

/// A member of a character class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClassItem<'a> {
    /// Text kept as written: a literal, a range, a POSIX class, a quote or another escape.
    Raw(&'a str),
    /// `\w`.
    Word,
    /// `\W`.
    NonWord,
    /// A `-` that follows a class escape: a literal, never a range.
    Hyphen,
    /// A numeric escape or a bare `[`, normalised so it stays one member when its
    /// neighbours move: `\0`, `\101`, `\x4` and `\x{41}` all become `\x{..}`.
    Codepoint(u32),
}

/// A piece of a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Token<'a> {
    /// Text kept as written.
    Raw(&'a str),
    /// `\w` outside a class.
    Word,
    /// `\W` outside a class.
    NonWord,
    /// `\b` outside a class.
    Boundary,
    /// `\B` outside a class.
    NotBoundary,
    /// A character class.
    Class {
        /// Whether it starts with `^`.
        negated: bool,
        /// Its members.
        items: Vec<ClassItem<'a>>,
    },
}

struct Tokenizer<'a> {
    src: &'a str,
    pos: usize,
    /// Pending text not yet pushed as [`Token::Raw`].
    raw_start: usize,
    tokens: Vec<Token<'a>>,
    /// Whether `x` (extended) mode is on, per open group.
    extended: Vec<bool>,
}

/// Split `pattern` into tokens.
pub(crate) fn tokenize(pattern: &str) -> Vec<Token<'_>> {
    let mut t = Tokenizer {
        src: pattern,
        pos: 0,
        raw_start: 0,
        tokens: Vec::new(),
        extended: vec![false],
    };
    t.run();
    t.tokens
}

impl<'a> Tokenizer<'a> {
    fn rest(&self) -> &'a str {
        sub(self.src, self.pos..)
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    /// Move past one character.
    fn bump(&mut self) {
        if let Some(c) = self.peek() {
            self.pos += c.len_utf8();
        }
    }

    /// Move to just past the next `needle`, or to the end.
    fn skip_past(&mut self, needle: &str) {
        self.pos = self
            .rest()
            .find(needle)
            .map_or(self.src.len(), |i| self.pos + i + needle.len());
    }

    fn flush(&mut self) {
        if self.raw_start < self.pos {
            self.tokens
                .push(Token::Raw(sub(self.src, self.raw_start..self.pos)));
        }
    }

    /// Push a special token for the text since `start`, after the pending raw text.
    fn special(&mut self, start: usize, token: Token<'a>) {
        let end = self.pos;
        self.pos = start;
        self.flush();
        self.pos = end;
        self.raw_start = end;
        self.tokens.push(token);
    }

    fn extended(&self) -> bool {
        self.extended.last().copied().unwrap_or(false)
    }

    fn run(&mut self) {
        while let Some(c) = self.peek() {
            let start = self.pos;
            match c {
                '\\' => self.escape(start),
                '[' => self.class(start),
                '(' => self.group(),
                ')' => {
                    if self.extended.len() > 1 {
                        self.extended.pop();
                    }
                    self.bump();
                }
                '#' if self.extended() => {
                    self.skip_past("\n");
                }
                _ => self.bump(),
            }
        }
        self.flush();
    }

    /// An escape outside a class. `start` is at the backslash.
    fn escape(&mut self, start: usize) {
        self.bump();
        let Some(n) = self.peek() else { return };
        self.bump();
        match n {
            'w' => self.special(start, Token::Word),
            'W' => self.special(start, Token::NonWord),
            'b' => self.special(start, Token::Boundary),
            'B' => self.special(start, Token::NotBoundary),
            _ => self.escape_tail(n),
        }
    }

    /// What follows an escape letter that is not a word-class escape.
    fn escape_tail(&mut self, letter: char) {
        match letter {
            'Q' => self.skip_past("\\E"),
            'c' => self.bump(),
            'x' | 'p' | 'P' | 'N' | 'o' | 'g' | 'k' if self.peek() == Some('{') => {
                self.skip_past("}");
            }
            _ => {}
        }
    }

    /// A group opener, or a comment, a verb or an inline option.
    fn group(&mut self) {
        let rest = self.rest();
        if rest.starts_with("(?#") {
            self.skip_past(")");
        } else if let Some(verb) = rest.strip_prefix("(*") {
            // `(*pla:` and the other lower-case names open a group. Every other
            // `(*NAME)` or `(*NAME:argument)` is a verb whose argument, which may hold `#`,
            // `[` or `\`, runs to the first `)`.
            let name = verb.split([':', ')']).next().unwrap_or("");
            let opens_group = sub(verb, name.len()..).starts_with(':')
                && !name.is_empty()
                && name.chars().all(|c| c.is_ascii_lowercase() || c == '_');
            if opens_group {
                self.extended.push(self.extended());
                self.skip_past(":");
            } else {
                self.skip_past(")");
            }
        } else if let Some(options) = rest.strip_prefix("(?") {
            let len = options
                .find(|c: char| !(c.is_ascii_alphabetic() || c == '^' || c == '-'))
                .unwrap_or(options.len());
            let (flags, after) = options.split_at(len);
            let parent = self.extended();
            let set = apply_flags(flags, parent);
            if after.starts_with(')') {
                // `(?x)` lasts to the end of the enclosing group.
                if let Some(top) = self.extended.last_mut() {
                    *top = set;
                }
                self.pos += 2 + len + 1;
            } else if after.starts_with(':') {
                self.extended.push(set);
                self.pos += 2 + len + 1;
            } else {
                self.extended.push(parent);
                self.pos += 2;
            }
        } else {
            self.extended.push(self.extended());
            self.bump();
        }
    }

    /// A class. `start` is at the `[`.
    fn class(&mut self, start: usize) {
        self.bump();
        let negated = self.peek() == Some('^');
        if negated {
            self.bump();
        }
        let mut items = Vec::new();
        let mut item_start = self.pos;
        let mut first = true;
        loop {
            let Some(c) = self.peek() else {
                // Unterminated: leave it to PCRE2 to report.
                return;
            };
            let here = self.pos;
            match c {
                ']' if !first => {
                    push_raw(&mut items, self.src, item_start, here);
                    self.bump();
                    self.special(start, Token::Class { negated, items });
                    return;
                }
                '\\' => {
                    self.bump();
                    let Some(n) = self.peek() else { return };
                    self.bump();
                    let item = match n {
                        'w' => Some(ClassItem::Word),
                        'W' => Some(ClassItem::NonWord),
                        '0'..='7' => Some(ClassItem::Codepoint(self.digits(
                            n.to_digit(8).unwrap_or(0),
                            1,
                            8,
                            3,
                        ))),
                        '8' | '9' => Some(ClassItem::Codepoint(u32::from(n))),
                        'x' if self.peek() != Some('{') => {
                            Some(ClassItem::Codepoint(self.digits(0, 0, 16, 2)))
                        }
                        _ => {
                            self.escape_tail(n);
                            None
                        }
                    };
                    if let Some(item) = item {
                        push_raw(&mut items, self.src, item_start, here);
                        items.push(item);
                        if matches!(item, ClassItem::Word | ClassItem::NonWord)
                            && self.peek() == Some('-')
                        {
                            self.bump();
                            items.push(ClassItem::Hyphen);
                        }
                        item_start = self.pos;
                    }
                }
                '[' => {
                    if let Some(len) = self.posix_class_len() {
                        self.pos += len;
                    } else {
                        // A lone `[` becomes a code point, so a neighbour that is removed
                        // cannot leave it next to `:` and form a POSIX class.
                        push_raw(&mut items, self.src, item_start, here);
                        items.push(ClassItem::Codepoint(u32::from('[')));
                        self.bump();
                        item_start = self.pos;
                    }
                }
                _ => self.bump(),
            }
            first = false;
        }
    }

    /// Read digits in `radix` after `taken` of at most `max` have made `value`.
    fn digits(&mut self, mut value: u32, mut taken: usize, radix: u32, max: usize) -> u32 {
        while taken < max {
            let Some(d) = self.peek().and_then(|c| c.to_digit(radix)) else {
                break;
            };
            value = value * radix + d;
            self.bump();
            taken += 1;
        }
        value
    }

    /// The length of a `[:name:]` or `[:^name:]` at the cursor.
    fn posix_class_len(&self) -> Option<usize> {
        let body = self.rest().strip_prefix("[:")?;
        let name = body.strip_prefix('^').unwrap_or(body);
        let letters = name.find(|c: char| !c.is_ascii_alphabetic())?;
        sub(name, letters..)
            .starts_with(":]")
            .then(|| self.rest().len() - name.len() + letters + 2)
    }
}

fn push_raw<'a>(items: &mut Vec<ClassItem<'a>>, src: &'a str, from: usize, to: usize) {
    if from < to {
        items.push(ClassItem::Raw(sub(src, from..to)));
    }
}

/// The `x` state after the option letters of `(?flags)`. `^` unsets every option first.
fn apply_flags(flags: &str, current: bool) -> bool {
    let mut on = current;
    let mut setting = true;
    for c in flags.chars() {
        match c {
            '^' => on = false,
            '-' => setting = false,
            'x' => on = setting,
            _ => {}
        }
    }
    on
}

/// Render tokens as a pattern with every word-class escape spelled out.
pub(crate) fn render(tokens: &[Token<'_>]) -> String {
    let word = format!("[{WORD}]");
    let non_word = format!("[^{WORD}]");
    let boundary = format!("(?:(?<![{WORD}])(?=[{WORD}])|(?<=[{WORD}])(?![{WORD}]))");
    let not_boundary = format!("(?:(?<![{WORD}])(?![{WORD}])|(?<=[{WORD}])(?=[{WORD}]))");
    let mut out = String::new();
    for token in tokens {
        match token {
            Token::Raw(text) => out.push_str(text),
            Token::Word => out.push_str(&word),
            Token::NonWord => out.push_str(&non_word),
            Token::Boundary => out.push_str(&boundary),
            Token::NotBoundary => out.push_str(&not_boundary),
            Token::Class { negated, items } => render_class(&mut out, *negated, items),
        }
    }
    out
}

/// A class with `\W` has no single-class spelling, because PCRE2 cannot complement a
/// union. `[\W x]` is the word complement or `[ x]`; `[^\W x]` is a word character that
/// is not in `[ x]`.
fn render_class(out: &mut String, negated: bool, items: &[ClassItem<'_>]) {
    let mut rest = String::new();
    let mut non_word = false;
    for item in items {
        match item {
            ClassItem::Raw(text) => rest.push_str(text),
            ClassItem::Word => rest.push_str(WORD),
            ClassItem::NonWord => non_word = true,
            ClassItem::Hyphen => rest.push_str(r"\-"),
            ClassItem::Codepoint(c) => rest.push_str(&hex_escape(*c)),
        }
    }
    if rest.starts_with('^') {
        rest.insert(0, '\\');
    }
    let rendered = match (non_word, negated, rest.is_empty()) {
        (false, _, _) => {
            if rest.starts_with(']') {
                rest.insert(0, '\\');
            }
            let caret = if negated { "^" } else { "" };
            format!("[{caret}{rest}]")
        }
        (true, false, true) => format!("[^{WORD}]"),
        (true, false, false) => format!("(?:[^{WORD}]|[{rest}])"),
        (true, true, true) => format!("[{WORD}]"),
        (true, true, false) => format!("(?:(?![{rest}])[{WORD}])"),
    };
    out.push_str(&rendered);
}

/// A code point as a `\x{..}` escape.
fn hex_escape(c: u32) -> String {
    format!("\\x{{{c:X}}}")
}

/// Rewrite `\w`, `\W`, `\b` and `\B` in `pattern` to the Python meaning.
pub(crate) fn compat(pattern: &str) -> String {
    render(&tokenize(pattern))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quote_is_one_raw_token() {
        assert_eq!(tokenize(r"a\Q\w[\Eb"), [Token::Raw(r"a\Q\w[\Eb")]);
    }

    #[test]
    fn a_class_member_escape_is_split_out() {
        assert_eq!(
            tokenize(r"[a\w-]"),
            [Token::Class {
                negated: false,
                items: vec![ClassItem::Raw("a"), ClassItem::Word, ClassItem::Hyphen],
            }]
        );
    }

    #[test]
    fn a_leading_bracket_in_a_class_is_a_member() {
        assert_eq!(
            tokenize(r"[^]a]"),
            [Token::Class {
                negated: true,
                items: vec![ClassItem::Raw("]a")],
            }]
        );
    }

    #[test]
    fn a_comment_is_raw() {
        assert_eq!(
            tokenize(r"(?#\w[)\b"),
            [Token::Raw("(?#\\w[)"), Token::Boundary]
        );
    }
}
