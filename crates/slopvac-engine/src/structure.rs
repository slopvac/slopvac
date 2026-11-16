//! Structure checks: block-level shape rules that the rule names in `match.check`.
//!
//! [`Check::HeadingHierarchy`] flags an ATX heading whose level is more than one deeper
//! than the heading before it ([`heading_skips`]). A check the engine does not know is
//! reported as a rule that did not run.

use std::ops::Range;

/// A structure check the engine runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Check {
    /// A heading that skips a level.
    HeadingHierarchy,
}

impl Check {
    /// The check named `name`, if the engine runs it.
    pub(crate) fn named(name: &str) -> Option<Self> {
        (name == "heading-hierarchy").then_some(Self::HeadingHierarchy)
    }
}

/// A heading line that skips a level, and the work the scan did.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Skips {
    /// The byte range of each flagged heading line, without its line ending.
    pub lines: Vec<Range<usize>>,
    /// The bytes the scan visited. It is linear in the source: a deterministic counter for
    /// the work-bound test.
    pub work: usize,
}

/// The heading lines in `source` that go more than one level deeper than the heading
/// before them.
///
/// The scan reads the whole file, front matter and HTML comments included. Lines end at
/// LF, CRLF or a lone CR. A heading line starts at column 0 with one or more `#` and a
/// space; an indented line, a setext heading and a heading in a block quote are not. A fence
/// line (up to three spaces, then three backticks or three tildes) toggles a fenced region,
/// whatever its marker. The first heading never counts as a skip.
pub(crate) fn heading_skips(source: &str) -> Skips {
    let mut out = Skips::default();
    let mut previous = 0usize;
    let mut fenced = false;
    let bytes = source.as_bytes();
    let mut start = 0usize;
    while start <= bytes.len() {
        let len = bytes[start..]
            .iter()
            .position(|&b| b == b'\n' || b == b'\r')
            .unwrap_or(bytes.len() - start);
        let content = &source[start..start + len];
        let end = start + len;
        let next = match (bytes.get(end), bytes.get(end + 1)) {
            (Some(b'\r'), Some(b'\n')) => end + 2,
            _ => end + 1,
        };
        out.work += next - start;
        if is_fence(content) {
            fenced = !fenced;
        } else if !fenced && let Some(level) = atx_level(content) {
            if previous > 0 && level > previous + 1 {
                out.lines.push(start..end);
            }
            previous = level;
        }
        start = next;
    }
    out
}

fn is_fence(line: &str) -> bool {
    let rest = line.trim_start_matches(' ');
    line.len() - rest.len() <= 3 && (rest.starts_with("```") || rest.starts_with("~~~"))
}

/// The level of an ATX heading line: the `#` run, followed by a space.
fn atx_level(line: &str) -> Option<usize> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    (hashes > 0 && line.as_bytes().get(hashes) == Some(&b' ')).then_some(hashes)
}

/// The ranges of the HTML comments in `source`: `<!--` up to the next `-->`, comments not
/// nested and not overlapping. A `<!--` with no `-->` after it is not a comment. The second
/// value counts the bytes the search visited, which is at most the source length plus the
/// tail after an unclosed opener.
pub(crate) fn html_comments(source: &str) -> (Vec<Range<usize>>, usize) {
    let mut comments = Vec::new();
    let mut work = 0usize;
    let mut from = 0usize;
    while let Some(open) = source[from..].find("<!--") {
        let open = from + open;
        let body = open + "<!--".len();
        let Some(close) = source[body..].find("-->") else {
            work += source.len() - from;
            break;
        };
        let end = body + close + "-->".len();
        work += end - from;
        comments.push(open..end);
        from = end;
    }
    (comments, work)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_work_is_linear_on_adversarial_documents() {
        let n = 5_000;
        let documents = [
            "<!--\n".repeat(n),
            "<!-->".repeat(n),
            "<!-- -->\n".repeat(n),
            "```\n#### x\n".repeat(n),
            "# a\n#### b\n".repeat(n),
            "# a\r#### b\r".repeat(n),
            "# a\r\n#### b\r\n".repeat(n),
            format!("{} x\n", "#".repeat(n)),
            "\n".repeat(n),
            "\r".repeat(n),
            "<!--".repeat(n) + "-->",
        ];
        for source in &documents {
            let work = heading_skips(source).work + html_comments(source).1;
            assert!(
                work <= 3 * source.len() + 3,
                "{} bytes took {work} steps",
                source.len()
            );
        }
    }
}
