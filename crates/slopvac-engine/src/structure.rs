//! Structure checks: block-level shape rules that the rule names in `match.check`.
//!
//! The 2.x reference runs exactly one of the six core structure rules. `heading-hierarchy`
//! ran as a Vale script over the raw file, and the other five have no 2.x implementation,
//! so they never reported anything and never counted as unchecked. The golden suite records
//! that behaviour, and this module keeps it.
//!
//! - [`Check::HeadingHierarchy`] flags an ATX heading whose level is more than one deeper
//!   than the heading before it ([`heading_skips`]).
//! - [`NOT_IMPLEMENTED_IN_2X`] lists the checks that stay silent, as in 2.x.

use std::ops::Range;

/// A structure check the engine runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Check {
    /// A heading that skips a level.
    HeadingHierarchy,
}

/// The checks that 2.x names but never implemented. They report nothing and add no
/// notice, as in the reference.
const NOT_IMPLEMENTED_IN_2X: [&str; 5] = [
    "vertical-list-lead-in-missing-colon",
    "vertical-list-item-punctuation",
    "safety-block-missing-consequence",
    "inconsistent-term-for-same-thing",
    "inconsistent-wording-for-same-step",
];

/// What the engine does with a structure check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Plan {
    /// It runs.
    Runs(Check),
    /// It stays silent, as in 2.x.
    Silent,
    /// The engine does not know the check.
    Unknown,
}

/// The plan for the check named `check`.
pub(crate) fn plan(check: &str) -> Plan {
    if check == "heading-hierarchy" {
        Plan::Runs(Check::HeadingHierarchy)
    } else if NOT_IMPLEMENTED_IN_2X.contains(&check) {
        Plan::Silent
    } else {
        Plan::Unknown
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
/// before them, as the 2.x script found them.
///
/// The scan reads the whole file, front matter and HTML comments included, because the
/// script did. A heading line is a line that starts at column 0 with one or more `#` and a
/// space; an indented line, a setext heading and a heading in a block quote are not. A fence
/// line (up to three spaces, then three backticks or three tildes) toggles a fenced region,
/// whatever its marker. The first heading never counts as a skip.
pub(crate) fn heading_skips(source: &str) -> Skips {
    let mut out = Skips::default();
    let mut previous = 0usize;
    let mut fenced = false;
    let mut at = 0usize;
    for line in source.split('\n') {
        let start = at;
        at += line.len() + 1;
        out.work += line.len() + 1;
        let content = line.strip_suffix('\r').unwrap_or(line);
        if is_fence(content) {
            fenced = !fenced;
        } else if !fenced && let Some(level) = atx_level(content) {
            if previous > 0 && level > previous + 1 {
                out.lines.push(start..start + content.len());
            }
            previous = level;
        }
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

/// The ranges of the HTML comments in `source`, as the 2.x filter found them: `<!--` up to
/// the next `-->`, comments not nested and not overlapping. A `<!--` with no `-->` after it
/// is not a comment. The second value counts the bytes the search visited, which is at most
/// the source length plus the tail after an unclosed opener.
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

    const NONE: [&str; 0] = [];

    fn flagged(source: &str) -> Vec<&str> {
        heading_skips(source)
            .lines
            .into_iter()
            .map(|r| &source[r])
            .collect()
    }

    #[test]
    fn a_skipped_level_is_flagged_and_a_step_down_is_not() {
        assert_eq!(flagged("# a\n## b\n#### c\n"), ["#### c"]);
        assert_eq!(flagged("# a\n## b\n### c\n"), NONE);
    }

    #[test]
    fn the_first_heading_may_start_at_any_level() {
        assert_eq!(flagged("### a\n"), NONE);
    }

    #[test]
    fn a_heading_after_a_skip_compares_with_the_skipped_one() {
        assert_eq!(flagged("# a\n#### b\n##### c\n###### d\n"), ["#### b"]);
    }

    #[test]
    fn going_back_up_and_then_down_by_one_is_fine() {
        assert_eq!(flagged("# a\n## b\n### c\n# d\n## e\n"), NONE);
    }

    #[test]
    fn crlf_lines_end_before_the_carriage_return() {
        assert_eq!(flagged("# a\r\n### b\r\n"), ["### b"]);
    }

    #[test]
    fn a_fence_hides_its_lines_and_is_closed_by_any_fence_line() {
        assert_eq!(flagged("# a\n```\n#### x\n~~~\n"), NONE);
        assert_eq!(flagged("# a\n```\n#### x\n~~~\n#### y\n"), ["#### y"]);
    }

    #[test]
    fn a_fence_indented_four_spaces_is_not_a_fence() {
        assert_eq!(flagged("# a\n    ```\n#### x\n"), ["#### x"]);
    }

    #[test]
    fn an_unclosed_fence_hides_the_rest() {
        assert_eq!(flagged("# a\n```\n#### x\n"), NONE);
    }

    #[test]
    fn lines_that_are_not_atx_headings_do_not_count() {
        for text in [
            "  #### indented",
            "#nospace",
            "####",
            "#\ttab",
            "> #### quoted",
            "- #### item",
        ] {
            assert_eq!(
                flagged(&format!("# a\n{text}\n")),
                NONE,
                "{text:?} is not a heading"
            );
        }
    }

    #[test]
    fn a_heading_of_seven_hashes_has_level_seven() {
        assert_eq!(flagged("# a\n####### b\n"), ["####### b"]);
    }

    #[test]
    fn a_heading_with_nothing_after_the_space_still_counts() {
        assert_eq!(flagged("# a\n#### \n"), ["#### "]);
    }

    #[test]
    fn comments_run_from_an_opener_to_the_next_closer() {
        let text = "a <!-- x --> b <!-- y\n--> c <!-- open";
        let (c, _) = html_comments(text);
        assert_eq!(
            c.iter().map(|r| &text[r.clone()]).collect::<Vec<_>>(),
            ["<!-- x -->", "<!-- y\n-->"]
        );
    }

    #[test]
    fn an_opener_does_not_close_on_its_own_dashes() {
        let (c, _) = html_comments("<!--> x -->");
        assert_eq!(c, vec![0..11]);
    }

    #[test]
    fn the_work_is_linear_on_adversarial_documents() {
        let n = 5_000;
        let documents = [
            "<!--\n".repeat(n),
            "<!-->".repeat(n),
            "<!-- -->\n".repeat(n),
            "```\n#### x\n".repeat(n),
            "# a\n#### b\n".repeat(n),
            format!("{} x\n", "#".repeat(n)),
            "\n".repeat(n),
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

    #[test]
    fn plans_follow_the_check_name() {
        assert_eq!(
            plan("heading-hierarchy"),
            Plan::Runs(Check::HeadingHierarchy)
        );
        assert_eq!(plan("vertical-list-item-punctuation"), Plan::Silent);
        assert_eq!(plan("something-else"), Plan::Unknown);
    }
}
