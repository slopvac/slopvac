//! Inline directive ids: `<!-- slopvac-allow: rule=<id> reason=<name> -->`.

use crate::RenameTable;

/// Rewrite 2.x rule ids in `slopvac-allow` directives to 3.0 qualified ids.
///
/// Directives inside fenced code or inline code spans are documentation and stay as
/// they are. The second value lists ids that the rename table does not know; those
/// directives are left unchanged.
#[must_use]
pub fn rewrite_directives(text: &str, renames: &RenameTable) -> (String, Vec<String>) {
    let mut out = String::with_capacity(text.len());
    let mut unknown = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut paragraph = String::new();
    for line in text.split_inclusive('\n') {
        let in_fence = fence_step(&mut fence, line);
        let blank = line.trim().is_empty();
        let indented_code = paragraph.is_empty() && is_indented_code(line);
        if in_fence || blank || indented_code {
            out.push_str(&rewrite_paragraph(&paragraph, renames, &mut unknown));
            paragraph.clear();
            out.push_str(line);
        } else {
            paragraph.push_str(line);
        }
    }
    out.push_str(&rewrite_paragraph(&paragraph, renames, &mut unknown));
    (out, unknown)
}

fn is_indented_code(line: &str) -> bool {
    line.starts_with("    ") || line.starts_with('\t')
}

/// Advance the fenced-code state over `line`. Returns true when the line is a fence
/// marker or sits inside a fence.
pub(crate) fn fence_step(fence: &mut Option<(char, usize)>, line: &str) -> bool {
    match (*fence, fence_marker(line)) {
        (None, Some(marker)) => {
            *fence = Some(marker);
            true
        }
        (Some((c, n)), Some(marker)) if c == marker.0 && marker.1 >= n && is_closing(line) => {
            *fence = None;
            true
        }
        (Some(_), _) => true,
        (None, None) => false,
    }
}

pub(crate) fn fence_marker(line: &str) -> Option<(char, usize)> {
    let t = line.trim_start_matches(' ');
    if line.len() - t.len() > 3 {
        return None;
    }
    let c = t.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = t.chars().take_while(|x| *x == c).count();
    (n >= 3).then_some((c, n))
}

fn is_closing(line: &str) -> bool {
    let t = line.trim();
    t.chars().all(|c| c == '`') || t.chars().all(|c| c == '~')
}

/// Byte ranges of the code spans in a paragraph. A span opens at an unescaped
/// backtick run and closes at the next run of the same length, possibly on a later
/// line. Backslashes are literal inside a span, so a closer may follow one. A run
/// with no partner is literal text.
fn code_spans(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    // (start, length, escaped): an odd number of backslashes before a run escapes
    // its first backtick, which is then plain text outside a span.
    let mut runs: Vec<(usize, usize, bool)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let start = i;
            while i < bytes.len() && bytes[i] == b'`' {
                i += 1;
            }
            let backslashes = bytes[..start]
                .iter()
                .rev()
                .take_while(|b| **b == b'\\')
                .count();
            runs.push((start, i - start, backslashes % 2 == 1));
        } else {
            i += 1;
        }
    }
    let mut spans = Vec::new();
    let mut k = 0;
    while k < runs.len() {
        let (mut start, mut len, escaped) = runs[k];
        if escaped {
            start += 1;
            len -= 1;
            if len == 0 {
                k += 1;
                continue;
            }
        }
        if let Some(j) = (k + 1..runs.len()).find(|&j| runs[j].1 == len) {
            spans.push((start, runs[j].0 + len));
            k = j + 1;
        } else {
            k += 1;
        }
    }
    spans
}

fn rewrite_paragraph(text: &str, renames: &RenameTable, unknown: &mut Vec<String>) -> String {
    if text.is_empty() {
        return String::new();
    }
    let spans = code_spans(text);
    let mut out = String::with_capacity(text.len());
    let mut consumed = 0;
    while let Some(i) = text[consumed..].find("<!--") {
        let start = consumed + i;
        let Some(end_rel) = text[start..].find("-->") else {
            break;
        };
        let end = start + end_rel + 3;
        out.push_str(&text[consumed..start]);
        let comment = &text[start..end];
        let quoted = spans.iter().any(|&(a, b)| a <= start && start < b);
        if !quoted && comment.contains("slopvac-allow") {
            out.push_str(&rewrite_comment(comment, renames, unknown));
        } else {
            out.push_str(comment);
        }
        consumed = end;
    }
    out.push_str(&text[consumed..]);
    out
}

fn rewrite_comment(comment: &str, renames: &RenameTable, unknown: &mut Vec<String>) -> String {
    let Some(pos) = comment.find("rule=") else {
        return comment.to_owned();
    };
    let id_start = pos + 5;
    let id_len = comment[id_start..]
        .find(char::is_whitespace)
        .unwrap_or(comment.len() - id_start);
    let raw = &comment[id_start..id_start + id_len];
    let id = raw.strip_suffix("-->").unwrap_or(raw);
    if id.contains(':') {
        return comment.to_owned();
    }
    if let Some(new) = renames.rule(id) {
        format!(
            "{}{}{}",
            &comment[..id_start],
            new,
            &comment[id_start + id.len()..]
        )
    } else {
        unknown.push(id.to_owned());
        comment.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> (String, Vec<String>) {
        rewrite_directives(text, &RenameTable::bundled())
    }

    #[test]
    fn qualifies_a_bare_rule_id() {
        let (out, unknown) = run(
            "<!-- slopvac-allow: rule=prose-format.no-unicode-dash reason=quotation -->\nText.\n",
        );
        assert_eq!(
            out,
            "<!-- slopvac-allow: rule=core:prose-format.no-unicode-dash reason=quotation -->\nText.\n"
        );
        assert_eq!(unknown, Vec::<String>::new());
    }

    #[test]
    fn leaves_fenced_and_inline_examples_alone() {
        let text = "```md\n<!-- slopvac-allow: rule=a.b reason=x -->\n```\nUse `<!-- slopvac-allow: rule=a.b reason=x -->` here.\n";
        assert_eq!(run(text).0, text);
    }

    #[test]
    fn double_backtick_span_holding_a_single_backtick_is_quoted() {
        let text = "Write ``a ` <!-- slopvac-allow: rule=a.b reason=x -->`` here.\n";
        assert_eq!(run(text).0, text);
    }

    #[test]
    fn code_span_that_wraps_across_lines_is_quoted() {
        let text = "Use `the directive\n<!-- slopvac-allow: rule=a.b reason=x --> in` prose.\n";
        assert_eq!(run(text).0, text);
    }

    #[test]
    fn unmatched_backtick_does_not_quote_the_directive() {
        let (out, _) = run("A stray ` tick <!-- slopvac-allow: rule=a.b reason=x -->\n");
        assert_eq!(
            out,
            "A stray ` tick <!-- slopvac-allow: rule=core:a.b reason=x -->\n"
        );
    }

    #[test]
    fn a_blank_line_ends_a_code_span() {
        let (out, _) = run("Open `tick\n\n<!-- slopvac-allow: rule=a.b reason=x --> `\n");
        assert_eq!(
            out,
            "Open `tick\n\n<!-- slopvac-allow: rule=core:a.b reason=x --> `\n"
        );
    }

    #[test]
    fn indented_code_is_left_alone() {
        let text = "    <!-- slopvac-allow: rule=a.b reason=x -->\n";
        assert_eq!(run(text).0, text);
    }

    #[test]
    fn longer_fence_is_not_closed_by_a_shorter_one() {
        let text = "````md\n```\n<!-- slopvac-allow: rule=a.b reason=x -->\n````\n";
        assert_eq!(run(text).0, text);
    }

    #[test]
    fn a_backslash_before_the_closing_backtick_does_not_escape_it() {
        // `a\` is a complete code span, so the directive after it is prose.
        let (out, _) = run("`a\\` x <!-- slopvac-allow: rule=a.b reason=x --> `\n");
        assert_eq!(
            out,
            "`a\\` x <!-- slopvac-allow: rule=core:a.b reason=x --> `\n"
        );
    }

    #[test]
    fn a_backslash_escapes_an_opening_backtick_outside_a_span() {
        let text = "\\`not a span <!-- slopvac-allow: rule=a.b reason=x --> still prose`\n";
        let (out, _) = run(text);
        assert_eq!(
            out,
            "\\`not a span <!-- slopvac-allow: rule=core:a.b reason=x --> still prose`\n"
        );
    }

    #[test]
    fn qualified_ids_stay() {
        let text = "<!-- slopvac-allow: rule=core:a.b reason=x -->\n";
        assert_eq!(run(text).0, text);
    }

    #[test]
    fn id_ending_the_comment_without_reason() {
        let (out, _) = run("<!-- slopvac-allow: rule=a.b-->\n");
        assert_eq!(out, "<!-- slopvac-allow: rule=core:a.b-->\n");
    }

    #[test]
    fn unknown_ids_are_reported_and_kept() {
        let text = "<!-- slopvac-allow: rule=Nope reason=x -->\n";
        let (out, unknown) = run(text);
        assert_eq!(out, text);
        assert_eq!(unknown, vec!["Nope".to_owned()]);
    }

    #[test]
    fn disable_comments_have_no_id_and_stay() {
        let text = "<!-- slopvac-disable -->\ntext\n<!-- slopvac-enable -->\n";
        assert_eq!(run(text).0, text);
    }
}
