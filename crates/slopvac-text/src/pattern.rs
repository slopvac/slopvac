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

/// Rewrite `\w` and `\b` into explicit classes (see [`WORD`]).
fn compat(pattern: &str) -> String {
    let boundary = format!("(?:(?<![{WORD}])(?=[{WORD}])|(?<=[{WORD}])(?![{WORD}]))");
    pattern
        .replace("[\\w", &format!("[{WORD}"))
        .replace("\\w", &format!("[{WORD}]"))
        .replace("\\b", &boundary)
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
fn build(pattern: &str, caseless: bool) -> Regex {
    RegexBuilder::new()
        .utf(true)
        .ucp(true)
        .caseless(caseless)
        .build(&compat(pattern))
        .unwrap_or_else(|e| panic!("built-in pattern {pattern:?} must compile: {e}"))
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
}
