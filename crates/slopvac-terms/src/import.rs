//! Import word lists from other tools into a terms file.

use serde::Deserialize;

use crate::TermsError;
use crate::messages::TermsText;

/// A word list in another tool's format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Vale `accept.txt`: one word or regular expression per line.
    Vale,
    /// cspell configuration: `words` and `ignoreWords` arrays.
    Cspell,
    /// codespell ignore list: one word per line, or `ignore-words-list = a,b`.
    Codespell,
}

impl Source {
    /// Parse the `--from` value.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "vale" => Some(Self::Vale),
            "cspell" => Some(Self::Cspell),
            "codespell" => Some(Self::Codespell),
            _ => None,
        }
    }
}

/// Words read from a file, and entries that could not be imported.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Imported {
    /// Plain words, in file order, without duplicates.
    pub words: Vec<String>,
    /// Entries skipped because they are patterns, not words.
    pub skipped: Vec<String>,
}

fn push(out: &mut Imported, word: &str) {
    let word = word.trim();
    if word.is_empty() {
        return;
    }
    if word.chars().any(|c| "[](){}*+?|^$\\".contains(c)) {
        out.skipped.push(word.to_owned());
    } else if !out.words.iter().any(|w| w == word) {
        out.words.push(word.to_owned());
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cspell {
    #[serde(default)]
    words: Vec<String>,
    #[serde(default)]
    ignore_words: Vec<String>,
}

fn strip_json_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_str = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            if c == '\\' {
                out.extend(chars.next());
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
            out.push(c);
        } else if c == '/' && chars.peek() == Some(&'/') {
            for n in chars.by_ref() {
                if n == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut prev = ' ';
            for n in chars.by_ref() {
                if prev == '*' && n == '/' {
                    break;
                }
                prev = n;
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Read a word list.
///
/// # Errors
/// [`TermsError::Parse`] when a cspell file is not valid JSON.
pub fn parse(path: &str, source: Source, text: &str) -> Result<Imported, TermsError> {
    let mut out = Imported::default();
    match source {
        Source::Vale => {
            // Vale: a line that starts with `# ` is a comment; `#tag` is an entry.
            for line in text.lines() {
                if !(line.starts_with("# ") || line.trim_end() == "#") {
                    push(&mut out, line);
                }
            }
        }
        Source::Cspell => {
            let ext = std::path::Path::new(path)
                .extension()
                .map(std::ffi::OsStr::to_ascii_lowercase);
            if ext.is_some_and(|e| e == "yml" || e == "yaml") {
                return Err(TermsError::Parse {
                    path: path.to_owned(),
                    message: TermsText::CspellYaml.to_string(),
                });
            }
            let cfg: Cspell = serde_json::from_str(&strip_json_comments(text)).map_err(|e| {
                TermsError::Parse {
                    path: path.to_owned(),
                    message: e.to_string(),
                }
            })?;
            for w in cfg.words.iter().chain(&cfg.ignore_words) {
                push(&mut out, w);
            }
        }
        Source::Codespell => {
            for line in text.lines() {
                let line = line.trim();
                let section = line.starts_with('[') && line.ends_with(']');
                if line.starts_with('#') || line.starts_with(';') || section {
                    continue;
                }
                if let Some((key, list)) = line.split_once('=') {
                    if key.trim() == "ignore-words-list" {
                        let list = unquote(path, list.trim())?;
                        list.split(',').for_each(|w| push(&mut out, w));
                    }
                } else {
                    push(&mut out, line);
                }
            }
        }
    }
    Ok(out)
}

/// Strip one pair of matching quotes from an `ignore-words-list` value, as in
/// `pyproject.toml`. An unbalanced quote or a TOML array is an error, so the
/// import stops before it changes anything.
fn unquote<'a>(path: &str, value: &'a str) -> Result<&'a str, TermsError> {
    let bad = |message: TermsText| TermsError::Parse {
        path: path.to_owned(),
        message: message.to_string(),
    };
    if value.starts_with('[') {
        return Err(bad(TermsText::IgnoreWordsArray));
    }
    for q in ['"', '\''] {
        if value.starts_with(q) || value.ends_with(q) {
            return value
                .strip_prefix(q)
                .and_then(|v| v.strip_suffix(q))
                .ok_or_else(|| bad(TermsText::IgnoreWordsQuote));
        }
    }
    Ok(value)
}
