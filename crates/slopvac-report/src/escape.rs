//! Escaping for XML and HTML.

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

/// Escape text for an XML attribute value.
///
/// Parsers normalise literal tabs and newlines in attributes to spaces, so they become
/// character references here and survive a round trip.
pub(crate) fn xml_attr(text: &str) -> String {
    xml(text)
        .replace('\t', "&#9;")
        .replace('\n', "&#10;")
        .replace('\r', "&#13;")
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
    fn attribute_whitespace_becomes_character_references() {
        assert_eq!(xml_attr("a\tb\nc\rd"), "a&#9;b&#10;c&#13;d");
    }
}
