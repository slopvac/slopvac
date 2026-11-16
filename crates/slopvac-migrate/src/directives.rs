//! Inline directive ids: `<!-- slopvac-allow: rule=<id> reason=<name> -->`.

use crate::RenameTable;

/// A directive rule id that the rename table does not know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownId {
    /// The 2.x id as written.
    pub id: String,
    /// One-based line of the directive.
    pub line: usize,
}

/// A directive for a rule that 3.0 removed. The rewrite deletes the directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedId {
    /// The 2.x id as written.
    pub id: String,
    /// One-based line of the directive.
    pub line: usize,
}

/// Ids found while rewriting directives.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    /// Ids that the rename table does not know; those directives stay unchanged.
    pub unknown: Vec<UnknownId>,
    /// Ids of removed rules; those directives are deleted.
    pub removed: Vec<RemovedId>,
}

/// Rewrite 2.x rule ids in `slopvac-allow` directives to 3.0 qualified ids.
///
/// Directives inside fenced code or inline code spans are documentation and stay as
/// they are. The second value lists ids that the rename table does not know (those
/// directives are left unchanged) and ids of removed rules (those directives are
/// deleted).
#[must_use]
pub fn rewrite_directives(text: &str, renames: &RenameTable) -> (String, Found) {
    let mut out = String::with_capacity(text.len());
    let mut found = Found::default();
    let mut fence: Option<(char, usize)> = None;
    let mut paragraph = String::new();
    // One-based line of the first line in `paragraph`.
    let mut start = 1;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        let in_fence = fence_step(&mut fence, line);
        let blank = line.trim().is_empty();
        let indented_code = paragraph.is_empty() && is_indented_code(line);
        if in_fence || blank || indented_code {
            out.push_str(&rewrite_paragraph(&paragraph, start, renames, &mut found));
            paragraph.clear();
            out.push_str(line);
        } else {
            if paragraph.is_empty() {
                start = index + 1;
            }
            paragraph.push_str(line);
        }
    }
    out.push_str(&rewrite_paragraph(&paragraph, start, renames, &mut found));
    (out, found)
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

fn rewrite_paragraph(
    text: &str,
    start_line: usize,
    renames: &RenameTable,
    found: &mut Found,
) -> String {
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
            let line = start_line + text[..start].matches('\n').count();
            if let Some(new) = rewrite_comment(comment, line, renames, found) {
                out.push_str(&new);
            } else {
                // A deleted directive that filled its line takes the line with it.
                let alone = out.is_empty() || out.ends_with('\n');
                if alone && text[end..].starts_with('\n') {
                    consumed = end + 1;
                    continue;
                }
            }
        } else {
            out.push_str(comment);
        }
        consumed = end;
    }
    out.push_str(&text[consumed..]);
    out
}

fn rewrite_comment(
    comment: &str,
    line: usize,
    renames: &RenameTable,
    found: &mut Found,
) -> Option<String> {
    let Some(pos) = comment.find("rule=") else {
        return Some(comment.to_owned());
    };
    let id_start = pos + 5;
    let id_len = comment[id_start..]
        .find(char::is_whitespace)
        .unwrap_or(comment.len() - id_start);
    let raw = &comment[id_start..id_start + id_len];
    let id = raw.strip_suffix("-->").unwrap_or(raw);
    if id.contains(':') {
        return Some(comment.to_owned());
    }
    if renames.is_removed(id) {
        found.removed.push(RemovedId {
            id: id.to_owned(),
            line,
        });
        None
    } else if let Some(new) = renames.rule(id) {
        Some(format!(
            "{}{}{}",
            &comment[..id_start],
            new,
            &comment[id_start + id.len()..]
        ))
    } else {
        found.unknown.push(UnknownId {
            id: id.to_owned(),
            line,
        });
        Some(comment.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> (String, Vec<UnknownId>) {
        let (out, found) = rewrite_directives(text, &RenameTable::bundled());
        (out, found.unknown)
    }

    #[test]
    fn a_directive_for_a_removed_rule_is_deleted_and_reported() {
        let text = "Intro
<!-- slopvac-allow: rule=ste-sentences.vertical-list-item-punctuation reason=x -->
Body <!-- slopvac-allow: rule=ste-words.inconsistent-term-for-same-thing reason=x --> end.
";
        let (out, found) = rewrite_directives(text, &RenameTable::bundled());
        assert_eq!(
            out,
            "Intro
Body  end.
"
        );
        assert_eq!(
            found.removed,
            vec![
                RemovedId {
                    id: "ste-sentences.vertical-list-item-punctuation".to_owned(),
                    line: 2
                },
                RemovedId {
                    id: "ste-words.inconsistent-term-for-same-thing".to_owned(),
                    line: 3
                },
            ]
        );
        assert_eq!(found.unknown, Vec::<UnknownId>::new());
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
        assert_eq!(unknown, Vec::<UnknownId>::new());
    }

    #[test]
    fn leaves_fenced_and_inline_examples_alone() {
        let text = "```md\n<!-- slopvac-allow: rule=orwell.not-un reason=x -->\n```\nUse `<!-- slopvac-allow: rule=orwell.not-un reason=x -->` here.\n";
        assert_eq!(run(text).0, text);
    }

    #[test]
    fn double_backtick_span_holding_a_single_backtick_is_quoted() {
        let text = "Write ``a ` <!-- slopvac-allow: rule=orwell.not-un reason=x -->`` here.\n";
        assert_eq!(run(text).0, text);
    }

    #[test]
    fn code_span_that_wraps_across_lines_is_quoted() {
        let text =
            "Use `the directive\n<!-- slopvac-allow: rule=orwell.not-un reason=x --> in` prose.\n";
        assert_eq!(run(text).0, text);
    }

    #[test]
    fn unmatched_backtick_does_not_quote_the_directive() {
        let (out, _) = run("A stray ` tick <!-- slopvac-allow: rule=orwell.not-un reason=x -->\n");
        assert_eq!(
            out,
            "A stray ` tick <!-- slopvac-allow: rule=core:orwell.not-un reason=x -->\n"
        );
    }

    #[test]
    fn a_blank_line_ends_a_code_span() {
        let (out, _) = run("Open `tick\n\n<!-- slopvac-allow: rule=orwell.not-un reason=x --> `\n");
        assert_eq!(
            out,
            "Open `tick\n\n<!-- slopvac-allow: rule=core:orwell.not-un reason=x --> `\n"
        );
    }

    #[test]
    fn indented_code_is_left_alone() {
        let text = "    <!-- slopvac-allow: rule=orwell.not-un reason=x -->\n";
        assert_eq!(run(text).0, text);
    }

    #[test]
    fn longer_fence_is_not_closed_by_a_shorter_one() {
        let text = "````md\n```\n<!-- slopvac-allow: rule=orwell.not-un reason=x -->\n````\n";
        assert_eq!(run(text).0, text);
    }

    #[test]
    fn a_backslash_before_the_closing_backtick_does_not_escape_it() {
        // `a\` is a complete code span, so the directive after it is prose.
        let (out, _) = run("`a\\` x <!-- slopvac-allow: rule=orwell.not-un reason=x --> `\n");
        assert_eq!(
            out,
            "`a\\` x <!-- slopvac-allow: rule=core:orwell.not-un reason=x --> `\n"
        );
    }

    #[test]
    fn a_backslash_escapes_an_opening_backtick_outside_a_span() {
        let text =
            "\\`not a span <!-- slopvac-allow: rule=orwell.not-un reason=x --> still prose`\n";
        let (out, _) = run(text);
        assert_eq!(
            out,
            "\\`not a span <!-- slopvac-allow: rule=core:orwell.not-un reason=x --> still prose`\n"
        );
    }

    #[test]
    fn qualified_ids_stay() {
        let text = "<!-- slopvac-allow: rule=core:orwell.not-un reason=x -->\n";
        assert_eq!(run(text).0, text);
    }

    #[test]
    fn id_ending_the_comment_without_reason() {
        let (out, _) = run("<!-- slopvac-allow: rule=orwell.not-un-->\n");
        assert_eq!(out, "<!-- slopvac-allow: rule=core:orwell.not-un-->\n");
    }

    #[test]
    fn unknown_ids_are_reported_with_their_line_and_kept() {
        let text = "Intro\n\n<!-- slopvac-allow: rule=prose-craft.no-such-rule reason=x -->\n\
                    text\n<!-- slopvac-allow: rule=orwell.gone reason=x -->\n";
        let (out, unknown) = run(text);
        assert_eq!(out, text);
        assert_eq!(
            unknown,
            vec![
                UnknownId {
                    id: "prose-craft.no-such-rule".to_owned(),
                    line: 3
                },
                UnknownId {
                    id: "orwell.gone".to_owned(),
                    line: 5
                },
            ]
        );
    }

    #[test]
    fn disable_comments_have_no_id_and_stay() {
        let text = "<!-- slopvac-disable -->\ntext\n<!-- slopvac-enable -->\n";
        assert_eq!(run(text).0, text);
    }
}
