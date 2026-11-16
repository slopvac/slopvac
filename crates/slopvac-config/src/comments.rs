//! Find the comments of a source file without being fooled by strings.
//!
//! One small scanner per comment family. It tracks only what decides whether a comment marker
//! starts a comment: strings, raw strings, character literals, template literals and block
//! comments. It does not parse the language.
//!
//! The language set follows the 2.x code-comment mode (`SOURCE_LANGUAGE_BY_EXTENSION` in
//! `packages/slopvac-lint`), with a few more plain-text formats.
//!
//! Known limitations. These constructs can hide a real comment or fake one, and a parser-backed
//! extractor would settle them (see bead slopvac-8l4.4.13):
//!
//! - JavaScript regular expression literals that hold a quote or a backtick, such as `/'/`.
//! - Python f-strings with nested quotes, and Ruby, shell and PHP heredocs and `%q` strings.
//! - C++ raw strings (`R"(…)"`), Swift `#"…"#` strings, Lua long strings (`[[…]]`) and Kotlin
//!   raw strings, whose backslashes do not escape.
//! - CSS, which has block comments only, is scanned with the C rules, so `//` in a `url(…)`
//!   at the start of a line reads as a comment.

use crate::sub;
use std::ops::Range;

/// What a family of languages does with strings and comments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent language features, not a state"
)]
pub(crate) struct Family {
    /// `//` and `/* */` comments.
    c_like: bool,
    /// `#` comments.
    hash: bool,
    /// `--` comments.
    dash: bool,
    /// `;` comments.
    semi: bool,
    /// Rust: raw strings, lifetimes, nested block comments, strings that span lines.
    rust: bool,
    /// Backtick template literals with `${…}` code and escapes (JavaScript, TypeScript).
    backtick: bool,
    /// Backtick raw strings with no escapes and no interpolation (Go).
    raw_backtick: bool,
    /// `"""` strings (Python, Java, Kotlin, Scala, Swift).
    triple: bool,
}

const NONE: Family = Family {
    c_like: false,
    hash: false,
    dash: false,
    semi: false,
    rust: false,
    backtick: false,
    raw_backtick: false,
    triple: false,
};

/// The comment family of a file extension, or `None` for text with no known comment syntax.
pub(crate) fn family_of(extension: &str) -> Option<Family> {
    let c = Family {
        c_like: true,
        ..NONE
    };
    let hash = Family { hash: true, ..NONE };
    Some(match extension {
        "rs" => Family { rust: true, ..c },
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" => Family {
            backtick: true,
            ..c
        },
        "go" => Family {
            raw_backtick: true,
            ..c
        },
        "java" | "kt" | "kts" | "scala" | "swift" => Family { triple: true, ..c },
        "c" | "h" | "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" | "cs" | "dart" | "m" | "fs"
        | "fsx" | "proto" | "css" | "scss" | "less" => c,
        "php" => Family { hash: true, ..c },
        "py" | "pyi" | "pyw" => Family {
            hash: true,
            triple: true,
            ..NONE
        },
        "rb" | "sh" | "bash" | "zsh" | "fish" | "pl" | "pm" | "r" | "ex" | "exs" | "jl" | "ps1"
        | "psm1" | "mk" | "toml" | "yaml" | "yml" | "conf" => hash,
        "sql" | "lua" | "hs" | "adb" | "ads" => Family { dash: true, ..NONE },
        "lisp" | "clj" | "cljs" | "el" | "ini" | "asm" | "s" => Family { semi: true, ..NONE },
        _ => return None,
    })
}

/// The comment family of a file known by its whole name, such as `Rakefile`.
pub(crate) fn family_of_name(name: &str) -> Option<Family> {
    match name.to_ascii_lowercase().as_str() {
        "sconstruct" => family_of("py"),
        "gemfile" | "rakefile" | "brewfile" => family_of("rb"),
        _ => None,
    }
}

/// One comment: where its marker starts and the range of text after the marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Comment {
    pub marker: usize,
    pub body: Range<usize>,
}

/// Every comment in `doc`, in order.
pub(crate) fn comments(doc: &str, fam: Family) -> Vec<Comment> {
    let b = doc.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&current) = b.get(i) {
        let next = b.get(i + 1).copied();
        match current {
            b'/' if fam.c_like && next == Some(b'/') => {
                let end = line_end(b, i);
                out.push(Comment {
                    marker: i,
                    body: i + 2..end,
                });
                i = end;
            }
            b'/' if fam.c_like && next == Some(b'*') => {
                let (end, closed) = block_end(b, i + 2, fam.rust);
                let body_end = if closed { end - 2 } else { end };
                out.push(Comment {
                    marker: i,
                    body: i + 2..body_end,
                });
                i = end;
            }
            b'#' if fam.hash => {
                let end = line_end(b, i);
                out.push(Comment {
                    marker: i,
                    body: i + 1..end,
                });
                i = end;
            }
            b'-' if fam.dash && next == Some(b'-') => {
                let end = line_end(b, i);
                out.push(Comment {
                    marker: i,
                    body: i + 2..end,
                });
                i = end;
            }
            b';' if fam.semi => {
                let end = line_end(b, i);
                out.push(Comment {
                    marker: i,
                    body: i + 1..end,
                });
                i = end;
            }
            b'"' | b'\'' | b'`' => i = skip_string(doc, i, fam),
            b'r' | b'b' if fam.rust && !ident_before(b, i) => {
                i = raw_string_end(b, i).unwrap_or(i + 1);
            }
            _ => i += 1,
        }
    }
    out
}

fn ident_before(b: &[u8], i: usize) -> bool {
    i.checked_sub(1)
        .and_then(|p| b.get(p))
        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
}

/// The offset of the next line terminator at or after `from`, or the end of the text.
fn line_end(b: &[u8], from: usize) -> usize {
    b.get(from..)
        .unwrap_or_default()
        .iter()
        .position(|&c| c == b'\n' || c == b'\r')
        .map_or(b.len(), |p| from + p)
}

/// The end of a block comment whose body starts at `from`, and whether it was closed.
fn block_end(b: &[u8], from: usize, nested: bool) -> (usize, bool) {
    let mut depth = 1usize;
    let mut i = from;
    while let Some(&current) = b.get(i) {
        if current == b'*' && b.get(i + 1) == Some(&b'/') {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return (i, true);
            }
        } else if nested && current == b'/' && b.get(i + 1) == Some(&b'*') {
            depth += 1;
            i += 2;
        } else {
            i += 1;
        }
    }
    (b.len(), false)
}

/// How many `#` start at `from`.
fn hash_run(b: &[u8], from: usize) -> usize {
    b.get(from..)
        .unwrap_or_default()
        .iter()
        .take_while(|&&c| c == b'#')
        .count()
}

/// The end of a Rust raw string (`r"…"`, `r#"…"#`, `br"…"`) that starts at `i`, if one does.
fn raw_string_end(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i;
    if b.get(j) == Some(&b'b') {
        j += 1;
    }
    if b.get(j) != Some(&b'r') {
        return None;
    }
    j += 1;
    let hashes = hash_run(b, j);
    j += hashes;
    if b.get(j) != Some(&b'"') {
        return None;
    }
    j += 1;
    while let Some(&c) = b.get(j) {
        if c == b'"' && hash_run(b, j + 1) >= hashes {
            return Some(j + 1 + hashes);
        }
        j += 1;
    }
    Some(b.len())
}

/// The offset after the string, character literal or template literal that starts at `i`.
fn skip_string(doc: &str, i: usize, fam: Family) -> usize {
    let b = doc.as_bytes();
    let Some(&quote) = b.get(i) else {
        return i + 1;
    };
    if quote == b'\'' && fam.rust {
        return rust_quote(doc, i);
    }
    if quote == b'`' {
        if fam.raw_backtick {
            return b
                .get(i + 1..)
                .unwrap_or_default()
                .iter()
                .position(|&c| c == b'`')
                .map_or(b.len(), |p| i + 1 + p + 1);
        }
        return if fam.backtick {
            skip_template(b, i + 1)
        } else {
            i + 1
        };
    }
    let here = b.get(i..).unwrap_or_default();
    if quote == b'"' && fam.triple && here.starts_with(b"\"\"\"") {
        return triple_end(b, i + 3, *b"\"\"\"");
    }
    if quote == b'\'' && fam.triple && fam.hash && here.starts_with(b"'''") {
        return triple_end(b, i + 3, *b"'''");
    }
    let spans_lines = fam.rust;
    let mut j = i + 1;
    while let Some(&current) = b.get(j) {
        match current {
            b'\\' => j += 2,
            c if c == quote => return j + 1,
            b'\n' | b'\r' if !spans_lines => return j,
            _ => j += 1,
        }
    }
    b.len()
}

/// The end of a triple-quoted string whose body starts at `from`. A backslash escapes the next
/// byte, so an escaped delimiter does not close the string.
fn triple_end(b: &[u8], from: usize, delim: [u8; 3]) -> usize {
    let mut j = from;
    while let Some(&current) = b.get(j) {
        if current == b'\\' {
            j += 2;
        } else if b.get(j..).unwrap_or_default().starts_with(&delim) {
            return j + 3;
        } else {
            j += 1;
        }
    }
    b.len()
}

/// A Rust `'`: a character literal is skipped whole, a lifetime only loses its quote.
fn rust_quote(doc: &str, i: usize) -> usize {
    let b = doc.as_bytes();
    if b.get(i + 1) == Some(&b'\\') {
        return b.get(i + 3..).map_or(b.len(), |rest| {
            rest.iter()
                .position(|&c| c == b'\'')
                .map_or(b.len(), |p| i + 3 + p + 1)
        });
    }
    match sub(doc, i + 1..).chars().next() {
        Some(c) if b.get(i + 1 + c.len_utf8()) == Some(&b'\'') => i + 1 + c.len_utf8() + 1,
        _ => i + 1,
    }
}

/// The end of a template literal whose body starts at `from`. `${ … }` holds code, which may
/// hold strings and templates of its own.
fn skip_template(b: &[u8], from: usize) -> usize {
    let mut j = from;
    while let Some(&current) = b.get(j) {
        match current {
            b'\\' => j += 2,
            b'`' => return j + 1,
            b'$' if b.get(j + 1) == Some(&b'{') => j = skip_braces(b, j + 2),
            _ => j += 1,
        }
    }
    b.len()
}

/// The end of a `${ … }` expression whose body starts at `from`.
fn skip_braces(b: &[u8], from: usize) -> usize {
    let mut depth = 1usize;
    let mut j = from;
    while let Some(&current) = b.get(j) {
        match current {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return j + 1;
                }
            }
            b'/' if b.get(j + 1) == Some(&b'/') => {
                j = line_end(b, j);
                continue;
            }
            b'/' if b.get(j + 1) == Some(&b'*') => {
                j = block_end(b, j + 2, false).0;
                continue;
            }
            b'`' => {
                j = skip_template(b, j + 1);
                continue;
            }
            q @ (b'"' | b'\'') => {
                j += 1;
                while b.get(j).is_some_and(|&c| c != q && c != b'\n') {
                    j += if b.get(j) == Some(&b'\\') { 2 } else { 1 };
                }
            }
            _ => {}
        }
        j += 1;
    }
    b.len()
}
