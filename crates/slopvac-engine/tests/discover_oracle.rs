//! Property test: discovery equals a brute-force oracle.
//!
//! The oracle walks every file of a random tree with `std::fs`, matches each path
//! against the glob with `globset` directly (native brace handling, no bounding),
//! and applies a small reference model of gitwildmatch excludes. Discovery bounds
//! its traversal by the pattern, so a bug there shows up as a dropped match.

use std::fs;
use std::path::{Path, PathBuf};

use globset::GlobBuilder;
use proptest::prelude::*;
use slopvac_engine::discover::{DiscoverError, Excludes, Options, Selection, discover};
use tempfile::TempDir;

const DIRS: [&str; 4] = ["a", "b", "c", "pub"];
const FILES: [&str; 4] = ["x.md", "y.txt", "w.rs", "z.md"];
const DIR_PARTS: [&str; 9] = ["a", "b", "c", "*", "**", "{a,b}", "{a,b/c}", "?", "p*"];
const FILE_PARTS: [&str; 6] = ["x.md", "*.md", "*.txt", "*.{md,txt}", "{x,y}.md", "*"];
const EXCLUDES: [&str; 8] = [
    "a/", "x.md", "*.txt", "!a/b/", "b/x.md", "!x.md", "a/b/", "c",
];

/// Directory parts that can never match the decoy directory name `zz`.
#[cfg(unix)]
const NON_MATCHING_PARTS: [&str; 6] = ["a", "b", "c", "{a,b}", "{a,b/c}", "p*"];

fn tree() -> impl Strategy<Value = Vec<(Vec<usize>, usize)>> {
    prop::collection::vec(
        (prop::collection::vec(0..DIRS.len(), 0..=3), 0..FILES.len()),
        1..8,
    )
}

fn glob() -> impl Strategy<Value = (Vec<usize>, usize)> {
    (
        prop::collection::vec(0..DIR_PARTS.len(), 0..=3),
        0..FILE_PARTS.len(),
    )
}

fn excludes() -> impl Strategy<Value = Vec<usize>> {
    prop::collection::vec(0..EXCLUDES.len(), 0..3)
}

fn build(root: &Path, tree: &[(Vec<usize>, usize)]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for (dirs, file) in tree {
        let mut dir = root.to_path_buf();
        for d in dirs {
            dir.push(DIRS[*d]);
        }
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILES[*file]);
        fs::write(&path, "").unwrap();
        files.push(path);
    }
    files.sort();
    files.dedup();
    files
}

/// Reference model of a gitwildmatch pattern list over the small alphabet.
fn reference_excluded(patterns: &[&str], rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    let mut excluded = false;
    for pattern in patterns {
        let (negate, text) = pattern
            .strip_prefix('!')
            .map_or((false, *pattern), |t| (true, t));
        let (dir_only, text) = text.strip_suffix('/').map_or((false, text), |t| (true, t));
        let anchored = text.contains('/');
        let comps: Vec<&str> = text.split('/').collect();
        let name_matches = |pat: &str, name: &str| match pat.strip_prefix('*') {
            Some(suffix) => name.ends_with(suffix),
            None => pat == name,
        };
        // A pattern matches the path or any ancestor directory; a trailing slash
        // matches directories only, so the final component never counts.
        let limit = if dir_only {
            parts.len() - 1
        } else {
            parts.len()
        };
        let hit = if anchored {
            comps.len() <= limit
                && comps
                    .iter()
                    .zip(&parts)
                    .all(|(pat, name)| name_matches(pat, name))
        } else {
            parts[..limit]
                .iter()
                .any(|name| name_matches(comps[0], name))
        };
        if hit {
            excluded = !negate;
        }
    }
    excluded
}

fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}

/// Makes directories unreadable and restores them on drop.
#[cfg(unix)]
struct Locked(Vec<PathBuf>);

#[cfg(unix)]
impl Locked {
    fn lock(dirs: Vec<PathBuf>) -> Self {
        use std::os::unix::fs::PermissionsExt;
        for d in &dirs {
            fs::set_permissions(d, fs::Permissions::from_mode(0o000)).unwrap();
        }
        Self(dirs)
    }

    fn enforced(&self) -> bool {
        self.0.iter().all(|d| fs::read_dir(d).is_err())
    }
}

#[cfg(unix)]
impl Drop for Locked {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        for d in &self.0 {
            let _ = fs::set_permissions(d, fs::Permissions::from_mode(0o755));
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn glob_discovery_matches_the_oracle(
        tree in tree(),
        (dir_parts, file_part) in glob(),
        exclude in excludes(),
        lock in any::<bool>(),
    ) {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let files = build(root, &tree);

        let pattern_rel = dir_parts
            .iter()
            .map(|i| DIR_PARTS[*i])
            .chain([FILE_PARTS[file_part]])
            .collect::<Vec<_>>()
            .join("/");
        let pattern = root.join(&pattern_rel).display().to_string();
        let exclude_patterns: Vec<&str> = exclude.iter().map(|i| EXCLUDES[*i]).collect();

        // Lock decoy `zz` directories at levels the pattern's directory parts
        // before the first `**` rule out, so reading one is a traversal bug. With
        // `**`, only levels inside the prefix are safe; without it, levels up to
        // the file component (which is never read) are.
        #[cfg(unix)]
        let _locked = {
            let parts: Vec<&str> = dir_parts.iter().map(|i| DIR_PARTS[*i]).collect();
            let prefix = parts.iter().position(|p| *p == "**");
            let before = &parts[..prefix.unwrap_or(parts.len())];
            let depth_limit = prefix.unwrap_or(parts.len() + 1);
            if lock && before.iter().all(|p| NON_MATCHING_PARTS.contains(p)) && depth_limit >= 1 {
                let mut dirs = Vec::new();
                for (depth, parent) in [(1, root.to_path_buf()), (2, root.join("a"))] {
                    if depth <= depth_limit && parent.is_dir() {
                        let zz = parent.join("zz");
                        fs::create_dir(&zz).unwrap();
                        dirs.push(zz);
                    }
                }
                if dirs.is_empty() {
                    None
                } else {
                    let locked = Locked::lock(dirs);
                    prop_assume!(locked.enforced());
                    Some(locked)
                }
            } else {
                None
            }
        };
        #[cfg(not(unix))]
        let _ = lock;

        let matcher = GlobBuilder::new(&pattern).literal_separator(true).build().unwrap().compile_matcher();
        let matched: Vec<&PathBuf> = files.iter().filter(|f| matcher.is_match(f)).collect();
        let mut expected: Vec<String> = matched
            .iter()
            .filter(|f| !reference_excluded(&exclude_patterns, &rel(root, f)))
            .map(|f| rel(root, f))
            .collect();
        expected.sort();

        let options = Options {
            selection: Selection::Both,
            excludes: Some(Excludes::new(root, &exclude_patterns).unwrap()),
            ..Options::default()
        };
        match discover(&[pattern.as_str()], options) {
            Ok(found) => {
                prop_assert!(!matched.is_empty(), "discovery found files the oracle did not");
                let mut got: Vec<String> = found.files.iter().map(|f| rel(root, &f.path)).collect();
                got.sort();
                prop_assert_eq!(got, expected);
            }
            Err(DiscoverError::NoSuchTarget(_)) => prop_assert!(matched.is_empty()),
            Err(other) => prop_assert!(false, "unexpected error {other}"),
        }
    }

    #[test]
    fn directory_discovery_matches_the_oracle(
        tree in tree(),
        exclude in excludes(),
    ) {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let files = build(root, &tree);
        let exclude_patterns: Vec<&str> = exclude.iter().map(|i| EXCLUDES[*i]).collect();

        let mut expected: Vec<String> = files
            .iter()
            .map(|f| rel(root, f))
            .filter(|r| !reference_excluded(&exclude_patterns, r))
            .collect();
        expected.sort();

        let options = Options {
            selection: Selection::Both,
            excludes: Some(Excludes::new(root, &exclude_patterns).unwrap()),
            ..Options::default()
        };
        let found = discover(&[root.to_str().unwrap()], options).unwrap();
        let mut got: Vec<String> = found.files.iter().map(|f| rel(root, &f.path)).collect();
        got.sort();
        prop_assert_eq!(got, expected);
    }
}
