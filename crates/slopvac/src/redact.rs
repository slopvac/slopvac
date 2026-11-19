//! Redaction helpers for logs, panics and error chains
//! (docs/contract/policies.md, 1.4).

use std::path::Path;

use slopvac_fs::display_physical;

/// The name shown for standard input everywhere.
pub const STDIN_NAME: &str = "<stdin>";

/// A path safe to log: relative to the project root, `<external>/<file>`
/// outside it.
#[must_use]
pub fn path_for_log(path: &Path, root: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(root) {
        return rel.display().to_string();
    }
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    format!("<external>/{name}")
}

/// How a path appears in a message. A relative path resolves against the working directory
/// and both paths resolve symlinks; the result is root-relative or `<external>/<file>`.
#[must_use]
pub fn shown_path(path: &Path, root: &Path) -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    let absolute = |p: &Path| {
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            cwd.join(p)
        }
    };
    display_physical(&absolute(path), &absolute(root))
}

/// Remove the user-info part of every `scheme://user:pass@host` in `text`.
#[must_use]
pub fn strip_url_userinfo(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("://") {
        let (head, tail) = rest.split_at(pos + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c == '/' || c == '?' || c == '#' || c.is_whitespace())
            .unwrap_or(tail.len());
        let (authority, after) = tail.split_at(end);
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        out.push_str(host);
        rest = after;
    }
    out.push_str(rest);
    out
}

#[must_use]
/// Replace the project root in `text` so no absolute path leaves the process.
///
/// A root under the working directory becomes a relative path. Any other absolute root
/// becomes `<external>`, and the part below the root stays (policies.md, 1.7).
pub fn redact_root(text: &str, root: &Path) -> String {
    if !root.is_absolute() {
        return text.to_owned();
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    let shown = match root.strip_prefix(&cwd) {
        Ok(rel) if rel.as_os_str().is_empty() => ".".to_owned(),
        Ok(rel) => rel.display().to_string(),
        Err(_) => "<external>".to_owned(),
    };
    let replaced = text.replace(&root.display().to_string(), &shown);
    if shown == "<external>" {
        normalise_after(&replaced, "<external>")
    } else {
        replaced
    }
}

/// Use `/` in the path that follows each `marker`, so output reads the same on every platform.
fn normalise_after(text: &str, marker: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(marker) {
        let (head, tail) = rest.split_at(at + marker.len());
        out.push_str(head);
        let end = tail.find(char::is_whitespace).unwrap_or(tail.len());
        out.push_str(&tail[..end].replace('\\', "/"));
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Redact text that an owner crate has already rendered: URL credentials first, then every
/// absolute path.
///
/// A path starts at a `/` that opens a word (after whitespace, a backtick or a quote). A quoted
/// path ends at its closing quote; an unquoted one runs to the next `: ` or line end, so a path
/// with spaces or quotes stays whole.
/// A `/` inside a word, such as the slashes of `https://host/a`, is not a path.
#[must_use]
pub fn redact_absolute(text: &str, cwd: &Path) -> String {
    let text = strip_url_userinfo(text);
    let mut out = String::with_capacity(text.len());
    let mut previous: Option<char> = None;
    let mut rest = text.as_str();
    while let Some(at) = rest.find('/') {
        let (head, tail) = rest.split_at(at);
        out.push_str(head);
        let before = head.chars().next_back().or(previous);
        let opens = before.is_none_or(|c| c.is_whitespace() || matches!(c, '`' | '"' | '\''));
        if !opens {
            out.push('/');
            previous = Some('/');
            rest = &tail[1..];
            continue;
        }
        // A path that opened with a quote ends at the same quote. An unquoted path ends at
        // `: ` or the line end only, so a quote inside it stays part of the path.
        let closing = before.filter(|c| matches!(c, '`' | '"' | '\''));
        let closing_at = closing.and_then(|c| tail.find(c));
        let end = [tail.find(": "), tail.find('\n'), closing_at]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(tail.len());
        out.push_str(&shown_path(Path::new(&tail[..end]), cwd));
        previous = tail[..end].chars().next_back();
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::path::PathBuf;

    #[rstest]
    #[case::current_dir_prefix("./a.md", "a.md")]
    #[case::folded_parent("a/../b.md", "b.md")]
    #[case::already_rooted("docs/a.md", "docs/a.md")]
    #[case::climbs_out("../x/y.md", "<external>/y.md")]
    #[case::climbs_out_after_folding("a/../../x.md", "<external>/x.md")]
    #[case::absolute_inside("ROOT/docs/a.md", "docs/a.md")]
    #[case::absolute_outside("/elsewhere/dir/a.md", "<external>/a.md")]
    fn shown_path_normalizes_before_classifying(#[case] given: &str, #[case] want: &str) {
        let cwd = std::env::current_dir().unwrap();
        let given = given.replace("ROOT", &cwd.display().to_string());
        assert_eq!(shown_path(Path::new(&given), &cwd), want);
    }

    #[test]
    fn path_inside_root_is_relative() {
        let p = PathBuf::from("/work/proj/docs/a.md");
        assert_eq!(path_for_log(&p, Path::new("/work/proj")), "docs/a.md");
    }

    #[test]
    fn path_outside_root_keeps_only_the_file_name() {
        let p = PathBuf::from("/home/alice/secret/a.md");
        assert_eq!(path_for_log(&p, Path::new("/work/proj")), "<external>/a.md");
    }

    #[test]
    fn userinfo_is_stripped() {
        let cases = [
            (
                "fetch https://user:tok@ghcr.io/v2/x",
                "fetch https://ghcr.io/v2/x",
            ),
            ("https://tok@host", "https://host"),
            ("https://host/a@b", "https://host/a@b"),
            ("no url here", "no url here"),
            (
                "a https://u:p@h1/x and http://u2@h2?q=1",
                "a https://h1/x and http://h2?q=1",
            ),
        ];
        for (input, want) in cases {
            assert_eq!(strip_url_userinfo(input), want);
        }
    }

    #[test]
    fn an_external_root_is_redacted_but_keeps_the_part_below_it() {
        // A real directory, so the root is absolute on every platform.
        let dir = tempfile::tempdir().unwrap();
        let file = dir
            .path()
            .join(".claude")
            .join("skills")
            .join("x")
            .join("SKILL.md");
        let text = format!("updated {}", file.display());
        assert_eq!(
            redact_root(&text, dir.path()),
            "updated <external>/.claude/skills/x/SKILL.md"
        );
    }

    #[test]
    fn a_relative_root_is_left_alone() {
        assert_eq!(
            redact_root("updated conf/AGENTS.md", Path::new("conf")),
            "updated conf/AGENTS.md"
        );
    }

    #[test]
    fn separators_after_the_marker_become_slashes_and_other_text_is_kept() {
        assert_eq!(
            normalise_after(
                r"updated <external>\.claude\skills\SKILL.md (kept\raw)",
                "<external>"
            ),
            r"updated <external>/.claude/skills/SKILL.md (kept\raw)"
        );
    }

    #[test]
    fn url_slashes_are_not_paths_and_credentials_go_first() {
        let cwd = Path::new("/work");
        assert_eq!(
            redact_absolute("bad `https://user:secret@example.com/a` here", cwd),
            "bad `https://example.com/a` here"
        );
        assert_eq!(
            redact_absolute(r#"level = "https://u:p@h""#, cwd),
            r#"level = "https://h""#
        );
    }

    #[test]
    fn an_absolute_path_with_spaces_is_one_path() {
        assert_eq!(
            redact_absolute(
                "cannot read /srv/my dir/x.toml: no such file",
                Path::new("/work")
            ),
            "cannot read <external>/x.toml: no such file"
        );
    }

    #[test]
    fn a_slash_inside_a_word_is_kept() {
        assert_eq!(
            redact_absolute("read/write a/b", Path::new("/work")),
            "read/write a/b"
        );
    }

    #[test]
    fn a_quoted_path_ends_at_its_closing_quote_and_an_unquoted_one_keeps_quotes() {
        let cwd = Path::new("/work");
        assert_eq!(
            redact_absolute(r#"path = "/srv/a b/c.toml" rest"#, cwd),
            r#"path = "<external>/c.toml" rest"#
        );
        assert_eq!(
            redact_absolute(r#"cannot read /srv/a"b/c d/e.toml: gone"#, cwd),
            "cannot read <external>/e.toml: gone"
        );
    }
}
