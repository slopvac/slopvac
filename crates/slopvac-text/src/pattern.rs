use pcre2::bytes::{Regex, RegexBuilder};

use crate::rewrite::compat;

/// A pattern failed at match time, for example by exhausting PCRE2's match limit.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("pattern match failed: {0}")]
pub struct TextError(String);

impl From<pcre2::Error> for TextError {
    fn from(e: pcre2::Error) -> Self {
        Self(e.to_string())
    }
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

    fn no_spans() -> Vec<std::ops::Range<usize>> {
        Vec::new()
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

    #[test]
    fn non_word_is_the_complement_of_the_python_word_class() {
        // U+0903 is a spacing mark and U+200D a joiner: word characters for Python, so `\W`
        // skips them, though native PCRE2 `\W` takes U+0903.
        assert_eq!(spans(r"\W", "a\u{903}b-"), [5..6]);
        assert_eq!(spans(r"\W", "a\u{200d}b-"), [5..6]);
    }

    #[test]
    fn a_not_boundary_ignores_marks_and_joiners_like_python() {
        assert_eq!(spans(r"a\B\x{903}", "a\u{903}"), [0..4]);
    }

    #[test]
    fn a_negated_class_with_a_non_word_escape_keeps_marks_and_joiners() {
        // `[^\W_]` is a word character that is not an underscore.
        assert_eq!(spans(r"[^\W_]+", "a\u{903}_b\u{200d}"), [0..4, 5..9]);
    }

    #[test]
    fn a_positive_class_with_a_non_word_escape_mixes_with_other_members() {
        assert_eq!(spans(r"[\W\d]+", "a\u{903}-5x"), [4..6]);
        assert_eq!(spans(r"[\Wx]+", "\u{903}-x\u{200d}"), [3..5]);
    }

    #[test]
    fn a_non_word_class_alone_is_the_complement() {
        assert_eq!(spans(r"[\W]+", "a\u{903}-,b"), [4..6]);
        assert_eq!(spans(r"[^\W]+", "-a\u{903}-"), [1..5]);
    }

    #[test]
    fn a_caret_left_first_by_a_removed_non_word_escape_stays_literal() {
        assert_eq!(spans(r"[\W^]+", "a^-b"), [1..3]);
    }

    #[test]
    fn quoted_text_is_matched_literally() {
        assert_eq!(spans(r"\Q\W\E", r"a\W"), [1..3]);
        assert_eq!(spans(r"\Q\w\E", r"a\w"), [1..3]);
        assert_eq!(spans(r"\Q[\Ex\b", "[x "), [0..2]);
    }

    #[test]
    fn an_unterminated_quote_runs_to_the_end() {
        assert_eq!(spans(r"a\Q\w", r"a\w"), [0..3]);
    }

    #[test]
    fn a_bracket_in_a_comment_does_not_open_a_class() {
        assert_eq!(spans(r"(?#[)a\x{903}\b", "a\u{903} b"), [0..4]);
        assert_eq!(
            spans("(?x) a\\x{903} \\b # [ comment", "a\u{903} b"),
            [0..4]
        );
    }

    #[test]
    fn extended_mode_ends_with_its_group() {
        // Outside the group `#` is a literal and `[\w]` a class again.
        assert_eq!(spans("(?x: a #c\n)#[\\w]", "a#\u{903}"), [0..5]);
    }

    #[test]
    fn extended_mode_set_inline_lasts_to_the_end_of_the_group() {
        assert_eq!(spans("(?:(?x) a # [\n)b", "ab"), [0..2]);
    }

    #[test]
    fn a_control_escape_does_not_open_a_class() {
        assert_eq!(spans(r"\c[\x{903}\b", "\u{1b}\u{903} "), [0..4]);
    }

    #[test]
    fn a_closing_bracket_first_in_a_class_is_a_member() {
        assert_eq!(spans(r"[]\w]+", "]a\u{903}-"), [0..5]);
        assert_eq!(spans(r"[^]\w]+", "]a-,"), [2..4]);
    }

    #[test]
    fn a_posix_class_inside_a_class_is_kept() {
        assert_eq!(spans(r"[[:digit:]\w]+", "1a\u{903}-"), [0..5]);
    }

    #[test]
    fn a_marker_verb_argument_is_opaque_in_extended_mode() {
        // The `#` belongs to the marker name, not to a comment, so `\w` still rewrites.
        assert_eq!(spans(r"(?x)(*MARK:#tag)\w+", "a\u{903}-"), [0..4]);
        assert_eq!(spans(r"(?x)(*MARK:[a)\w+", "a\u{903}-"), [0..4]);
    }

    #[test]
    fn a_verb_without_an_argument_is_opaque() {
        assert_eq!(spans(r"(*UCP)\w+", "a\u{903}-"), [0..4]);
        assert_eq!(spans(r"a(*COMMIT)\w+", "a\u{903}-"), [0..4]);
    }

    #[test]
    fn an_alphabetic_assertion_still_opens_a_group() {
        assert_eq!(spans("(*pla:(?x) a # c\n)a\\w", "a\u{903}b"), [0..4]);
        assert_eq!(spans(r"(*positive_lookahead:\w)\w", "a"), [0..1]);
        assert_eq!(spans(r"(*atomic:\w+)-", "a\u{903}-"), [0..5]);
    }

    #[test]
    fn named_groups_and_assertions_open_groups() {
        for open in [
            "(?<n>", "(?P<n>", "(?'n'", "(?|", "(?>", "(?=", "(?!", "(?<=", "(?<!",
        ] {
            let pattern = format!("(?x: {open}a # c\n){close}\\w", close = ")");
            assert!(Pattern::new(&pattern, false).is_ok(), "{pattern}");
        }
    }

    #[test]
    fn removing_a_non_word_escape_does_not_merge_octal_members() {
        // `\0` then `0` are two members; joined they would read as the octal `\00`.
        assert_eq!(spans(r"[\0\W0]+", "0-a"), [0..2]);
        assert_eq!(spans(r"[^\0\W0]+", "0-a"), [2..3]);
        assert_eq!(spans(r"[\1\W1]+", "1\u{1}-a"), [0..3]);
    }

    #[test]
    fn numeric_escapes_in_a_class_keep_their_meaning_next_to_a_non_word_escape() {
        assert_eq!(spans(r"[\101\W]+", "A-b"), [0..2]);
        assert_eq!(spans(r"[\x41\W1]+", "A1-b"), [0..3]);
        assert_eq!(spans(r"[\x4\W1]+", "\u{4}1-b"), [0..3]);
        assert_eq!(spans(r"[\x\W1]+", "\u{0}1-b"), [0..3]);
    }

    #[test]
    fn an_opening_bracket_does_not_join_a_posix_class_across_a_removed_escape() {
        assert_eq!(spans(r"[[\W:alpha:]+", "[:a]"), [0..4]);
        assert_eq!(spans(r"[[\W:alpha:]+", "z"), no_spans());
    }

    /// PCRE2 10.43 added variable-length lookbehind. The rule data relies on it
    /// (`prose-craft.future-tense`), so a build that links an older system library fails
    /// here. `.cargo/config.toml` sets `PCRE2_SYS_STATIC=1` and the workspace pins
    /// `pcre2-sys` so the vendored PCRE2 is linked (slopvac-8l4.3.20).
    #[test]
    fn the_linked_pcre2_supports_a_bounded_variable_length_lookbehind() {
        let pattern = Pattern::new(r"(?<=a{1,3}|bc)x", false).unwrap_or_else(|e| {
            panic!(
                "PCRE2 is older than 10.43 ({e}); build with PCRE2_SYS_STATIC=1, see \
                 .cargo/config.toml and slopvac-8l4.3.20"
            )
        });
        assert_eq!(pattern.find_all("aax bcx dx").unwrap(), [2..3, 6..7]);
    }
}
