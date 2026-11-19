//! Managed steering blocks between `<!-- slopvac:begin -->` and `<!-- slopvac:end -->`.

use slopvac_parse::marker_lines;

const BEGIN: &str = "<!-- slopvac:begin -->";
const END: &str = "<!-- slopvac:end -->";

/// Replace the managed block with `block`, the text `slopvac setup` writes.
///
/// The caller passes the block in because `slopvac-agents` depends on this crate; it
/// cannot be imported here without a cycle. Pass `slopvac_agents::content::steering_block`
/// output so migrate and setup write the same bytes.
///
/// Returns `None` when the text has no managed block. Markers shown inside fenced
/// or indented code do not count; `slopvac setup` uses the same scanner. A begin
/// without a matching end, or the reverse, is left alone and reported through `Err`
/// so the user repairs it.
pub fn rewrite_steering(text: &str, block: &str) -> Result<Option<String>, String> {
    let found = marker_lines(text, &[BEGIN, END]);
    match found.as_slice() {
        [] => Ok(None),
        [begin, end] if begin.marker == BEGIN && end.marker == END => {
            let (start, end) = (&begin.start, &end.end);
            let newline = if text[*start..*end].contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            let mut block = block.replace('\n', newline);
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

    const NEW: &str = "<!-- slopvac:begin -->\n## Slopvac\n\nnew\n<!-- slopvac:end -->\n";

    fn rewrite(text: &str) -> Result<Option<String>, String> {
        rewrite_steering(text, NEW)
    }

    #[test]
    fn replaces_the_block_and_keeps_the_rest() {
        let text = format!("# Title\n\n{BEGIN}\nold\n{END}\n\nTail\n");
        let out = rewrite(&text).unwrap().unwrap();
        assert!(out.starts_with("# Title\n\n<!-- slopvac:begin -->\n## Slopvac\n\nnew"));
        assert!(!out.contains("old"));
        assert!(out.ends_with("<!-- slopvac:end -->\n\nTail\n"));
    }

    #[test]
    fn no_markers_is_none() {
        assert_eq!(rewrite("plain\n").unwrap(), None);
    }

    #[test]
    fn markers_in_fenced_code_are_ignored() {
        let text = format!("```\n{BEGIN}\n{END}\n```\n");
        assert_eq!(rewrite(&text).unwrap(), None);
    }

    #[test]
    fn a_fence_like_line_with_an_info_string_does_not_close_a_fence() {
        let text = format!("```\n```md\n{BEGIN}\nx\n{END}\n```\n");
        assert_eq!(rewrite(&text).unwrap(), None);
    }

    #[test]
    fn unpaired_marker_is_an_error() {
        assert!(rewrite(&format!("{BEGIN}\nx\n")).is_err());
    }

    #[test]
    fn migrating_twice_changes_nothing() {
        let text = format!("{BEGIN}\nold\n{END}\n");
        let once = rewrite(&text).unwrap().unwrap();
        assert_eq!(rewrite(&once).unwrap().unwrap(), once);
    }
}
