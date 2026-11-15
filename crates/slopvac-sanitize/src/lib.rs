//! One sanitiser for plugin-origin text (threat model O1 to O4 and P15).
//!
//! Text from a plugin, a registry, a cache or a lock file is hostile. Before it
//! reaches a terminal, a log or a report, pass it through the function for its
//! destination:
//!
//! | Destination | Function |
//! |---|---|
//! | Terminal or log, several lines | [`terminal`] |
//! | Terminal or log, one line | [`line()`] |
//! | Markdown | [`markdown`] |
//! | HTML text or attribute | [`html`] |
//! | SARIF, `CodeClimate` and other JSON text fields | [`structured`] |
//! | A link in any report | [`https_uri`] |
//! | A GitHub Actions annotation message | [`workflow_data`] |
//! | A GitHub Actions annotation property | [`workflow_property`] |
//!
//! Every function first neutralises unsafe characters: C0 controls (except
//! newline and tab where a function keeps them), DEL, C1 controls, format
//! characters (bidirectional overrides and isolates, zero-width characters,
//! the soft hyphen), default-ignorable code points, line and paragraph
//! separators, private use and noncharacters. Each one becomes a visible
//! `\u{..}` escape, which keeps the evidence. An escape sequence such as
//! `ESC [ 31 m` or an OSC 8 link loses its introducer (`ESC`, `CSI`, `OSC`,
//! `BEL`, `ST`), so the terminal sees plain text. The set comes from the
//! Unicode Character Database [`UNICODE_VERSION`] through
//! `tools/gen_tables.py`; the Python SDK's `escape_text` in
//! `packages/slopvac-plugin` covers the same set.
//!
//! The functions are total and never panic. They do not truncate. The caller
//! bounds the length.

use std::fmt::Write as _;

mod tables;

pub use tables::UNICODE_VERSION;

/// True for a character that can forge or hide terminal output.
///
/// This holds for the C0 controls (newline and tab included), DEL, the C1
/// controls and every other code point in the generated table.
#[must_use]
pub fn is_unsafe(c: char) -> bool {
    let code = u32::from(c);
    tables::UNSAFE
        .binary_search_by(|&(low, high)| {
            if high < code {
                std::cmp::Ordering::Less
            } else if low > code {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

fn push_escape(out: &mut String, c: char) {
    let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
}

fn map(text: &str, mut each: impl FnMut(char, &mut String)) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        each(c, &mut out);
    }
    out
}

/// Escapes a command introducer that starts a line. The GitHub Actions runner
/// reads a line that starts with `::` as a workflow command, and one that
/// starts with `##[` as a legacy command, after any leading blanks.
fn guard_line_starts(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let rest = line.trim_start_matches([' ', '\t']);
        let blanks = &line[..line.len() - rest.len()];
        out.push_str(blanks);
        if let Some(tail) = rest.strip_prefix("::") {
            out.push_str("\\u{3a}:");
            out.push_str(tail);
        } else if let Some(tail) = rest.strip_prefix("##[") {
            out.push_str("\\u{23}#[");
            out.push_str(tail);
        } else {
            out.push_str(rest);
        }
    }
    out
}

/// Makes text safe to write to a terminal or a log, keeping newline and tab.
///
/// Every other unsafe character becomes `\u{..}`. A line that starts with `::`
/// or `##[` (after blanks) gets its first character escaped, so a CI log never
/// reads it as a workflow command (O4).
///
/// ```
/// assert_eq!(slopvac_sanitize::terminal("a\u{1b}[31m\nb"), "a\\u{1b}[31m\nb");
/// ```
#[must_use]
pub fn terminal(text: &str) -> String {
    guard_line_starts(&map(text, |c, out| {
        if is_unsafe(c) && c != '\n' && c != '\t' {
            push_escape(out, c);
        } else {
            out.push(c);
        }
    }))
}

/// Like [`terminal`], for a value that must stay on one line: newline and tab
/// are escaped too.
///
/// ```
/// assert_eq!(slopvac_sanitize::line("a\nb\tc"), "a\\u{a}b\\u{9}c");
/// ```
#[must_use]
pub fn line(text: &str) -> String {
    guard_line_starts(&map(text, |c, out| {
        if is_unsafe(c) {
            push_escape(out, c);
        } else {
            out.push(c);
        }
    }))
}

/// Makes text safe for a string field in SARIF, `CodeClimate` or another JSON
/// format. The serialiser still has to encode the string. Newline and tab stay,
/// since JSON encodes them.
#[must_use]
pub fn structured(text: &str) -> String {
    map(text, |c, out| {
        if is_unsafe(c) && c != '\n' && c != '\t' {
            push_escape(out, c);
        } else {
            out.push(c);
        }
    })
}

/// Makes text safe to place in Markdown: unsafe characters are escaped, and
/// every Markdown or HTML control character gets a backslash, so no link,
/// image, emphasis, heading, list, table, code span or raw HTML can form.
/// Newline and tab stay.
///
/// ```
/// assert_eq!(
///     slopvac_sanitize::markdown("[x](javascript:alert(1)) <script>"),
///     "\\[x\\]\\(javascript:alert\\(1\\)\\) \\<script\\>",
/// );
/// ```
#[must_use]
pub fn markdown(text: &str) -> String {
    const SPECIAL: &str = "\\`*_{}[]()<>#+-.!|~&=\"'";
    map(text, |c, out| {
        if is_unsafe(c) && c != '\n' && c != '\t' {
            let _ = write!(out, "\\\\u\\{{{:x}\\}}", u32::from(c));
        } else if SPECIAL.contains(c) {
            out.push('\\');
            out.push(c);
        } else {
            out.push(c);
        }
    })
}

/// Makes text safe for HTML element content and quoted attribute values.
/// Unsafe characters are escaped, then `&`, `<`, `>`, `"` and `'` become
/// character references. Newline and tab stay.
///
/// ```
/// assert_eq!(
///     slopvac_sanitize::html("<script>x</script>"),
///     "&lt;script&gt;x&lt;/script&gt;",
/// );
/// ```
#[must_use]
pub fn html(text: &str) -> String {
    map(text, |c, out| match c {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        '\'' => out.push_str("&#39;"),
        c if is_unsafe(c) && c != '\n' && c != '\t' => push_escape(out, c),
        c => out.push(c),
    })
}

/// Returns `uri` when it is safe to emit as a link, otherwise `None`.
///
/// Only `https` passes (O3): the scheme is `https://`, a host follows, and the
/// rest is printable ASCII without characters that break out of an attribute
/// or a Markdown link (`"`, `<`, `>`, `\`, `` ` ``, `^`, `{`, `|`, `}`). User
/// information (`@` in the authority) is refused, because it disguises the host.
/// A caller that needs a non-ASCII link percent-encodes it first.
///
/// ```
/// assert!(slopvac_sanitize::https_uri("https://example.com/a?b=c").is_some());
/// assert!(slopvac_sanitize::https_uri("javascript:alert(1)").is_none());
/// ```
#[must_use]
pub fn https_uri(uri: &str) -> Option<&str> {
    let rest = uri.strip_prefix("https://")?;
    let printable = |b: u8| b.is_ascii_graphic() && !b"\"<>\\`^{|}".contains(&b);
    if !rest.bytes().all(printable) {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    Some(uri)
}

/// Escapes the message of a GitHub Actions workflow command (an annotation):
/// `%` becomes `%25`, CR `%0D`, LF `%0A` and `::` `%3A%3A`, after
/// [`structured`]-style escaping of the other unsafe characters.
///
/// The runner decodes `%25`, `%0D` and `%0A` back when it shows the message.
/// It does not decode `%3A`, so a message with `::` shows the literal `%3A%3A`.
///
/// ```
/// assert_eq!(
///     slopvac_sanitize::workflow_data("x\n::error::y %0A"),
///     "x%0A%3A%3Aerror%3A%3Ay %250A",
/// );
/// ```
#[must_use]
pub fn workflow_data(text: &str) -> String {
    map(text, |c, out| match c {
        '%' => out.push_str("%25"),
        '\r' => out.push_str("%0D"),
        '\n' => out.push_str("%0A"),
        c if is_unsafe(c) && c != '\t' => push_escape(out, c),
        c => out.push(c),
    })
    .replace("::", "%3A%3A")
}

/// Escapes a property of a workflow command (`title`, `file`): as
/// [`workflow_data`], and `,` becomes `%2C` and `:` becomes `%3A`, since a
/// property ends at a comma or the closing `::`.
#[must_use]
pub fn workflow_property(text: &str) -> String {
    workflow_data(text).replace(':', "%3A").replace(',', "%2C")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Code points chosen from the Unicode standard, not from the table.
    const UNSAFE_SAMPLES: &[(u32, &str)] = &[
        (0x00, "NUL"),
        (0x07, "BEL"),
        (0x08, "BS"),
        (0x0d, "CR"),
        (0x1b, "ESC"),
        (0x1f, "US"),
        (0x7f, "DEL"),
        (0x80, "first C1"),
        (0x9b, "CSI"),
        (0x9c, "ST"),
        (0x9d, "OSC"),
        (0x9f, "last C1"),
        (0xad, "soft hyphen"),
        (0x61c, "arabic letter mark"),
        (0x200b, "zero-width space"),
        (0x200e, "left-to-right mark"),
        (0x2028, "line separator"),
        (0x2029, "paragraph separator"),
        (0x202a, "left-to-right embedding"),
        (0x202e, "right-to-left override"),
        (0x2060, "word joiner"),
        (0x2066, "left-to-right isolate"),
        (0x2069, "pop directional isolate"),
        (0x3164, "hangul filler"),
        (0xfe0f, "variation selector 16"),
        (0xfeff, "byte order mark"),
        (0xfff9, "interlinear annotation anchor"),
        (0xe0041, "tag latin capital letter a"),
        (0xe0100, "variation selector 17"),
        (0xe000, "private use"),
        (0xfdd0, "noncharacter"),
        (0xffff, "noncharacter"),
        (0x0010_ffff, "last noncharacter"),
    ];

    #[test]
    fn unsafe_code_points_are_flagged() {
        for &(code, what) in UNSAFE_SAMPLES {
            let c = char::from_u32(code).unwrap();
            assert!(is_unsafe(c), "{code:#x} {what}");
        }
    }

    #[test]
    fn ordinary_text_is_not_flagged() {
        for c in "Zürich – ok ✓ 日本語 😀 \u{a0}~".chars() {
            assert!(!is_unsafe(c), "{c:?}");
        }
    }

    #[test]
    fn the_table_is_sorted_and_disjoint() {
        for w in tables::UNSAFE.windows(2) {
            assert!(w[0].1 + 1 < w[1].0, "{w:?}");
        }
        assert!(tables::UNSAFE.iter().all(|&(low, high)| low <= high));
    }

    #[test]
    fn terminal_keeps_newline_and_tab_and_escapes_the_rest() {
        let got = terminal("a\nb\tc\u{1b}[31m\u{9b}\u{7f}\u{202e}x\r\u{7}");
        assert_eq!(
            got,
            "a\nb\tc\\u{1b}[31m\\u{9b}\\u{7f}\\u{202e}x\\u{d}\\u{7}"
        );
    }

    #[test]
    fn an_osc_8_link_loses_its_introducers() {
        let hostile = "\u{1b}]8;;https://evil.example\u{7}click\u{1b}]8;;\u{7}";
        assert_eq!(
            terminal(hostile),
            "\\u{1b}]8;;https://evil.example\\u{7}click\\u{1b}]8;;\\u{7}"
        );
        let c1 = "\u{9d}8;;https://evil.example\u{9c}x";
        assert_eq!(terminal(c1), "\\u{9d}8;;https://evil.example\\u{9c}x");
    }

    #[test]
    fn safe_text_passes_unchanged_everywhere_it_has_no_syntax() {
        for f in [terminal, line, structured] {
            assert_eq!(f("Zürich – ok ✓ 日本語 😀"), "Zürich – ok ✓ 日本語 😀");
        }
    }

    #[test]
    fn line_escapes_newline_and_tab() {
        assert_eq!(line("a\nb\tc"), "a\\u{a}b\\u{9}c");
    }

    #[test]
    fn a_line_that_starts_a_command_is_escaped_at_its_start() {
        let cases = [
            ("::error::x", "\\u{3a}:error::x"),
            (
                "ok\n::set-output name=a::b",
                "ok\n\\u{3a}:set-output name=a::b",
            ),
            ("  \t::warning::x", "  \t\\u{3a}:warning::x"),
            ("##[error]x", "\\u{23}#[error]x"),
            ("a\n##[add-mask]x", "a\n\\u{23}#[add-mask]x"),
            ("mid :: line", "mid :: line"),
            ("a:b", "a:b"),
        ];
        for (input, want) in cases {
            assert_eq!(terminal(input), want, "{input:?}");
        }
    }

    #[test]
    fn a_command_hidden_behind_a_newline_escape_stays_inert_in_one_line_mode() {
        // `line` escapes the newline, so the text is a single line that starts with "ok".
        assert_eq!(line("ok\n::error::x"), "ok\\u{a}::error::x");
    }

    #[test]
    fn markdown_neutralises_links_html_and_headings() {
        let cases = [
            (
                "[x](javascript:alert(1))",
                "\\[x\\]\\(javascript:alert\\(1\\)\\)",
            ),
            (
                "<script>alert(1)</script>",
                "\\<script\\>alert\\(1\\)\\</script\\>",
            ),
            ("![i](https://e/x.png)", "\\!\\[i\\]\\(https://e/x\\.png\\)"),
            ("# title\n- item\n1. one", "\\# title\n\\- item\n1\\. one"),
            (
                "a|b `c` *d* _e_ &amp; ~f~",
                "a\\|b \\`c\\` \\*d\\* \\_e\\_ \\&amp; \\~f\\~",
            ),
            ("back\\slash", "back\\\\slash"),
            ("a\u{202e}b", "a\\\\u\\{202e\\}b"),
        ];
        for (input, want) in cases {
            assert_eq!(markdown(input), want, "{input:?}");
        }
    }

    #[test]
    fn html_escapes_markup_quotes_and_unsafe_characters() {
        let cases = [
            ("<script>x</script>", "&lt;script&gt;x&lt;/script&gt;"),
            ("a & b", "a &amp; b"),
            (r#"" onmouseover='x'"#, "&quot; onmouseover=&#39;x&#39;"),
            (
                "<a href=\"javascript:x\">",
                "&lt;a href=&quot;javascript:x&quot;&gt;",
            ),
            ("a\u{202e}b\u{1b}", "a\\u{202e}b\\u{1b}"),
            ("a\nb\tc", "a\nb\tc"),
        ];
        for (input, want) in cases {
            assert_eq!(html(input), want, "{input:?}");
        }
    }

    #[test]
    fn structured_keeps_json_markup_for_the_serialiser() {
        assert_eq!(
            structured("<b>\u{1b}[0m\u{202e}\n"),
            "<b>\\u{1b}[0m\\u{202e}\n"
        );
    }

    #[test]
    fn only_clean_https_links_pass() {
        let ok = [
            "https://example.com",
            "https://example.com/a/b?c=d#e",
            "https://example.com:8443/x",
            "https://[::1]/x",
        ];
        for uri in ok {
            assert_eq!(https_uri(uri), Some(uri), "{uri}");
        }
        let bad = [
            "javascript:alert(1)",
            "data:text/html;base64,AAAA",
            "http://example.com",
            "HTTPS://example.com",
            "file:///etc/passwd",
            "//example.com",
            "https://",
            "https:///path",
            "https://user@evil.example/",
            "https://good.example@evil.example/",
            "https://example.com/a b",
            "https://example.com/\"onclick=x",
            "https://example.com/<x>",
            "https://example.com/\u{202e}x",
            "https://example.com/\nx",
            "https://exämple.com",
            "",
        ];
        for uri in bad {
            assert_eq!(https_uri(uri), None, "{uri:?}");
        }
    }

    #[test]
    fn workflow_data_leaves_no_command_and_survives_a_runner_decode() {
        let cases = [
            ("x\n::error::y", "x%0A%3A%3Aerror%3A%3Ay"),
            ("%0A", "%250A"),
            ("a\r\nb", "a%0D%0Ab"),
            ("100% sure", "100%25 sure"),
            ("a\u{1b}[0m\u{202e}", "a\\u{1b}[0m\\u{202e}"),
            ("tab\tkept", "tab\tkept"),
        ];
        for (input, want) in cases {
            assert_eq!(workflow_data(input), want, "{input:?}");
        }
    }

    #[test]
    fn workflow_property_also_escapes_colon_and_comma() {
        assert_eq!(workflow_property("a:b,c\n%"), "a%3Ab%2Cc%0A%25");
    }

    #[test]
    fn no_function_emits_an_unsafe_character_for_any_scalar_value() {
        for code in 0..=0x0010_ffff_u32 {
            let Some(c) = char::from_u32(code) else {
                continue;
            };
            let text = format!("<{c}>");
            for (name, out) in [
                ("terminal", terminal(&text)),
                ("line", line(&text)),
                ("structured", structured(&text)),
                ("markdown", markdown(&text)),
                ("html", html(&text)),
                ("workflow_data", workflow_data(&text)),
                ("workflow_property", workflow_property(&text)),
            ] {
                let bad = out
                    .chars()
                    .find(|&o| is_unsafe(o) && o != '\n' && o != '\t');
                assert_eq!(bad, None, "{name} on {code:#x}");
            }
            for out in [line(&text), workflow_data(&text)] {
                assert!(!out.contains(['\n', '\r']), "{code:#x}");
            }
        }
    }
}
