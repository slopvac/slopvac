//! Managed steering blocks between `<!-- slopvac:begin -->` and `<!-- slopvac:end -->`.

use crate::directives::fence_step;

const BEGIN: &str = "<!-- slopvac:begin -->";
const END: &str = "<!-- slopvac:end -->";

// STUB(slopvac-8l4.16.6): fakes the 3.0 steering block. The real text comes from
// `slopvac setup` (slopvac-8l4.11.2).
const BLOCK: &str = "<!-- slopvac:begin -->
## Slopvac

Before writing or reviewing prose, run `slopvac prime` and follow the current
CLI guidance. Use `slopvac rules` and `slopvac explain <rule_id>` for rule
details and named exceptions.

Treat lint exit 2 as incomplete.
<!-- slopvac:end -->
";

/// Replace the managed block with the 3.0 text.
///
/// Returns `None` when the text has no managed block. Markers shown inside fenced
/// code do not count. A begin without a matching end, or the reverse, is left
/// alone and reported through `Err` so the user repairs it.
pub fn rewrite_steering(text: &str) -> Result<Option<String>, String> {
    let mut fence: Option<(char, usize)> = None;
    let mut found: Vec<(&str, usize, usize)> = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if fence_step(&mut fence, line) {
            // fenced text and fence markers are examples
        } else {
            let v = line.trim().trim_start_matches('\u{feff}');
            if v == BEGIN {
                found.push((BEGIN, offset, offset + line.len()));
            } else if v == END {
                found.push((END, offset, offset + line.len()));
            }
        }
        offset += line.len();
    }
    match found.as_slice() {
        [] => Ok(None),
        [(BEGIN, start, _), (END, _, end)] => {
            let newline = if text[*start..*end].contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            let mut block = BLOCK.replace('\n', newline);
            if !text[*start..*end].ends_with('\n') {
                block.truncate(block.trim_end_matches(['\r', '\n']).len());
            }
            let mut out = String::with_capacity(text.len());
            out.push_str(&text[..*start]);
            out.push_str(&block);
            out.push_str(&text[*end..]);
            Ok(Some(out))
        }
        _ => Err("malformed or ambiguous slopvac markers; repair the begin/end pair first".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_the_block_and_keeps_the_rest() {
        let text = format!("# Title\n\n{BEGIN}\nold\n{END}\n\nTail\n");
        let out = rewrite_steering(&text).unwrap().unwrap();
        assert!(out.starts_with("# Title\n\n<!-- slopvac:begin -->\n## Slopvac"));
        assert!(!out.contains("old"));
        assert!(out.ends_with("<!-- slopvac:end -->\n\nTail\n"));
    }

    #[test]
    fn no_markers_is_none() {
        assert_eq!(rewrite_steering("plain\n").unwrap(), None);
    }

    #[test]
    fn markers_in_fenced_code_are_ignored() {
        let text = format!("```\n{BEGIN}\n{END}\n```\n");
        assert_eq!(rewrite_steering(&text).unwrap(), None);
    }

    #[test]
    fn unpaired_marker_is_an_error() {
        assert!(rewrite_steering(&format!("{BEGIN}\nx\n")).is_err());
    }

    #[test]
    fn migrating_twice_changes_nothing() {
        let text = format!("{BEGIN}\nold\n{END}\n");
        let once = rewrite_steering(&text).unwrap().unwrap();
        assert_eq!(rewrite_steering(&once).unwrap().unwrap(), once);
    }
}
