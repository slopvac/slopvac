//! Parse the added-line ranges out of a `git diff --unified=0` patch.
//!
//! Git's patch format has enough metadata to tell a binary or rename-only section from
//! malformed text. Silently skipping a truncated section would turn a requested
//! changed-line check into a false clean result, so malformed headers and hunks fail
//! closed, as they did in 2.x (`_parse_patch` in `diff_scope.py`).
//!
//! Differences from 2.x, both of which only turn an exit 2 into a result:
//!
//! - A section with extended headers but no `---`/`+++` pair is valid. Git prints that
//!   for an empty new file and for a mode-only change.
//! - Lines split on `\n` only. A form feed or lone `\r` inside changed text cannot split
//!   a patch line.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::path::PathBuf;

use super::DiffError;

/// Added-line ranges per repository-relative path.
pub(super) type Parsed = BTreeMap<PathBuf, Vec<RangeInclusive<u32>>>;

fn malformed(message: impl Into<String>) -> DiffError {
    DiffError::Malformed(message.into())
}

/// Extended header lines that git prints between `diff --git` and the first hunk.
const EXTENDED_HEADERS: [&[u8]; 14] = [
    b"index ",
    b"old mode ",
    b"new mode ",
    b"new file mode ",
    b"deleted file mode ",
    b"similarity index ",
    b"dissimilarity index ",
    b"rename from ",
    b"rename to ",
    b"copy from ",
    b"copy to ",
    b"Binary files ",
    b"GIT binary patch",
    b"\\ No newline at end of file",
];

// Parser state: each flag records a header already seen in the current section.
#[allow(clippy::struct_excessive_bools)]
#[derive(Default)]
struct Section {
    open: bool,
    current: Option<PathBuf>,
    old_seen: bool,
    new_seen: bool,
    /// Whether any extended header (including binary and rename lines) was present.
    metadata: bool,
    saw_hunk: bool,
    hunk_body: bool,
}

impl Section {
    fn finish(&self) -> Result<(), DiffError> {
        if self.saw_hunk && !self.hunk_body {
            return Err(malformed("hunk has no body"));
        }
        if self.open && !self.new_seen && !self.metadata {
            return Err(malformed("file section has no new path"));
        }
        Ok(())
    }
}

/// Parse a complete patch.
pub(super) fn parse(patch: &[u8]) -> Result<Parsed, DiffError> {
    let mut found: Parsed = BTreeMap::new();
    let mut section = Section::default();

    let patch = patch.strip_suffix(b"\n").unwrap_or(patch);
    for line in patch.split(|b| *b == b'\n') {
        if line.starts_with(b"diff --git ") {
            section.finish()?;
            section = Section {
                open: true,
                ..Section::default()
            };
            continue;
        }
        if line.starts_with(b"diff --") {
            return Err(malformed("ambiguous Git diff format"));
        }
        if !section.open {
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            return Err(malformed("content before file header"));
        }
        if line.starts_with(b"@@") {
            if !section.new_seen {
                return Err(malformed("hunk before file paths"));
            }
            if section.saw_hunk && !section.hunk_body {
                return Err(malformed("hunk has no body"));
            }
            let (start, count) = parse_hunk_header(line)?;
            section.saw_hunk = true;
            section.hunk_body = false;
            if let (Some(path), true) = (&section.current, count > 0) {
                let end = start
                    .checked_add(count - 1)
                    .ok_or_else(|| malformed("hunk range overflows"))?;
                if let Some(ranges) = found.get_mut(path) {
                    ranges.push(start..=end);
                }
            }
            continue;
        }
        if section.saw_hunk {
            // Once a hunk starts, every line up to the next header is its body, so a
            // deleted `--- x` or an added `+++ x` is never read as file metadata.
            if !line.starts_with(b"\\ No newline at end of file") {
                section.hunk_body = true;
            }
            continue;
        }
        if line.starts_with(b"\\ No newline at end of file") {
            return Err(malformed("newline marker without hunk"));
        }
        if let Some(old) = line.strip_prefix(b"--- ") {
            if section.old_seen {
                return Err(malformed("duplicate old path"));
            }
            decode_path(old, b"a/")?;
            section.old_seen = true;
        } else if let Some(new) = line.strip_prefix(b"+++ ") {
            if !section.old_seen || section.new_seen {
                return Err(malformed("misplaced new path"));
            }
            section.current = decode_path(new, b"b/")?;
            section.new_seen = true;
            if let Some(path) = &section.current
                && found.insert(path.clone(), Vec::new()).is_some()
            {
                return Err(malformed(format!(
                    "ambiguous Git diff: duplicate path {}",
                    path.display()
                )));
            }
        } else if EXTENDED_HEADERS.iter().any(|h| line.starts_with(h)) {
            section.metadata = true;
        }
        // Anything else between headers carries no interval.
    }
    section.finish()?;

    Ok(found
        .into_iter()
        .filter(|(_, ranges)| !ranges.is_empty())
        .map(|(path, ranges)| (path, merge(ranges)))
        .collect())
}

/// Sort ranges and join those that overlap or touch.
fn merge(mut ranges: Vec<RangeInclusive<u32>>) -> Vec<RangeInclusive<u32>> {
    ranges.sort_by_key(|r| (*r.start(), *r.end()));
    let mut merged: Vec<RangeInclusive<u32>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if *range.start() <= last.end().saturating_add(1) => {
                *last = *last.start()..=(*last.end()).max(*range.end());
            }
            _ => merged.push(range),
        }
    }
    merged
}

/// Read `@@ -a[,b] +c[,d] @@` and return the new side `(start, count)`.
fn parse_hunk_header(line: &[u8]) -> Result<(u32, u32), DiffError> {
    let bad = || {
        malformed(format!(
            "malformed hunk header: {}",
            String::from_utf8_lossy(line)
        ))
    };
    let rest = line.strip_prefix(b"@@ -").ok_or_else(bad)?;
    let (_, rest) = take_range(rest).ok_or_else(bad)?;
    let rest = rest.strip_prefix(b" +").ok_or_else(bad)?;
    let ((start, count), rest) = take_range(rest).ok_or_else(bad)?;
    if !rest.starts_with(b" @@") {
        return Err(bad());
    }
    let count = count.unwrap_or(1);
    if count > 0 && start < 1 {
        return Err(malformed(format!(
            "invalid hunk range: {}",
            String::from_utf8_lossy(line)
        )));
    }
    Ok((start, count))
}

/// Read `<number>[,<number>]` and return the numbers with the remaining bytes.
/// A start with an optional count.
type Numbers = (u32, Option<u32>);

fn take_range(input: &[u8]) -> Option<(Numbers, &[u8])> {
    let (first, rest) = take_number(input)?;
    match rest.strip_prefix(b",") {
        Some(after) => {
            let (second, rest) = take_number(after)?;
            Some(((first, Some(second)), rest))
        }
        None => Some(((first, None), rest)),
    }
}

fn take_number(input: &[u8]) -> Option<(u32, &[u8])> {
    let digits = input.iter().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let (number, rest) = input.split_at(digits);
    let value = std::str::from_utf8(number).ok()?.parse().ok()?;
    Some((value, rest))
}

/// Decode one `---` or `+++` path. `/dev/null` is `None`.
///
/// Git quotes a name with a control character, `"` or `\` in C style, and follows a name
/// that has a space by a tab. A tab inside a name is always quoted, so one trailing tab is
/// never part of the name.
fn decode_path(value: &[u8], prefix: &[u8]) -> Result<Option<PathBuf>, DiffError> {
    if value == b"/dev/null" {
        return Ok(None);
    }
    let value = value.strip_suffix(b"\t").unwrap_or(value);
    let bytes = if value.len() >= 2 && value.starts_with(b"\"") && value.ends_with(b"\"") {
        unquote(&value[1..value.len() - 1], value)?
    } else {
        value.to_vec()
    };
    let relative = bytes.strip_prefix(prefix).ok_or_else(|| {
        malformed(format!(
            "malformed path marker: {}",
            String::from_utf8_lossy(value)
        ))
    })?;
    to_path(relative).map(Some)
}

fn unquote(body: &[u8], whole: &[u8]) -> Result<Vec<u8>, DiffError> {
    let mut out = Vec::with_capacity(body.len());
    let mut index = 0;
    while index < body.len() {
        if body[index] != b'\\' {
            out.push(body[index]);
            index += 1;
            continue;
        }
        let marker = *body
            .get(index + 1)
            .ok_or_else(|| malformed("malformed quoted path"))?;
        let simple = match marker {
            b'\\' => Some(b'\\'),
            b'"' => Some(b'"'),
            b'a' => Some(7),
            b'b' => Some(8),
            b'f' => Some(12),
            b'n' => Some(10),
            b'r' => Some(13),
            b't' => Some(9),
            b'v' => Some(11),
            _ => None,
        };
        if let Some(byte) = simple {
            out.push(byte);
            index += 2;
            continue;
        }
        let octal = body
            .get(index + 1..index + 4)
            .filter(|digits| digits.iter().all(|d| (b'0'..=b'7').contains(d)))
            .ok_or_else(|| {
                malformed(format!(
                    "ambiguous path escape: {}",
                    String::from_utf8_lossy(whole)
                ))
            })?;
        let value = octal
            .iter()
            .fold(0u32, |acc, d| acc * 8 + u32::from(d - b'0'));
        out.push(u8::try_from(value).map_err(|_| malformed("path escape out of range"))?);
        index += 4;
    }
    Ok(out)
}

#[cfg(unix)]
#[allow(clippy::unnecessary_wraps)] // Same signature as the non-Unix build, which can fail.
pub(super) fn to_path(bytes: &[u8]) -> Result<PathBuf, DiffError> {
    use std::os::unix::ffi::OsStrExt;
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
}

#[cfg(not(unix))]
pub(super) fn to_path(bytes: &[u8]) -> Result<PathBuf, DiffError> {
    std::str::from_utf8(bytes)
        .map(PathBuf::from)
        .map_err(|_| malformed("path is not valid UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(lines: &[&str]) -> Vec<u8> {
        let mut text = lines.join("\n");
        text.push('\n');
        text.into_bytes()
    }

    fn ranges(entries: &[(&str, &[(u32, u32)])]) -> Parsed {
        entries
            .iter()
            .map(|(path, spans)| {
                (
                    PathBuf::from(path),
                    spans.iter().map(|&(a, b)| a..=b).collect(),
                )
            })
            .collect()
    }

    #[test]
    #[allow(clippy::too_many_lines)] // A table of patch cases.
    fn variants_yield_added_ranges() {
        let cases: Vec<(&str, Vec<u8>, Parsed)> = vec![
            (
                "modified",
                patch(&[
                    "diff --git a/doc.md b/doc.md",
                    "index 1111111..2222222 100644",
                    "--- a/doc.md",
                    "+++ b/doc.md",
                    "@@ -3,1 +3,2 @@ heading",
                    "+new",
                    "+second",
                ]),
                ranges(&[("doc.md", &[(3, 4)])]),
            ),
            (
                "added",
                patch(&[
                    "diff --git a/new.md b/new.md",
                    "new file mode 100644",
                    "index 0000000..2222222",
                    "--- /dev/null",
                    "+++ b/new.md",
                    "@@ -0,0 +1,3 @@",
                    "+one",
                    "+two",
                    "+three",
                ]),
                ranges(&[("new.md", &[(1, 3)])]),
            ),
            (
                "deleted",
                patch(&[
                    "diff --git a/old.md b/old.md",
                    "deleted file mode 100644",
                    "index 2222222..0000000",
                    "--- a/old.md",
                    "+++ /dev/null",
                    "@@ -1,2 +0,0 @@",
                    "-old",
                    "-text",
                ]),
                ranges(&[]),
            ),
            (
                "rename",
                patch(&[
                    "diff --git a/old.md b/new.md",
                    "similarity index 98%",
                    "rename from old.md",
                    "rename to new.md",
                    "--- a/old.md",
                    "+++ b/new.md",
                    "@@ -2 +2 @@",
                    "-old",
                    "+new",
                ]),
                ranges(&[("new.md", &[(2, 2)])]),
            ),
            (
                "rename only",
                patch(&[
                    "diff --git a/old.md b/new.md",
                    "similarity index 100%",
                    "rename from old.md",
                    "rename to new.md",
                ]),
                ranges(&[]),
            ),
            (
                "binary",
                patch(&[
                    "diff --git a/image.bin b/image.bin",
                    "new file mode 100644",
                    "index 0000000..2222222",
                    "Binary files /dev/null and b/image.bin differ",
                ]),
                ranges(&[]),
            ),
            (
                "empty new file",
                patch(&[
                    "diff --git a/empty.md b/empty.md",
                    "new file mode 100644",
                    "index 0000000..e69de29",
                ]),
                ranges(&[]),
            ),
            (
                "mode only",
                patch(&[
                    "diff --git a/run.sh b/run.sh",
                    "old mode 100644",
                    "new mode 100755",
                ]),
                ranges(&[]),
            ),
            (
                "path with spaces and a trailing tab",
                patch(&[
                    "diff --git a/docs/release notes.md b/docs/release notes.md",
                    "index 1111111..2222222 100644",
                    "--- a/docs/release notes.md\t",
                    "+++ b/docs/release notes.md\t",
                    "@@ -1 +1 @@",
                    "-old",
                    "+new",
                ]),
                ranges(&[("docs/release notes.md", &[(1, 1)])]),
            ),
            (
                "quoted path with octal UTF-8 escapes",
                patch(&[
                    "diff --git \"a/caf\\303\\251\\\"q\" \"b/caf\\303\\251\\\"q\"",
                    "index 1111111..2222222 100644",
                    "--- \"a/caf\\303\\251\\\"q\"",
                    "+++ \"b/caf\\303\\251\\\"q\"",
                    "@@ -1 +1 @@",
                    "-old",
                    "+new",
                ]),
                ranges(&[("caf\u{e9}\"q", &[(1, 1)])]),
            ),
            (
                "multiple hunks",
                patch(&[
                    "diff --git a/doc.md b/doc.md",
                    "index 1111111..2222222 100644",
                    "--- a/doc.md",
                    "+++ b/doc.md",
                    "@@ -2 +2 @@",
                    "-old",
                    "+new",
                    "@@ -8,1 +9,2 @@",
                    "+one",
                    "+two",
                ]),
                ranges(&[("doc.md", &[(2, 2), (9, 10)])]),
            ),
            (
                "touching hunks merge",
                patch(&[
                    "diff --git a/doc.md b/doc.md",
                    "--- a/doc.md",
                    "+++ b/doc.md",
                    "@@ -2 +2 @@",
                    "-a",
                    "+b",
                    "@@ -3 +3 @@",
                    "-c",
                    "+d",
                ]),
                ranges(&[("doc.md", &[(2, 3)])]),
            ),
            (
                "pure deletion has no range",
                patch(&[
                    "diff --git a/doc.md b/doc.md",
                    "--- a/doc.md",
                    "+++ b/doc.md",
                    "@@ -4,2 +3,0 @@",
                    "-gone",
                    "-gone",
                ]),
                ranges(&[]),
            ),
            (
                "no newline marker",
                patch(&[
                    "diff --git a/doc.md b/doc.md",
                    "index 1111111..2222222 100644",
                    "--- a/doc.md",
                    "+++ b/doc.md",
                    "@@ -1 +1 @@",
                    "-old",
                    "\\ No newline at end of file",
                    "+new",
                    "\\ No newline at end of file",
                ]),
                ranges(&[("doc.md", &[(1, 1)])]),
            ),
            (
                "body lines that look like file headers",
                patch(&[
                    "diff --git a/doc.md b/doc.md",
                    "--- a/doc.md",
                    "+++ b/doc.md",
                    "@@ -1,2 +1,2 @@",
                    "--- a/other.md",
                    "+++ b/other.md",
                ]),
                ranges(&[("doc.md", &[(1, 2)])]),
            ),
        ];
        for (name, input, expected) in cases {
            assert_eq!(parse(&input).unwrap(), expected, "{name}");
        }
    }

    #[test]
    fn malformed_or_ambiguous_output_is_an_error() {
        let cases = [
            "+++ b/doc.md\n@@ -1 +1 @@\n+new\n",
            "diff --git a/doc.md b/doc.md\n--- a/doc.md\n+++ b/doc.md\n@@ nope\n",
            "diff --git a/doc.md b/doc.md\n+++ b/doc.md\n",
            "diff --git a/doc.md b/doc.md\n--- a/doc.md\n+++ b/doc.md\n@@ -1 +1 @@\n",
            "diff --git a/doc.md b/doc.md\n--- a/doc.md\n+++ b/doc.md\n+++ b/other.md\n",
            "diff --git a/doc.md b/doc.md\n--- a/doc.md\n+++ c/doc.md\n@@ -1 +1 @@\n",
            "diff --git a/doc.md b/doc.md\n@@ -1 +1 @@\n",
            "diff --git a/doc.md b/doc.md\n\\ No newline at end of file\n",
            "diff --git a/doc.md b/doc.md\n",
            "diff --cc doc.md\n",
            "diff --git a/doc.md b/doc.md\n--- a/doc.md\n+++ b/doc.md\n@@ -1 +0 @@\n-x\n",
        ];
        for input in cases {
            assert!(
                matches!(parse(input.as_bytes()), Err(DiffError::Malformed(_))),
                "{input:?}"
            );
        }
    }
}
