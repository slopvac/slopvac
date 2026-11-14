//! Find the comments of a source file without being fooled by strings.
//!
//! One small scanner per comment family. It tracks only what decides whether a comment marker
//! starts a comment: strings, raw strings, character literals, template literals and block
//! comments. It does not parse the language.

use std::ops::Range;

/// What a family of languages does with strings and comments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // independent language features, not a state
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
    /// Backtick strings (JavaScript, TypeScript, Go).
    backtick: bool,
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
    triple: false,
};

/// The comment family of a file extension, or `None` for text with no known comment syntax.
pub(crate) fn family_of(extension: &str) -> Option<Family> {
    let c = Family {
        c_like: true,
        ..NONE
    };
    Some(match extension {
        "rs" => Family { rust: true, ..c },
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "go" => Family {
            backtick: true,
            ..c
        },
        "java" | "kt" | "kts" | "scala" | "swift" => Family { triple: true, ..c },
        "c" | "h" | "cc" | "cpp" | "hpp" | "cs" | "php" | "dart" | "m" => c,
        "py" | "pyi" => Family {
            hash: true,
            triple: true,
            ..NONE
        },
        "rb" | "sh" | "bash" | "zsh" | "fish" | "pl" | "r" | "mk" | "toml" | "yaml" | "yml"
        | "ps1" | "conf" => Family { hash: true, ..NONE },
        "sql" | "lua" | "hs" | "adb" | "ads" => Family { dash: true, ..NONE },
        "lisp" | "clj" | "el" | "ini" | "asm" | "s" => Family { semi: true, ..NONE },
        _ => return None,
    })
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
    while i < b.len() {
        let next = b.get(i + 1).copied();
        match b[i] {
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
    i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')
}

/// The offset of the next line terminator at or after `from`, or the end of the text.
fn line_end(b: &[u8], from: usize) -> usize {
    b[from..]
        .iter()
        .position(|&c| c == b'\n' || c == b'\r')
        .map_or(b.len(), |p| from + p)
}

/// The end of a block comment whose body starts at `from`, and whether it was closed.
fn block_end(b: &[u8], from: usize, nested: bool) -> (usize, bool) {
    let mut depth = 1usize;
    let mut i = from;
    while i < b.len() {
        if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return (i, true);
            }
        } else if nested && b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            depth += 1;
            i += 2;
        } else {
            i += 1;
        }
    }
    (b.len(), false)
}

/// The end of a Rust raw string (`r"…"`, `r#"…"#`, `br"…"`) that starts at `i`, if one does.
fn raw_string_end(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i;
    if b[j] == b'b' {
        j += 1;
    }
    if b.get(j) != Some(&b'r') {
        return None;
    }
    j += 1;
    let hashes = b[j..].iter().take_while(|&&c| c == b'#').count();
    j += hashes;
    if b.get(j) != Some(&b'"') {
        return None;
    }
    j += 1;
    while j < b.len() {
        if b[j] == b'"' && b[j + 1..].iter().take_while(|&&c| c == b'#').count() >= hashes {
            return Some(j + 1 + hashes);
        }
        j += 1;
    }
    Some(b.len())
}

/// The offset after the string, character literal or template literal that starts at `i`.
fn skip_string(doc: &str, i: usize, fam: Family) -> usize {
    let b = doc.as_bytes();
    let quote = b[i];
    if quote == b'\'' && fam.rust {
        return rust_quote(doc, i);
    }
    if quote == b'`' {
        return if fam.backtick {
            skip_template(b, i + 1)
        } else {
            i + 1
        };
    }
    if quote == b'"' && fam.triple && b[i..].starts_with(b"\"\"\"") {
        return b[i + 3..]
            .windows(3)
            .position(|w| w == b"\"\"\"")
            .map_or(b.len(), |p| i + 3 + p + 3);
    }
    if quote == b'\'' && fam.triple && fam.hash && b[i..].starts_with(b"'''") {
        return b[i + 3..]
            .windows(3)
            .position(|w| w == b"'''")
            .map_or(b.len(), |p| i + 3 + p + 3);
    }
    let spans_lines = fam.rust;
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            c if c == quote => return j + 1,
            b'\n' | b'\r' if !spans_lines => return j,
            _ => j += 1,
        }
    }
    b.len()
}

/// A Rust `'`: a character literal is skipped whole, a lifetime only loses its quote.
fn rust_quote(doc: &str, i: usize) -> usize {
    let b = doc.as_bytes();
    if b.get(i + 1) == Some(&b'\\') {
        return b[i + 2..]
            .iter()
            .position(|&c| c == b'\'')
            .map_or(b.len(), |p| i + 2 + p + 1);
    }
    match doc[i + 1..].chars().next() {
        Some(c) if b.get(i + 1 + c.len_utf8()) == Some(&b'\'') => i + 1 + c.len_utf8() + 1,
        _ => i + 1,
    }
}

/// The end of a template literal whose body starts at `from`. `${ … }` holds code, which may
/// hold strings and templates of its own.
fn skip_template(b: &[u8], from: usize) -> usize {
    let mut j = from;
    while j < b.len() {
        match b[j] {
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
    while j < b.len() {
        match b[j] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return j + 1;
                }
            }
            b'`' => {
                j = skip_template(b, j + 1);
                continue;
            }
            q @ (b'"' | b'\'') => {
                j += 1;
                while j < b.len() && b[j] != q && b[j] != b'\n' {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
            }
            _ => {}
        }
        j += 1;
    }
    b.len()
}
