//! Escaping for XML, HTML and GitHub workflow commands.

/// Escape text for XML and HTML content and attributes.
///
/// Characters that XML 1.0 forbids become U+FFFD, so the output stays well formed.
pub(crate) fn xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 || c == '\u{fffe}' || c == '\u{ffff}' => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
    out
}

/// Escape a GitHub workflow-command message (`::error ::message`).
pub(crate) fn github_data(text: &str) -> String {
    text.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// Escape a GitHub workflow-command property value.
pub(crate) fn github_property(text: &str) -> String {
    github_data(text).replace(':', "%3A").replace(',', "%2C")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_escapes_markup_and_control_characters() {
        assert_eq!(
            xml("<a href=\"x\">&'\u{1}</a>"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;\u{fffd}&lt;/a&gt;"
        );
    }

    #[test]
    fn github_escapes_per_the_workflow_command_rules() {
        assert_eq!(github_data("50%\r\nnext"), "50%25%0D%0Anext");
        assert_eq!(github_property("a:b,c%"), "a%3Ab%2Cc%25");
    }
}
