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
    /// One-based lines of comma-list directives. 2.x took one id per directive, so
    /// these were already invalid; they stay as written.
    pub invalid: Vec<usize>,
    /// One-based lines of directives whose id was renamed.
    pub renamed: Vec<usize>,
}

/// Rewrite 2.x rule ids in `slopvac-allow` directives to 3.0 qualified ids.
///
/// Directives inside fenced code or inline code spans are documentation and stay as
/// they are. The second value lists ids that the rename table does not know (those
/// directives are left unchanged) and ids of removed rules (those directives are
/// deleted).
#[must_use]
pub fn rewrite_directives(text: &str, renames: &RenameTable) -> (String, Found) {
    rewrite_with(text, renames, Removed::Delete)
}

/// `text` with each 2.x directive in its 3.0 form, for the before side of the lint.
///
/// A renamed id is qualified as in [`rewrite_directives`]. A directive of a removed rule
/// becomes an empty comment in place of being deleted, so the document keeps its block
/// structure and no other directive covers more text than it does in the file on disk.
pub(crate) fn before_directives(text: &str, renames: &RenameTable) -> String {
    rewrite_with(text, renames, Removed::Blank).0
}

/// What happens to a directive of a removed rule.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Removed {
    /// Delete it, and its line when it stands alone there.
    Delete,
    /// Replace it with [`BLANK_COMMENT`].
    Blank,
}

/// An HTML comment that is no directive.
const BLANK_COMMENT: &str = "<!-- -->";

fn rewrite_with(text: &str, renames: &RenameTable, removed: Removed) -> (String, Found) {
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
            out.push_str(&rewrite_paragraph(
                &paragraph, start, renames, removed, &mut found,
            ));
            paragraph.clear();
            out.push_str(line);
        } else {
            if paragraph.is_empty() {
                start = index + 1;
            }
            paragraph.push_str(line);
        }
    }
    out.push_str(&rewrite_paragraph(
        &paragraph, start, renames, removed, &mut found,
    ));
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
    removed: Removed,
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
        if !quoted && is_directive(comment) {
            let line = start_line + text[..start].matches('\n').count();
            if let Some(new) = rewrite_comment(comment, line, renames, found) {
                out.push_str(&new);
            } else if removed == Removed::Blank {
                out.push_str(BLANK_COMMENT);
            } else {
                // A deleted directive alone on its line takes the whole line with it.
                let line_start = out.rfind('\n').map_or(0, |n| n + 1);
                let rest = &text[end..];
                let line_end = rest.find('\n').unwrap_or(rest.len());
                if out[line_start..].trim().is_empty() && rest[..line_end].trim().is_empty() {
                    out.truncate(line_start);
                    consumed = (end + line_end + 1).min(text.len());
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

/// The text after `slopvac-allow:` when the comment is a directive: it starts with
/// `slopvac-allow:` and its first attribute is `rule=`. Prose that only mentions the
/// directive is not one.
fn directive_body(comment: &str) -> Option<&str> {
    let inner = comment.strip_prefix("<!--")?;
    let inner = inner.strip_suffix("-->").unwrap_or(inner);
    let body = inner.trim_start().strip_prefix("slopvac-allow:")?;
    body.trim_start().starts_with("rule=").then_some(body)
}

fn is_directive(comment: &str) -> bool {
    directive_body(comment).is_some()
}

/// Rewrite the id of one directive. `None` means the rule was removed and the
/// directive should go. A comma list was invalid in 2.x (one id per directive), so it
/// is reported and left as written.
fn rewrite_comment(
    comment: &str,
    line: usize,
    renames: &RenameTable,
    found: &mut Found,
) -> Option<String> {
    let Some(pos) = comment.find("rule=") else {
        return Some(comment.to_owned());
    };
    let value_start = pos + "rule=".len();
    let tail = &comment[value_start..];
    let mut value_len = tail.find(char::is_whitespace).unwrap_or(tail.len());
    if tail[..value_len].ends_with("-->") {
        value_len -= 3;
    }
    let id = &tail[..value_len];
    if id.contains(',') {
        found.invalid.push(line);
        return Some(comment.to_owned());
    }
    if id.contains(':') {
        return Some(comment.to_owned());
    }
    if renames.is_removed(id) {
        found.removed.push(RemovedId {
            id: id.to_owned(),
            line,
        });
        return None;
    }
    if let Some(new) = renames.rule(id) {
        found.renamed.push(line);
        return Some(format!(
            "{}{new}{}",
            &comment[..value_start],
            &tail[value_len..]
        ));
    }
    found.unknown.push(UnknownId {
        id: id.to_owned(),
        line,
    });
    Some(comment.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> (String, Vec<UnknownId>) {
        let (out, found) = rewrite_directives(text, &RenameTable::bundled());
        (out, found.unknown)
    }

    const GONE: &str = "ste-sentences.vertical-list-item-punctuation";
    const GONE2: &str = "ste-words.inconsistent-term-for-same-thing";

    fn rewrite(text: &str) -> (String, Found) {
        rewrite_directives(text, &RenameTable::bundled())
    }

    fn removed_ids(found: &Found) -> Vec<&str> {
        found.removed.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn a_directive_for_a_removed_rule_is_deleted_with_its_line() {
        let text = format!("Intro\n<!-- slopvac-allow: rule={GONE} reason=x -->\nBody\n");
        let (out, found) = rewrite(&text);
        assert_eq!(out, "Intro\nBody\n");
        assert_eq!(
            found.removed,
            vec![RemovedId {
                id: GONE.to_owned(),
                line: 2
            }]
        );
        assert_eq!(found.unknown, Vec::<UnknownId>::new());
    }

    #[test]
    fn an_inline_directive_keeps_the_text_around_it() {
        let text = format!("Body <!-- slopvac-allow: rule={GONE2} reason=x --> end.\n");
        let (out, found) = rewrite(&text);
        assert_eq!(out, "Body  end.\n");
        assert_eq!(removed_ids(&found), vec![GONE2]);
    }

    #[test]
    fn an_indented_or_trailing_space_directive_takes_its_whole_line() {
        let text = format!("A\n   <!-- slopvac-allow: rule={GONE} reason=x -->  \nB\n");
        let (out, _) = rewrite(&text);
        assert_eq!(out, "A\nB\n");
    }

    #[test]
    fn a_comma_list_stays_as_written_and_is_reported() {
        let lists = [
            "prose-format.no-unicode-dash,prose-craft.latinisms".to_owned(),
            format!("{GONE},prose-format.no-unicode-dash"),
            format!("prose-format.no-unicode-dash,{GONE}"),
            format!("core:prose-format.no-unicode-dash,{GONE},prose-craft.latinisms"),
        ];
        for value in lists {
            let text = format!("A\n  <!-- slopvac-allow: rule={value} reason=x -->\nB\n");
            let (out, found) = rewrite(&text);
            assert_eq!(out, text, "{value}");
            assert_eq!(found.invalid, vec![2], "{value}");
            assert_eq!(found.removed, Vec::<RemovedId>::new(), "{value}");
            assert_eq!(found.unknown, Vec::<UnknownId>::new(), "{value}");
        }
    }

    #[test]
    fn a_comment_that_only_mentions_a_removed_rule_stays() {
        let text = format!(
            "<!-- note: the slopvac-allow rule={GONE} comment was removed -->\n<!-- see slopvac-allow: rule={GONE2} -->\n"
        );
        let (out, found) = rewrite(&text);
        assert_eq!(out, text);
        assert_eq!(found, Found::default());
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
