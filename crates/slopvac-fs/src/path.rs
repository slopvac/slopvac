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
    let rel = if root == Path::new(".") {
        // An empty or `.` root is the working directory: a relative path is already below it.
        path.is_relative().then(|| path.clone())
    } else if path.is_absolute() == root.is_absolute() {
        match path.strip_prefix(&root) {
            Ok(rel) => Some(rel.to_path_buf()),
            Err(_) => None,
        }
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

/// Show an absolute `path` like [`display_under_root`], but classify it by where it really is.
///
/// A lexical fold cancels `link/..` even when `link` is a symlink, so it can place a path in
/// the project that lives elsewhere. This function first resolves the longest existing
/// ancestor of each path with the operating system, appends the components that do not exist
/// yet, and compares the two physical paths. A path or root that cannot be resolved with
/// certainty (a relative input, or an error other than "not found") is external, so the
/// result never shows a directory it cannot place.
#[must_use]
pub fn display_physical(path: &Path, root: &Path) -> String {
    match (resolve_physical(path), resolve_physical(root)) {
        (Some(path), Some(root)) => display_under_root(&path, &root),
        _ => match lexical_normalize(path).file_name() {
            Some(name) => format!("{EXTERNAL}/{}", name.to_string_lossy()),
            None => EXTERNAL.to_owned(),
        },
    }
}

/// Resolve symlinks in the longest existing ancestor of an absolute `path`, then append the
/// rest lexically. Returns `None` for a relative path or an unexpected I/O error.
fn resolve_physical(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let mut existing = path.to_path_buf();
    let mut tail: Vec<OsString> = Vec::new();
    loop {
        match existing.canonicalize() {
            Ok(real) => {
                let mut joined = real;
                joined.extend(tail.iter().rev());
                return Some(lexical_normalize(&joined));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let name = existing.file_name().map(OsString::from);
                // `..` and `.` have no file name: keep them in the tail as written.
                let component = match name {
                    Some(name) => name,
                    None => existing.components().next_back()?.as_os_str().to_owned(),
                };
                tail.push(component);
                if !existing.pop() {
                    return None;
                }
            }
            Err(_) => return None,
        }
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

    #[cfg(unix)]
    #[test]
    fn display_physical_sees_through_a_symlinked_parent() {
        let dir = tempfile::tempdir().unwrap();
        let top = dir.path().canonicalize().unwrap();
        let proj = top.join("base").join("proj");
        let sub = top.join("secret").join("sub");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::create_dir_all(&sub).unwrap();
        std::os::unix::fs::symlink(&sub, top.join("base").join("link")).unwrap();
        let via_link = top.join("base/link/../proj/private/missing.md");
        // Lexically this is inside the project; physically it is below `secret`.
        assert_eq!(display_under_root(&via_link, &proj), "private/missing.md");
        assert_eq!(display_physical(&via_link, &proj), "<external>/missing.md");
        assert_eq!(
            display_physical(&proj.join("docs/./a.md"), &proj),
            "docs/a.md"
        );
        assert_eq!(
            display_physical(Path::new("relative.md"), &proj),
            "<external>/relative.md"
        );
    }

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
    fn display_with_cwd_root() {
        for root in ["", "."] {
            for (path, want) in [
                ("docs/a.md", "docs/a.md"),
                (".", "."),
                ("./docs/../a.md", "a.md"),
                ("../a.md", "<external>/a.md"),
                ("/abs/a.md", "<external>/a.md"),
            ] {
                assert_eq!(
                    display_under_root(Path::new(path), Path::new(root)),
                    want,
                    "{root:?} {path}"
                );
            }
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
