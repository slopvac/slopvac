//! Redaction helpers for logs, panics and error chains
//! (docs/contract/policies.md, 1.4).

use std::path::Path;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
}
