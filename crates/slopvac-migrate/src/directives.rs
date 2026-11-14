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
    for line in text.split_inclusive('\n') {
        if let Some(marker) = fence_marker(line) {
            match fence {
                None => fence = Some(marker),
                Some((c, n)) if c == marker.0 && marker.1 >= n && is_closing(line) => {
                    fence = None;
                }
                Some(_) => {}
            }
            out.push_str(line);
        } else if fence.is_some() {
            out.push_str(line);
        } else {
            out.push_str(&rewrite_line(line, renames, &mut unknown));
        }
    }
    (out, unknown)
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

fn rewrite_line(line: &str, renames: &RenameTable, unknown: &mut Vec<String>) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    let mut consumed = 0;
    while let Some(i) = rest.find("<!--") {
        let start = consumed + i;
        let ticks = line[..start].bytes().filter(|b| *b == b'`').count();
        let Some(end_rel) = rest[i..].find("-->") else {
            break;
        };
        let end = start + end_rel + 3;
        out.push_str(&line[consumed..start]);
        let comment = &line[start..end];
        if ticks % 2 == 0 && comment.contains("slopvac-allow") {
            out.push_str(&rewrite_comment(comment, renames, unknown));
        } else {
            out.push_str(comment);
        }
        consumed = end;
        rest = &line[end..];
    }
    out.push_str(&line[consumed..]);
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
