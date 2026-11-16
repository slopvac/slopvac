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
//! # The escape form
//!
//! Every function first neutralises unsafe characters: C0 controls (except
//! newline and tab where a function keeps them), DEL, C1 controls, format
//! characters (bidirectional overrides and isolates, zero-width characters,
//! the soft hyphen), default-ignorable code points, line and paragraph
//! separators, private use and noncharacters. Each one becomes a visible
//! `\u{..}` escape, which keeps the evidence. An escape sequence such as
//! `ESC [ 31 m` or an OSC 8 link loses its introducer (`ESC`, `CSI`, `OSC`,
//! `BEL`, `ST`), so the terminal sees plain text.
//!
//! A literal backslash becomes `\\`. Without that rule the text `\u{202e}`
//! would look like an escaped control character. With it, a reader can decode
//! the output without doubt: `\\` is a backslash and `\u{hex}` is a code point
//! that the sanitiser removed.
//!
//! A consequence: **pass raw text, once.** Text that already went through one of
//! these functions is escaped again (`\u{1b}` becomes `\\u{1b}`). That is safe
//! and still unambiguous, but it reads badly, so a type that stores text keeps
//! it raw and escapes it where it formats it.
//!
//! The character set comes from the Unicode Character Database
//! [`UNICODE_VERSION`] through `tools/gen_tables.py`. The Python SDK's
//! `escape_text` in `packages/slopvac-plugin` uses a table that the same script
//! generates from the same data, and a test compares the two.
//!
//! # GitHub Actions workflow commands (O4)
//!
//! The runner reads two command forms (`src/Runner.Common/ActionCommand.cs` in
//! `actions/runner`):
//!
//! - `TryParseV2` trims leading white space of a line, then needs the line to
//!   start with `::` and to hold a second `::` (`::name props::data`).
//! - `TryParse` (legacy) searches the line for `##[` **anywhere**, then reads
//!   `name props]data` up to the next `]`.
//!
//! So [`terminal`] and [`line()`] escape the first `:` of a `::` that starts a
//! line (after any white space, Unicode white space included) and every `##[`
//! wherever it occurs. Each becomes `\u{3a}` or `\u{23}`. A `::` in the middle
//! of a line is harmless to the runner and stays, since `std::vec` and
//! `key::value` are common.
//!
//! The functions are total and never panic. They do not truncate. The caller
//! bounds the length.
#![expect(
    clippy::let_underscore_must_use,
    reason = "fmt::Write for String never returns an error"
)]
#![warn(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::string_slice
)]

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

/// The shared first step: a backslash is doubled, an unsafe character becomes
/// `\u{..}`, and newline and tab stay when `keep_layout` holds.
fn visible(text: &str, keep_layout: bool) -> String {
    map(text, |c, out| {
        if c == '\\' {
            out.push_str("\\\\");
        } else if is_unsafe(c) && !(keep_layout && (c == '\n' || c == '\t')) {
            push_escape(out, c);
        } else {
            out.push(c);
        }
    })
}

/// Neutralises the runner's command introducers in already escaped text.
fn guard_commands(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let rest = line.trim_start_matches(char::is_whitespace);
        out.push_str(line.strip_suffix(rest).unwrap_or_default());
        match rest.strip_prefix("::") {
            Some(tail) => {
                out.push_str("\\u{3a}:");
                out.push_str(tail);
            }
            None => out.push_str(rest),
        }
    }
    out.replace("##[", "\\u{23}#[")
}

/// Makes text safe to write to a terminal or a log, keeping newline and tab.
///
/// Every other unsafe character becomes `\u{..}` and a backslash becomes `\\`.
/// The first `:` of a `::` that starts a line and every `##[` get escaped, so a
/// CI log never reads the text as a workflow command (see the module
/// documentation).
///
/// ```
/// assert_eq!(slopvac_sanitize::terminal("a\u{1b}[31m\nb"), "a\\u{1b}[31m\nb");
/// ```
#[must_use]
pub fn terminal(text: &str) -> String {
    guard_commands(&visible(text, true))
}

/// Like [`terminal`], for a value that must stay on one line: newline and tab
/// are escaped too.
///
/// ```
/// assert_eq!(slopvac_sanitize::line("a\nb\tc"), "a\\u{a}b\\u{9}c");
/// ```
#[must_use]
pub fn line(text: &str) -> String {
    guard_commands(&visible(text, false))
}

/// Makes text safe for a string field in SARIF, `CodeClimate` or another JSON
/// format. The serialiser still has to encode the string. Newline and tab stay,
/// since JSON encodes them.
#[must_use]
pub fn structured(text: &str) -> String {
    visible(text, true)
}

/// Makes text safe to place in Markdown: unsafe characters are escaped, and
/// every Markdown or HTML control character gets a backslash, so no link,
/// image, emphasis, heading, list, table, code span or raw HTML can form.
/// Newline and tab stay. Rendered, the text reads as [`structured`] would show
/// it.
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
    map(&visible(text, true), |c, out| {
        if SPECIAL.contains(c) {
            out.push('\\');
        }
        out.push(c);
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
    map(&visible(text, true), |c, out| match c {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        '\'' => out.push_str("&#39;"),
        c => out.push(c),
    })
}

/// Returns `uri` as a link target that is safe in HTML attributes, Markdown
/// destinations and SARIF, or `None` when it is not a clean https link.
///
/// Only `https` passes (O3): the scheme is `https://`, a host follows, and the
/// rest is printable ASCII without `"`, `<`, `>`, `\`, `` ` ``, `^`, `{`, `|`
/// and `}`. User information (`@` in the authority) is refused, because it
/// disguises the host. The characters `'`, `(` and `)` are legal in a URI but
/// end an attribute or a Markdown destination, so they come back
/// percent-encoded (`%27`, `%28`, `%29`). A caller that needs a non-ASCII link
/// percent-encodes it first.
///
/// ```
/// assert_eq!(
///     slopvac_sanitize::https_uri("https://example.com/a_(b)"),
///     Some("https://example.com/a_%28b%29".to_owned()),
/// );
/// assert!(slopvac_sanitize::https_uri("javascript:alert(1)").is_none());
/// ```
#[must_use]
pub fn https_uri(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("https://")?;
    let printable = |b: u8| b.is_ascii_graphic() && !b"\"<>\\`^{|}".contains(&b);
    if !rest.bytes().all(printable) {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    Some(
        uri.replace('\'', "%27")
            .replace('(', "%28")
            .replace(')', "%29"),
    )
}

/// Escapes the message of a GitHub Actions workflow command (an annotation):
/// `%` becomes `%25`, CR `%0D`, LF `%0A` and `::` `%3A%3A`; a backslash and the
/// other unsafe characters are escaped as in [`structured`], and `##[` is
/// neutralised as in [`terminal`].
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
        '\\' => out.push_str("\\\\"),
        c if is_unsafe(c) && c != '\t' => push_escape(out, c),
        c => out.push(c),
    })
    .replace("::", "%3A%3A")
    .replace("##[", "\\u{23}#[")
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
        (0x2065, "reserved default-ignorable"),
        (0x2066, "left-to-right isolate"),
        (0x2069, "pop directional isolate"),
        (0x3164, "hangul filler"),
        (0xfe0f, "variation selector 16"),
        (0xfeff, "byte order mark"),
        (0xfff0, "reserved default-ignorable"),
        (0xfff9, "interlinear annotation anchor"),
        (0xe0041, "tag latin capital letter a"),
        (0xe0080, "reserved tag-block code point"),
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
    fn ordinary_and_merely_unassigned_text_is_not_flagged() {
        // U+0378 is unassigned in every Unicode version so far, and U+1F600 is an emoji.
        for c in "Zürich – ok ✓ 日本語 😀 \u{a0}~\u{378}".chars() {
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

    /// The oracle is the vendored UCD extract, parsed here with code that shares
    /// nothing with the generator or the table.
    #[test]
    fn the_table_equals_the_vendored_ucd_data_for_every_code_point() {
        const UNICODE_DATA: &str = include_str!("../tools/ucd/17.0.0/UnicodeData.subset.txt");
        const DERIVED: &str = include_str!("../tools/ucd/17.0.0/DerivedCoreProperties.subset.txt");
        let mut want = vec![false; 0x11_0000];
        let mut open = 0_usize;
        for l in UNICODE_DATA.lines() {
            let mut f = l.split(';');
            let code = usize::from_str_radix(f.next().unwrap(), 16).unwrap();
            let name = f.next().unwrap();
            if name.ends_with(", First>") {
                open = code;
            } else if name.ends_with(", Last>") {
                want[open..=code].fill(true);
            } else {
                want[code] = true;
            }
        }
        for l in DERIVED.lines() {
            let span = l.split(';').next().unwrap().trim();
            let (lo, hi) = span.split_once("..").unwrap_or((span, span));
            let lo = usize::from_str_radix(lo, 16).unwrap();
            let hi = usize::from_str_radix(hi, 16).unwrap();
            want[lo..=hi].fill(true);
        }
        want[0xfdd0..=0xfdef].fill(true);
        for plane in 0..=16 {
            want[plane << 16 | 0xfffe] = true;
            want[plane << 16 | 0xffff] = true;
        }
        let bad: Vec<String> = (0..0x11_0000_u32)
            .filter_map(char::from_u32)
            .filter(|&c| is_unsafe(c) != want[usize::try_from(u32::from(c)).unwrap()])
            .map(|c| format!("{:#x}", u32::from(c)))
            .collect();
        assert!(bad.is_empty(), "{bad:?}");
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
    fn a_literal_backslash_is_doubled_so_it_cannot_pose_as_an_escape() {
        let cases = [
            ("\\u{202e}", "\\\\u{202e}"),
            ("a\\b", "a\\\\b"),
            ("\\\u{1b}", "\\\\\\u{1b}"),
            ("\\\\", "\\\\\\\\"),
            ("C:\\dir", "C:\\\\dir"),
        ];
        for f in [terminal, line, structured] {
            for (input, want) in cases {
                assert_eq!(f(input), want, "{input:?}");
            }
        }
    }

    /// Cases follow `ActionCommand.TryParseV2` and `TryParse` of actions/runner:
    /// V2 trims leading white space and needs a leading `::`; the legacy parser
    /// finds `##[` anywhere in the line.
    #[test]
    fn a_line_that_starts_a_command_is_escaped_at_its_start() {
        let cases = [
            ("::error::x", "\\u{3a}:error::x"),
            (
                "ok\n::set-output name=a::b",
                "ok\n\\u{3a}:set-output name=a::b",
            ),
            ("  \t::warning::x", "  \t\\u{3a}:warning::x"),
            // C# TrimStart also trims these; Rust's white space set matches.
            ("\u{a0}::error::x", "\u{a0}\\u{3a}:error::x"),
            ("\u{3000}::error::x", "\u{3000}\\u{3a}:error::x"),
            ("\u{2003}\u{a0}::error::x", "\u{2003}\u{a0}\\u{3a}:error::x"),
            ("mid :: line", "mid :: line"),
            ("std::vec and a:b", "std::vec and a:b"),
        ];
        for (input, want) in cases {
            assert_eq!(terminal(input), want, "{input:?}");
        }
    }

    #[test]
    fn a_legacy_command_is_escaped_wherever_it_occurs() {
        let cases = [
            ("##[error]x", "\\u{23}#[error]x"),
            ("prefix ##[error]x", "prefix \\u{23}#[error]x"),
            ("a ##[b] ##[c]", "a \\u{23}#[b] \\u{23}#[c]"),
            ("###[error]x", "#\\u{23}#[error]x"),
            ("## [not] a command", "## [not] a command"),
        ];
        for f in [terminal, line] {
            for (input, want) in cases {
                assert_eq!(f(input), want, "{input:?}");
            }
        }
    }

    #[test]
    fn a_legacy_command_after_a_newline_is_escaped_in_terminal_text() {
        assert_eq!(terminal("a\n##[add-mask]x"), "a\n\\u{23}#[add-mask]x");
    }

    #[test]
    fn a_command_behind_a_newline_stays_inert_in_one_line_mode() {
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
            // Rendered, a literal backslash reads `\\`, an escaped control reads `\u{202e}`.
            ("back\\slash", "back\\\\\\\\slash"),
            ("a\u{202e}b", "a\\\\u\\{202e\\}b"),
            ("\\u{202e}", "\\\\\\\\u\\{202e\\}"),
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
            ("\\u{202e}", "\\\\u{202e}"),
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
            assert_eq!(https_uri(uri).as_deref(), Some(uri), "{uri}");
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
    fn a_returned_link_cannot_end_an_attribute_or_a_markdown_destination() {
        let cases = [
            ("https://e.example/a'b", "https://e.example/a%27b"),
            ("https://e.example/a)b", "https://e.example/a%29b"),
            ("https://e.example/a(b", "https://e.example/a%28b"),
            (
                "https://e.example/w/Rust_(language)",
                "https://e.example/w/Rust_%28language%29",
            ),
            (
                "https://e.example/x')onerror=alert(1)",
                "https://e.example/x%27%29onerror=alert%281%29",
            ),
        ];
        for (input, want) in cases {
            let got = https_uri(input).unwrap();
            assert_eq!(got, want, "{input}");
            assert!(!got.contains(['\'', '(', ')', '"', '<', '>', ' ']));
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
            ("a ##[error]b", "a \\u{23}#[error]b"),
            ("\\u{202e}", "\\\\u{202e}"),
        ];
        for (input, want) in cases {
            assert_eq!(workflow_data(input), want, "{input:?}");
        }
    }

    #[test]
    fn workflow_property_also_escapes_colon_and_comma() {
        assert_eq!(workflow_property("a:b,c\n%"), "a%3Ab%2Cc%0A%25");
    }

    /// Every scalar value, through every function. Debug builds make this slow, so
    /// it runs on request: `cargo test -p slopvac-sanitize -- --ignored`.
    #[test]
    #[ignore = "exhaustive sweep, several seconds in a debug build"]
    fn no_function_emits_an_unsafe_character_for_any_scalar_value() {
        for code in 0..=0x0010_ffff_u32 {
            if let Some(c) = char::from_u32(code) {
                check_no_unsafe_output(c);
            }
        }
    }

    fn check_no_unsafe_output(c: char) {
        let code = u32::from(c);
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

    /// The default-run version of the sweep: the edges of every table range, the
    /// ASCII and Latin-1 blocks, and the start of each plane.
    #[test]
    fn no_function_emits_an_unsafe_character_at_the_edges_of_the_table() {
        let mut codes: Vec<u32> = (0..0x300).collect();
        for &(low, high) in tables::UNSAFE {
            codes.extend([low.saturating_sub(1), low, high, high + 1]);
        }
        codes.extend((0..=16).map(|plane| plane << 16));
        for code in codes {
            if let Some(c) = char::from_u32(code) {
                check_no_unsafe_output(c);
            }
        }
    }
}
