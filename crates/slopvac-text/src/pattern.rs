use pcre2::bytes::{Regex, RegexBuilder};

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
        .build(pattern)
        .unwrap_or_else(|e| panic!("built-in pattern {pattern:?} must compile: {e}"))
}

/// Byte ranges of every non-overlapping match.
pub(crate) fn find_all(re: &Regex, text: &str) -> Vec<(usize, usize)> {
    re.find_iter(text.as_bytes())
        .map(|m| {
            let m = m.expect("built-in pattern must not hit a match limit");
            (m.start(), m.end())
        })
        .collect()
}

/// Whether `re` matches at the very start of `text`. Patterns passed here begin with `^`.
pub(crate) fn matches_start(re: &Regex, text: &str) -> bool {
    re.is_match(text.as_bytes())
        .expect("built-in pattern must not hit a match limit")
}

/// Replace each match with the result of `f(range)`.
pub(crate) fn replace_all(
    re: &Regex,
    text: &str,
    mut f: impl FnMut(usize, usize) -> String,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (s, e) in find_all(re, text) {
        out.push_str(&text[last..s]);
        out.push_str(&f(s, e));
        last = e;
    }
    out.push_str(&text[last..]);
    out
}
