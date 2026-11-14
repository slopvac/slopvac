use pcre2::bytes::{Regex, RegexBuilder};

/// A pattern failed at match time, for example by exhausting PCRE2's match limit.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("pattern match failed: {0}")]
pub struct TextError(String);

impl From<pcre2::Error> for TextError {
    fn from(e: pcre2::Error) -> Self {
        Self(e.to_string())
    }
}

/// The Python `regex` module's `\w`, which follows UTS #18: Alphabetic, marks (M), decimal
/// digits (Nd), connector punctuation (Pc) and `Join_Control`. PCRE2's UCP `\w` is L, N,
/// Mn and Pc, so the built-in patterns spell the class out.
const WORD: &str = r"\p{Alphabetic}\p{M}\p{Nd}\p{Pc}\x{200C}\x{200D}";

/// Rewrite `\w`, `\W`, `\b` and `\B` into explicit classes (see [`WORD`]).
///
/// The walk skips escaped characters, so `\\w` (a literal backslash and `w`) stays as it is,
/// and inside a character class `\b` stays a backspace. After a class-level `\w` a `-` is
/// escaped, because PCRE2 reads `\x{200D}-.` as a range.
fn compat(pattern: &str) -> String {
    let boundary = format!("(?:(?<![{WORD}])(?=[{WORD}])|(?<=[{WORD}])(?![{WORD}]))");
    let not_boundary = format!("(?:(?<![{WORD}])(?![{WORD}])|(?<=[{WORD}])(?=[{WORD}]))");
    let word = format!("[{WORD}]");
    let non_word = format!("[^{WORD}]");
    let mut out = String::with_capacity(pattern.len() + 64);
    let mut chars = pattern.chars().peekable();
    let mut in_class = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' => match (chars.next(), in_class) {
                (Some('w'), true) => {
                    out.push_str(WORD);
                    if chars.next_if_eq(&'-').is_some() {
                        out.push_str("\\-");
                    }
                }
                (Some('w'), false) => out.push_str(&word),
                (Some('W'), false) => out.push_str(&non_word),
                (Some('b'), false) => out.push_str(&boundary),
                (Some('B'), false) => out.push_str(&not_boundary),
                (Some(other), _) => {
                    out.push('\\');
                    out.push(other);
                }
                (None, _) => out.push('\\'),
            },
            '[' if in_class && chars.peek() == Some(&':') => {
                // A POSIX class such as `[:alpha:]` inside a class: copy it through `:]`.
                out.push(c);
                while let Some(n) = chars.next() {
                    out.push(n);
                    if n == ':' && chars.peek() == Some(&']') {
                        out.extend(chars.next());
                        break;
                    }
                }
            }
            '[' if !in_class => {
                in_class = true;
                out.push(c);
                if chars.next_if_eq(&'^').is_some() {
                    out.push('^');
                }
                if chars.next_if_eq(&']').is_some() {
                    out.push_str("\\]");
                }
            }
            ']' if in_class => {
                in_class = false;
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Compile a pattern in UTF and Unicode-property mode, as the Python reference's
/// `regex` module reads it.
pub(crate) fn re(pattern: &str) -> Regex {
    build(pattern, false)
}

/// Same as [`re`], case-insensitive.
pub(crate) fn re_i(pattern: &str) -> Regex {
    build(pattern, true)
}

// STUB(slopvac-8l4.3.25): pcre2 crate defaults; apply the build and limits policy from slopvac-8l4.3.20.
fn try_build(pattern: &str, caseless: bool) -> Result<Regex, pcre2::Error> {
    RegexBuilder::new()
        .utf(true)
        .ucp(true)
        .caseless(caseless)
        .build(&compat(pattern))
}

fn build(pattern: &str, caseless: bool) -> Regex {
    try_build(pattern, caseless)
        .unwrap_or_else(|e| panic!("built-in pattern {pattern:?} must compile: {e}"))
}

/// A pattern did not compile.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("pattern does not compile: {0}")]
pub struct PatternError(String);

/// A PCRE2 pattern compiled in UTF and Unicode-property mode, for rule data.
///
/// `\w`, `\W`, `\b` and `\B` follow the Python `regex` module (UTS #18), as in the
/// built-in patterns. Offsets are UTF-8 byte offsets.
#[derive(Debug)]
pub struct Pattern(Regex);

impl Pattern {
    /// Compile `source`; `caseless` turns on case-insensitive matching.
    pub fn new(source: &str, caseless: bool) -> Result<Self, PatternError> {
        try_build(source, caseless)
            .map(Self)
            .map_err(|e| PatternError(e.to_string()))
    }

    /// Byte ranges of every non-overlapping match, left to right.
    pub fn find_all(&self, text: &str) -> Result<Vec<std::ops::Range<usize>>, TextError> {
        Ok(find_all(&self.0, text)?
            .into_iter()
            .map(|(s, e)| s..e)
            .collect())
    }

    /// Whether the pattern matches anywhere in `text`.
    pub fn is_match(&self, text: &str) -> Result<bool, TextError> {
        Ok(self.0.is_match(text.as_bytes())?)
    }
}

/// Byte ranges of every non-overlapping match.
pub(crate) fn find_all(re: &Regex, text: &str) -> Result<Vec<(usize, usize)>, TextError> {
    re.find_iter(text.as_bytes())
        .map(|m| m.map(|m| (m.start(), m.end())).map_err(TextError::from))
        .collect()
}

/// Whether `re` matches in `text`. Patterns passed here begin with `^`.
pub(crate) fn matches_start(re: &Regex, text: &str) -> Result<bool, TextError> {
    Ok(re.is_match(text.as_bytes())?)
}

/// Replace each match with the result of `f(range)`.
pub(crate) fn replace_all(
    re: &Regex,
    text: &str,
    mut f: impl FnMut(usize, usize) -> Result<String, TextError>,
) -> Result<String, TextError> {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (s, e) in find_all(re, text)? {
        out.push_str(&text[last..s]);
        out.push_str(&f(s, e)?);
        last = e;
    }
    out.push_str(&text[last..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::single_range_in_vec_init)]

    use super::*;

    #[test]
    fn exhausting_the_match_limit_is_an_error_not_a_panic() {
        let re = re(r"^(a|aa)+$");
        let text = format!("{}b", "a".repeat(64));
        assert!(find_all(&re, &text).is_err());
    }

    #[test]
    fn word_class_includes_spacing_marks_like_python() {
        let re = re(r"(?<!\w)x(?!\w)");
        assert_eq!(find_all(&re, "x").unwrap().len(), 1);
        assert_eq!(find_all(&re, "\u{903}x").unwrap(), []);
    }

    fn spans(pattern: &str, text: &str) -> Vec<std::ops::Range<usize>> {
        Pattern::new(pattern, false)
            .unwrap()
            .find_all(text)
            .unwrap()
    }

    #[test]
    fn an_escaped_backslash_before_w_is_not_a_word_class() {
        assert_eq!(spans(r"\\w", r"a\w"), [1..3]);
    }

    #[test]
    fn a_hyphen_after_a_class_word_escape_stays_a_literal() {
        assert_eq!(spans(r"[\w-.]+", "ab-c.d e"), [0..6, 7..8]);
    }

    #[test]
    fn a_word_escape_in_a_class_includes_marks_like_python() {
        assert_eq!(spans(r"[\w]+", "a\u{903}b c"), [0..5, 6..7]);
    }

    #[test]
    fn a_boundary_sits_between_a_mark_and_a_space() {
        assert_eq!(spans(r"a\x{903}\b", "a\u{903} x"), [0..4]);
    }

    #[test]
    fn a_non_boundary_is_the_negation_of_a_boundary() {
        assert_eq!(spans(r"\Bb", "ab b"), [1..2]);
    }

    #[test]
    fn a_pattern_that_does_not_compile_is_an_error() {
        assert!(Pattern::new("(unclosed", false).is_err());
    }
}
