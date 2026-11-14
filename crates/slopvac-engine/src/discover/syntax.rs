//! Small pieces of path and pattern syntax shared by globs and excludes.

use std::path::{Component, Path, PathBuf};

/// Index just past the `]` that closes the character class opening at `open`
/// (`chars[open] == '['`), or `None` when the class never closes.
///
/// A class is `[`, an optional `!` or `^`, an optional literal `]`, then members
/// up to the next `]`.
pub(super) fn class_end(chars: &[char], open: usize) -> Option<usize> {
    let mut j = open + 1;
    if matches!(chars.get(j), Some('!' | '^')) {
        j += 1;
    }
    if chars.get(j) == Some(&']') {
        j += 1;
    }
    let close = chars.get(j..)?.iter().position(|&c| c == ']')?;
    Some(j + close + 1)
}

/// Collapse `.` and `..` components without touching the file system. A `..`
/// that has nothing to pop stays when the path is relative and vanishes at a
/// root. An empty result becomes `.`.
pub(super) fn lexical_clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let last = out.components().next_back();
                match last {
                    Some(Component::Normal(_)) => {
                        out.pop();
                    }
                    Some(Component::RootDir | Component::Prefix(_)) => {}
                    _ => out.push(".."),
                }
            }
            other => out.push(other),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// Normalise a command-line target.
///
/// A path with `..` that exists is resolved through the file system up to its last
/// `..`, because `link/..` names the parent of the link's *target*. The final
/// components stay as written, so a symlink named as a target remains one. A path
/// without `..`, or one that does not exist, is only cleaned lexically.
pub(super) fn resolve_target(path: &Path) -> PathBuf {
    let components: Vec<_> = path.components().collect();
    let Some(last_parent) = components.iter().rposition(|c| *c == Component::ParentDir) else {
        return lexical_clean(path);
    };
    let prefix: PathBuf = components[..=last_parent].iter().collect();
    let Ok(mut resolved) = prefix.canonicalize() else {
        return lexical_clean(path);
    };
    resolved.extend(&components[last_parent + 1..]);
    lexical_clean(&resolved)
}

/// Expand `{a,b}` alternation into brace-free patterns, nested groups and
/// separators inside alternatives included. `None` means more than `limit`
/// results. Escaped braces and braces inside a character class stay literal; a
/// group with no closing brace is left for the glob parser to reject.
pub(super) fn expand_braces(pattern: &str, limit: usize) -> Option<Vec<String>> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = Vec::new();
    expand(&chars, limit, &mut out).then_some(out)
}

fn expand(chars: &[char], limit: usize, out: &mut Vec<String>) -> bool {
    let Some((open, close, commas)) = first_group(chars) else {
        out.push(chars.iter().collect());
        return out.len() <= limit;
    };
    let mut starts = vec![open + 1];
    starts.extend(commas.iter().map(|c| c + 1));
    let mut ends = commas;
    ends.push(close);
    for (start, end) in starts.into_iter().zip(ends) {
        let mut next: Vec<char> = chars[..open].to_vec();
        next.extend(&chars[start..end]);
        next.extend(&chars[close + 1..]);
        if !expand(&next, limit, out) {
            return false;
        }
    }
    true
}

/// The first top-level `{...}`: its open and close index and its top-level commas.
fn first_group(chars: &[char]) -> Option<(usize, usize, Vec<usize>)> {
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '[' => i = class_end(chars, i).unwrap_or(i + 1),
            '{' => return group_from(chars, i),
            _ => i += 1,
        }
    }
    None
}

fn group_from(chars: &[char], open: usize) -> Option<(usize, usize, Vec<usize>)> {
    let mut depth = 0usize;
    let mut commas = Vec::new();
    let mut i = open;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '[' => i = class_end(chars, i).map_or(i, |end| end - 1),
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((open, i, commas));
                }
            }
            ',' if depth == 1 => commas.push(i),
            _ => {}
        }
        i += 1;
    }
    None
}
