//! Lexical path handling: normalise and display with redaction.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// The marker that replaces the directory part of a path outside the project root.
pub const EXTERNAL: &str = "<external>";

/// Resolve `.` and `..` without touching the filesystem.
///
/// - `..` removes the preceding normal component.
/// - `..` at the root is dropped, so `/..` is `/` and an absolute path never escapes the root.
/// - A relative path keeps the `..` it cannot resolve: `../a` stays `../a`, and `a/../..` is `..`.
/// - An empty result is `.`.
///
/// Symlinks are not followed, so the result can differ from the real location.
#[must_use]
pub fn lexical_normalize(path: &Path) -> PathBuf {
    let mut stack: Vec<Component<'_>> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match stack.last() {
                Some(Component::Normal(_)) => {
                    stack.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => stack.push(component),
            },
            other => stack.push(other),
        }
    }
    if stack.is_empty() {
        return PathBuf::from(".");
    }
    stack.iter().map(|c| c.as_os_str()).collect()
}

/// Show `path` for a message, log or report without leaking where the project lives.
///
/// This is the redaction of docs/contract/policies.md section 1.7. Both paths are normalised
/// lexically first. Then:
///
/// - a path under `root` shows relative to it, with `/` separators; the root itself is `.`;
/// - a relative path that stays below its own start shows as given, because it is already
///   root-relative;
/// - any other path shows as `<external>/<file name>`, or `<external>` when it has no file name.
///   That covers an absolute path outside the root and a relative path that climbs out with `..`.
#[must_use]
pub fn display_under_root(path: &Path, root: &Path) -> String {
    let path = lexical_normalize(path);
    let root = lexical_normalize(root);
    let rel = if path.is_absolute() == root.is_absolute() {
        path.strip_prefix(&root).ok().map(Path::to_path_buf)
    } else if path.is_relative() {
        Some(path.clone())
    } else {
        None
    };
    if let Some(rel) = rel
        && !matches!(rel.components().next(), Some(Component::ParentDir))
    {
        return join_slash(&rel);
    }
    match path.file_name() {
        Some(name) => format!("{EXTERNAL}/{}", name.to_string_lossy()),
        None => EXTERNAL.to_owned(),
    }
}

fn join_slash(rel: &Path) -> String {
    let parts: Vec<OsString> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_owned()),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        return ".".to_owned();
    }
    parts
        .iter()
        .map(|p| p.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_cases() {
        for (input, want) in [
            ("/a/./b/../c", "/a/c"),
            ("/..", "/"),
            ("/../../a", "/a"),
            ("/a/../..", "/"),
            ("a/b/..", "a"),
            ("a/..", "."),
            ("../a", "../a"),
            ("a/../..", ".."),
            ("../../a/..", "../.."),
            ("", "."),
            ("./.", "."),
            ("a//b/", "a/b"),
        ] {
            assert_eq!(
                lexical_normalize(Path::new(input)),
                Path::new(want),
                "{input}"
            );
        }
    }

    #[test]
    fn display_cases() {
        let root = "/work/proj";
        for (path, want) in [
            ("/work/proj/docs/a.md", "docs/a.md"),
            ("/work/proj", "."),
            ("/work/proj/./docs/../a.md", "a.md"),
            ("/etc/passwd", "<external>/passwd"),
            ("/work/other/a.md", "<external>/a.md"),
            ("/work/proj/../secret/k.md", "<external>/k.md"),
            ("/work/projx/a.md", "<external>/a.md"),
            ("/", "<external>"),
            ("docs/a.md", "docs/a.md"),
            ("../a.md", "<external>/a.md"),
            ("..", "<external>"),
            ("a/../../b/c.md", "<external>/c.md"),
        ] {
            assert_eq!(
                display_under_root(Path::new(path), Path::new(root)),
                want,
                "{path}"
            );
        }
    }

    #[test]
    fn display_with_relative_root() {
        assert_eq!(
            display_under_root(Path::new("a/b.md"), Path::new("a")),
            "b.md"
        );
        assert_eq!(
            display_under_root(Path::new("c/b.md"), Path::new("a")),
            "<external>/b.md"
        );
    }
}
