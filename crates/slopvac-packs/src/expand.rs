//! Expand the globs of a `provides` table against the files a pack holds.

use globset::GlobBuilder;
use regex::bytes::{Regex, RegexBuilder};

use crate::{PackError, PackPath, PackSource, Provides};

/// Most bytes in a `provides` glob.
pub const GLOB_MAX_BYTES: usize = 512;
/// Deepest nesting of `{` and `[` in a `provides` glob.
pub const GLOB_MAX_DEPTH: usize = 8;
/// Most commas, the alternatives of all `{a,b}` groups together, in a `provides` glob.
pub const GLOB_MAX_ALTERNATIVES: usize = 256;

/// Deepest regex nesting a glob may compile to.
const REGEX_NEST_LIMIT: u32 = 32;
/// Largest compiled regex a glob may build, in bytes.
const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// A cheap pre-filter: whether a glob is over the limits above.
///
/// The scan is not what keeps a glob from panicking. `GlobMatcher` compiles its regex with an
/// `expect`, and a bracket written as `{]` hides nesting from a scan. So [`matcher`] never calls
/// `compile_matcher`: it builds the regex from `Glob::regex()` with `regex`'s own nesting and
/// size limits, and any build error is a refusal. The scan only turns away the obvious cases
/// before any parsing. It is conservative: a `\` escapes the next character, and a closing
/// bracket only lowers the depth.
fn too_complex(glob: &str) -> bool {
    if glob.len() > GLOB_MAX_BYTES {
        return true;
    }
    let (mut depth, mut commas, mut escaped) = (0_usize, 0_usize, false);
    for c in glob.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '{' | '[' => {
                depth += 1;
                if depth > GLOB_MAX_DEPTH {
                    return true;
                }
            }
            '}' | ']' => depth = depth.saturating_sub(1),
            ',' => {
                commas += 1;
                if commas > GLOB_MAX_ALTERNATIVES {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Characters that make a `provides` entry a glob.
const META: [char; 4] = ['*', '?', '[', '{'];

/// Replace every glob in `provides` with the files it matches, in sorted order, once each.
///
/// `*` and `?` stay inside one path segment and `**` crosses segments. A glob that matches
/// nothing adds nothing. A path without a glob stays as written, so a missing file still fails
/// when the loader reads it. The pack's file list is read only when some entry is a glob.
///
/// # Errors
/// [`PackError::GlobTooComplex`] for a glob over the limits, [`PackError::Glob`] for a glob that
/// does not parse, and [`PackError::Source`] when the
/// pack cannot be listed.
pub(crate) fn expand(source: &dyn PackSource, provides: &Provides) -> Result<Provides, PackError> {
    let lists = [
        &provides.categories,
        &provides.styles,
        &provides.genres,
        &provides.terms,
        &provides.taggers,
        &provides.configs,
        &provides.prime,
        &provides.blocklists,
    ];
    let has_glob = lists.iter().flat_map(|l| l.iter()).any(is_glob);
    let files = if has_glob {
        source.files()?
    } else {
        Vec::new()
    };
    let one = |list: &[PackPath]| -> Result<Vec<PackPath>, PackError> {
        let mut out: Vec<PackPath> = Vec::new();
        for entry in list {
            if is_glob(entry) {
                let matcher = matcher(entry)?;
                for file in files.iter().filter(|f| matcher.is_match(f.as_bytes())) {
                    // A file whose name is not a valid pack path cannot be provided.
                    if let Some(path) = PackPath::parse(file).ok().filter(|p| !out.contains(p)) {
                        out.push(path);
                    }
                }
            } else if !out.contains(entry) {
                out.push(entry.clone());
            }
        }
        Ok(out)
    };
    let mut expanded = provides.clone();
    expanded.categories = one(&provides.categories)?;
    expanded.styles = one(&provides.styles)?;
    expanded.genres = one(&provides.genres)?;
    expanded.terms = one(&provides.terms)?;
    expanded.taggers = one(&provides.taggers)?;
    expanded.configs = one(&provides.configs)?;
    expanded.prime = one(&provides.prime)?;
    expanded.blocklists = one(&provides.blocklists)?;
    Ok(expanded)
}

fn is_glob(path: &PackPath) -> bool {
    path.as_str().contains(META)
}

/// Compile a `provides` glob to a regex over path bytes, as `globset` matches paths, under
/// limits that make the build fallible instead of a panic.
fn matcher(path: &PackPath) -> Result<Regex, PackError> {
    if too_complex(path.as_str()) {
        return Err(PackError::GlobTooComplex);
    }
    let glob = GlobBuilder::new(path.as_str())
        .literal_separator(true)
        .build()
        .map_err(|_| PackError::Glob {
            path: path.as_str().to_owned(),
        })?;
    RegexBuilder::new(glob.regex())
        .dot_matches_new_line(true)
        .nest_limit(REGEX_NEST_LIMIT)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|_| PackError::GlobTooComplex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_double_star_matches_a_segment_with_a_newline_as_globset_does() {
        let glob = "genres/**/a.yaml";
        let path = "genres/sub\n/a.yaml";
        let ours = matcher(&PackPath::parse(glob).unwrap()).unwrap();
        let theirs = GlobBuilder::new(glob)
            .literal_separator(true)
            .build()
            .unwrap()
            .compile_matcher();

        assert!(theirs.is_match(path));
        assert_eq!(ours.is_match(path.as_bytes()), theirs.is_match(path));
    }
}
