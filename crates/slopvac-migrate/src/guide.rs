//! The migration guide, kept in one source for `slopvac prime migrate` and
//! `docs/migrate-2-to-3.md`.
//!
//! Each `##` heading in the source is followed by a marker line,
//! `<!-- prime: include -->` or `<!-- prime: link -->`. The prime topic prints
//! an `include` section in full and replaces a `link` section with one line that
//! points to its heading in the generated doc. The generated doc drops the
//! marker lines.

const GUIDE: &str = include_str!("../data/migrate-guide.md");

const DOC_NOTICE: &str = "<!-- Generated from crates/slopvac-migrate/data/migrate-guide.md. \
Edit that file and run `UPDATE_DOCS=1 cargo test -p slopvac-migrate --test guide`. -->\n\n";

const DOC_URL: &str = "https://github.com/slopvac/slopvac/blob/main/docs/migrate-2-to-3.md";

const MARKER_PREFIX: &str = "<!-- prime: ";
const MARKER_SUFFIX: &str = " -->";

/// The marker value of a `link` section.
const LINK: &str = "link";

/// The value of a marker line, if `line` is one.
fn marker(line: &str) -> Option<&str> {
    line.trim_end()
        .strip_prefix(MARKER_PREFIX)?
        .strip_suffix(MARKER_SUFFIX)
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

/// The text `slopvac prime migrate` prints.
#[must_use]
pub fn prime_migrate() -> String {
    let mut out = String::new();
    let mut linked = false;
    let mut lines = GUIDE.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        if let Some(heading) = line.strip_prefix("## ") {
            let heading = heading.trim_end();
            linked = lines.peek().and_then(|next| marker(next)) == Some(LINK);
            if linked {
                for part in [
                    "Read \"",
                    heading,
                    "\" at ",
                    DOC_URL,
                    "#",
                    &anchor(heading),
                    ".\n\n",
                ] {
                    out.push_str(part);
                }
                continue;
            }
        } else if linked {
            continue;
        }
        if marker(line).is_none() {
            out.push_str(line);
        }
    }
    out
}

/// The text of `docs/migrate-2-to-3.md`: the guide under a generated-file notice.
#[must_use]
pub fn doc() -> String {
    let body: String = GUIDE
        .split_inclusive('\n')
        .filter(|line| marker(line).is_none())
        .collect();
    format!("{DOC_NOTICE}{body}")
}
