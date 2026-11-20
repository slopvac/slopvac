//! The migration guide, kept in one source for `slopvac prime migrate` and
//! `docs/migrate-2-to-3.md`.
//!
//! Each `##` heading in the source is followed by a marker line,
//! `<!-- prime: include -->` or `<!-- prime: link -->`. The prime topic prints
//! an `include` section in full and replaces a `link` section with one line that
//! points to its heading in the generated doc. The generated doc drops the
//! marker lines.

use crate::messages::MigrateText;

const GUIDE: &str = include_str!("../data/migrate-guide.md");

const DOC_NOTICE: &str = "<!-- Generated from crates/slopvac-migrate/data/migrate-guide.md. \
Edit that file and run `UPDATE_DOCS=1 cargo test -p slopvac-migrate --test guide`. -->\n\n";

const DOC_URL: &str = "https://github.com/slopvac/slopvac/blob/main/docs/migrate-2-to-3.md";

const MARKER_PREFIX: &str = "<!-- prime: ";
const MARKER_SUFFIX: &str = " -->";

#[cfg(test)]
const INCLUDE: &str = "include";

/// The marker value of a `link` section.
const LINK: &str = "link";

/// A marker line that breaks the contract of the guide source.
#[cfg(test)]
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
enum MarkerError {
    #[error("line {line}: the heading has no marker on the next line")]
    Missing { line: usize },
    #[error("line {line}: unknown marker value {value:?}")]
    Unknown { line: usize, value: String },
    #[error("line {line}: a marker not directly after a `##` heading")]
    Stray { line: usize },
}

/// Whether `line` is meant to be a marker, well formed or not.
fn is_marker_like(line: &str) -> bool {
    line.trim_start().starts_with("<!-- prime:")
}

/// The value of a well-formed marker line.
fn marker(line: &str) -> Option<&str> {
    line.trim_end()
        .strip_prefix(MARKER_PREFIX)?
        .strip_suffix(MARKER_SUFFIX)
}

/// Check that exactly one recognised marker follows each `##` heading and no
/// other marker line exists.
#[cfg(test)]
fn validate(guide: &str) -> Result<(), MarkerError> {
    let lines: Vec<&str> = guide.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let number = i + 1;
        if line.starts_with("## ") {
            match lines.get(i + 1) {
                Some(next) if is_marker_like(next) => match marker(next) {
                    Some(INCLUDE | LINK) => {}
                    other => {
                        return Err(MarkerError::Unknown {
                            line: number + 1,
                            value: other.unwrap_or(next).to_owned(),
                        });
                    }
                },
                _ => return Err(MarkerError::Missing { line: number }),
            }
        } else if is_marker_like(line)
            && !lines
                .get(i.wrapping_sub(1))
                .is_some_and(|p| p.starts_with("## "))
        {
            return Err(MarkerError::Stray { line: number });
        }
    }
    Ok(())
}

/// The GitHub anchor of a heading such as `4. Apply`.
fn anchor(heading: &str) -> String {
    heading
        .chars()
        .filter_map(|c| match c {
            ' ' | '-' => Some('-'),
            c if c.is_alphanumeric() => Some(c.to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

/// Render the prime topic. `validate` guards the embedded guide in a test; a
/// section whose marker is not `link` is printed in full.
fn render_prime(guide: &str) -> String {
    let mut out = String::new();
    let mut linked = false;
    let mut lines = guide.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        if let Some(heading) = line.strip_prefix("## ") {
            let heading = heading.trim_end();
            linked = lines.peek().and_then(|next| marker(next)) == Some(LINK);
            if linked {
                out.push_str(
                    &MigrateText::GuideLink {
                        heading: heading.to_owned(),
                        url: DOC_URL.to_owned(),
                        anchor: anchor(heading),
                    }
                    .to_string(),
                );
                continue;
            }
        } else if linked {
            continue;
        }
        if !is_marker_like(line) {
            out.push_str(line);
        }
    }
    out
}

fn render_doc(guide: &str) -> String {
    let body: String = guide
        .split_inclusive('\n')
        .filter(|line| !is_marker_like(line))
        .collect();
    format!("{DOC_NOTICE}{body}")
}

/// The text `slopvac prime migrate` prints.
#[must_use]
pub fn prime_migrate() -> String {
    render_prime(GUIDE)
}

/// The text of `docs/migrate-2-to-3.md`: the guide under a generated-file notice.
#[must_use]
pub fn doc() -> String {
    render_doc(GUIDE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[test]
    fn the_embedded_guide_has_one_recognised_marker_after_every_heading() {
        assert_eq!(validate(GUIDE), Ok(()));
        let headings = GUIDE.lines().filter(|l| l.starts_with("## ")).count();
        let markers = GUIDE.lines().filter(|l| is_marker_like(l)).count();
        assert_eq!(headings, 6);
        assert_eq!(markers, headings);
    }

    #[rstest]
    #[case::missing("## A\n\ntext\n")]
    #[case::missing_at_end("## A")]
    #[case::unknown_value("## A\n<!-- prime: skip -->\n")]
    #[case::malformed("## A\n<!-- prime: include\n")]
    #[case::stray_in_body("## A\n<!-- prime: include -->\ntext\n<!-- prime: link -->\n")]
    #[case::before_any_heading("<!-- prime: include -->\n## A\n<!-- prime: include -->\n")]
    #[case::second_after_heading("## A\n<!-- prime: include -->\n<!-- prime: link -->\n")]
    #[case::after_a_blank_line("## A\n\n<!-- prime: include -->\n")]
    fn a_broken_marker_is_rejected(#[case] guide: &str) {
        assert!(validate(guide).is_err());
    }

    #[rstest]
    #[case::include("## A\n<!-- prime: include -->\ntext\n")]
    #[case::link("## A\n<!-- prime: link -->\ntext\n")]
    #[case::two_sections("## A\n<!-- prime: include -->\nx\n## B\n<!-- prime: link -->\ny\n")]
    fn well_formed_markers_are_accepted(#[case] guide: &str) {
        assert!(validate(guide).is_ok());
    }
}
