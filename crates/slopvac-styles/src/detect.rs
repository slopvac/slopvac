//! House-style detection: read the files a project already uses to name its
//! style guide and suggest the bundled presets that match.
//!
//! Three sources are read, each as plain text, with the 1-based line of the match
//! as evidence:
//! - `.vale.ini`: the names listed in `BasedOnStyles`.
//! - `STYLE.md` and `_STYLE.md`: any line that names a known style guide.
//! - `CONTRIBUTING.md`: the same, but only inside a section whose heading contains
//!   "style".
//!
//! Detection never fails. A missing or unreadable file yields no suggestion.

use std::path::Path;

use crate::source::BuiltinSource;

/// A known style guide: how projects name it and which bundled preset encodes it.
struct Guide {
    /// How the guide is shown to the user.
    display: &'static str,
    /// Lower-case names that a Vale `BasedOnStyles` entry uses for it.
    vale_names: &'static [&'static str],
    /// Lower-case phrases that name the guide in prose.
    phrases: &'static [&'static str],
    /// The bundled preset, if one exists.
    preset: Option<&'static str>,
}

const GUIDES: &[Guide] = &[
    Guide {
        display: "Google developer documentation style guide",
        vale_names: &["google"],
        phrases: &[
            "google developer documentation",
            "developers.google.com/style",
        ],
        preset: Some("google"),
    },
    Guide {
        display: "Simplified Technical English",
        vale_names: &["ste"],
        phrases: &["simplified technical english", "asd-ste100"],
        preset: Some("ste"),
    },
    Guide {
        display: "Microsoft Writing Style Guide",
        vale_names: &["microsoft"],
        phrases: &[
            "microsoft writing style guide",
            "learn.microsoft.com/style-guide",
            "docs.microsoft.com/style-guide",
        ],
        preset: None,
    },
];

/// Vale styles that are not house styles.
const NOT_HOUSE_STYLES: &[&str] = &["vale"];

/// Where a suggestion comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    /// The file, relative to the project root.
    pub file: String,
    /// The 1-based line that names the style.
    pub line: usize,
    /// The trimmed text of that line.
    pub text: String,
}

/// One house style a project appears to use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// The style as the project names it.
    pub style: String,
    /// The bundled preset to put in `styles = [...]`, or `None` when slopvac
    /// ships no preset for this style.
    pub preset: Option<String>,
    /// The first line that names the style.
    pub evidence: Evidence,
}

impl Suggestion {
    /// One human line: the suggestion and where it was found.
    #[must_use]
    pub fn describe(&self) -> String {
        let at = format!("{}:{}", self.evidence.file, self.evidence.line);
        match &self.preset {
            Some(p) => format!("{}: suggest the \"{p}\" preset ({at})", self.style),
            None => format!("{}: no bundled preset ({at})", self.style),
        }
    }
}

/// Detect the house styles `root` declares.
///
/// The result is ordered by source (`.vale.ini`, `STYLE.md`, `_STYLE.md`,
/// `CONTRIBUTING.md`) and then by line. A style appears once, at its first match.
#[must_use]
pub fn detect_house_styles(root: &Path) -> Vec<Suggestion> {
    let mut found: Vec<Suggestion> = Vec::new();
    let mut add = |s: Suggestion| {
        if !found.iter().any(|f| f.style == s.style) {
            found.push(s);
        }
    };
    if let Some(text) = read(root, ".vale.ini") {
        vale_styles(&text).into_iter().for_each(&mut add);
    }
    for name in ["STYLE.md", "_STYLE.md"] {
        if let Some(text) = read(root, name) {
            phrase_matches(name, &text, |_| true)
                .into_iter()
                .for_each(&mut add);
        }
    }
    if let Some(text) = read(root, "CONTRIBUTING.md") {
        let in_style = style_section_lines(&text);
        phrase_matches("CONTRIBUTING.md", &text, |n| in_style.contains(&n))
            .into_iter()
            .for_each(&mut add);
    }
    found
}

/// The most bytes read from one file. A larger file is skipped.
const MAX_BYTES: u64 = 1024 * 1024;

/// Why [`read`] skipped a file.
#[derive(Debug, PartialEq, Eq)]
enum Skip {
    /// The file is larger than [`MAX_BYTES`].
    TooLarge,
    /// The file is missing, outside the root, not a regular file, not UTF-8 or unreadable.
    Unusable,
}

impl Skip {
    /// The stable code of the skip, or `None` when the file is merely unusable.
    ///
    /// `style-detect-file-too-large` is the code for [`Skip::TooLarge`]. Detection never
    /// fails, so no caller reports it yet.
    #[cfg(test)]
    fn code(&self) -> Option<&'static str> {
        match self {
            Skip::TooLarge => Some("style-detect-file-too-large"),
            Skip::Unusable => None,
        }
    }
}

/// Read a regular file under `root`, or `None`.
fn read(root: &Path, name: &str) -> Option<String> {
    read_checked(root, name).ok()
}

/// Read a regular file under `root`, or say why it was skipped.
///
/// The path is resolved first. A target outside the root is skipped. The file is
/// opened without blocking, so a FIFO swapped in after the check cannot hang the
/// open, and the opened handle must itself be a regular file. The read stops one
/// byte past [`MAX_BYTES`], so memory use is bounded for any input.
///
/// A local writer who races a symlink swap between the containment check and the
/// open is out of scope: they already control the repository's files.
fn read_checked(root: &Path, name: &str) -> Result<String, Skip> {
    use std::io::Read;
    let unusable = |_| Skip::Unusable;
    let root = std::fs::canonicalize(root).map_err(unusable)?;
    let path = std::fs::canonicalize(root.join(name)).map_err(unusable)?;
    if !path.starts_with(&root) {
        return Err(Skip::Unusable);
    }
    let file = open_regular(&path).map_err(unusable)?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(unusable)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(Skip::TooLarge);
    }
    String::from_utf8(bytes).map_err(unusable)
}

/// Open `path` for reading and require the opened handle to be a regular file.
///
/// This does no metadata check on the path first. The open must not block on a
/// FIFO, and the type is read from the handle, so a file swapped between a check and
/// the open cannot change what is read.
pub(crate) fn open_regular(path: &Path) -> std::io::Result<std::fs::File> {
    let file = open_nonblocking(path)?;
    if file.metadata()?.is_file() {
        Ok(file)
    } else {
        Err(std::io::Error::other("not a regular file"))
    }
}

/// Open for reading. On unix the open does not block on a FIFO.
fn open_nonblocking(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let flag = i32::try_from(rustix::fs::OFlags::NONBLOCK.bits()).unwrap_or(0);
        options.custom_flags(flag);
    }
    options.open(path)
}

fn suggestion(guide: &Guide, file: &str, line: usize, text: &str) -> Suggestion {
    Suggestion {
        style: guide.display.to_owned(),
        preset: guide
            .preset
            .filter(|p| BuiltinSource::names().any(|n| n == *p))
            .map(str::to_owned),
        evidence: Evidence {
            file: file.to_owned(),
            line,
            text: text.trim().to_owned(),
        },
    }
}

fn vale_styles(text: &str) -> Vec<Suggestion> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.starts_with(['#', ';']) {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("BasedOnStyles") {
            continue;
        }
        for name in strip_inline_comment(value)
            .split(',')
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            let lower = name.to_ascii_lowercase();
            if NOT_HOUSE_STYLES.contains(&lower.as_str()) {
                continue;
            }
            match GUIDES
                .iter()
                .find(|g| g.vale_names.contains(&lower.as_str()))
            {
                Some(g) => out.push(suggestion(g, ".vale.ini", i + 1, line)),
                None => out.push(Suggestion {
                    style: name.to_owned(),
                    preset: None,
                    evidence: Evidence {
                        file: ".vale.ini".to_owned(),
                        line: i + 1,
                        text: line.to_owned(),
                    },
                }),
            }
        }
    }
    out
}

fn phrase_matches(file: &str, text: &str, wanted: impl Fn(usize) -> bool) -> Vec<Suggestion> {
    let mut out = Vec::new();
    let fenced = fenced_lines(text);
    for (i, line) in text.lines().enumerate() {
        if !wanted(i + 1) || fenced[i] {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        for g in GUIDES {
            if g.phrases.iter().any(|p| adopted(&lower, p)) {
                out.push(suggestion(g, file, i + 1, line));
            }
        }
    }
    out
}

/// Words that reject a guide when they come before its name in the same clause.
const NEGATIONS: &[&str] = &[
    " not ",
    "n't",
    "n\u{2019}t",
    " never ",
    " no longer ",
    "unlike",
    "instead of",
    "rather than",
    " without ",
    " except ",
];

/// Words that make a mention an example when they come anywhere before it on the line.
const EXAMPLES: &[&str] = &["for example", "e.g."];

/// Text that ends a clause, with the length of the marker.
const CLAUSE_BREAKS: &[&str] = &[";", ",", ". ", " but ", " however "];

/// Whether `lower` mentions `phrase` as something the project follows.
///
/// A mention counts unless a negation comes before it in its own clause, or the line
/// introduces it as an example. A cue after the name, or in another clause, does not
/// count: "We follow it, but never use emoji" still adopts it.
fn adopted(lower: &str, phrase: &str) -> bool {
    lower.match_indices(phrase).any(|(at, _)| {
        let before = &lower[..at];
        if EXAMPLES.iter().any(|e| before.contains(e)) {
            return false;
        }
        let start = CLAUSE_BREAKS
            .iter()
            .flat_map(|b| lower[..at].match_indices(b).map(move |(i, _)| (i, *b)))
            .filter(|(i, b)| *b != "," || !starts_with_guide(&lower[i + 1..]))
            .map(|(i, b)| i + b.len())
            .max()
            .unwrap_or(0);
        let clause = format!(" {}", &before[start..]);
        !NEGATIONS.iter().any(|n| clause.contains(n))
    })
}

/// Whether `after` (the text after a comma) starts another guide name, optionally
/// behind "and", "or", "and/or" or "the". Such a comma separates a list of guides and
/// does not end the clause, so a rejection before the list covers every item.
fn starts_with_guide(after: &str) -> bool {
    let mut rest = after.trim_start();
    for lead in ["and/or ", "and ", "or "] {
        if let Some(r) = rest.strip_prefix(lead) {
            rest = r.trim_start();
            break;
        }
    }
    rest = rest.strip_prefix("the ").unwrap_or(rest);
    GUIDES.iter().any(|g| {
        g.phrases.iter().any(|p| rest.starts_with(p))
            || g.vale_names.iter().any(|n| {
                rest.strip_prefix(n)
                    .is_some_and(|tail| !tail.starts_with(|c: char| c.is_alphanumeric()))
            })
    })
}

/// For each line, whether it is a fence delimiter or inside a fenced code block.
///
/// A fence opens on three or more backticks or tildes (up to three spaces of
/// indent). It closes on the same character, at least as many, and nothing but
/// whitespace after them (`CommonMark`).
fn fenced_lines(text: &str) -> Vec<bool> {
    let mut open: Option<(char, usize)> = None;
    text.lines()
        .map(|line| {
            let indent = line.len() - line.trim_start_matches(' ').len();
            let t = &line[indent..];
            let marker = t.chars().next().filter(|c| *c == '`' || *c == '~');
            let run = marker.map_or(0, |m| t.chars().take_while(|c| *c == m).count());
            let fence = marker.filter(|_| indent <= 3 && run >= 3);
            match (open, fence) {
                (None, Some(m)) => {
                    open = Some((m, run));
                    true
                }
                (Some((m, len)), Some(c)) if c == m && run >= len && t[run..].trim().is_empty() => {
                    open = None;
                    true
                }
                (Some(_), _) => true,
                (None, None) => false,
            }
        })
        .collect()
}

/// Cut a Vale INI value at an inline comment: a `;` or `#` at the start of the
/// value or after whitespace.
fn strip_inline_comment(value: &str) -> &str {
    let mut prev_space = true;
    for (i, c) in value.char_indices() {
        if prev_space && (c == ';' || c == '#') {
            return &value[..i];
        }
        prev_space = c.is_whitespace();
    }
    value
}

/// The level and text of an ATX heading: up to three spaces of indent, one to six
/// `#`, then a space, a tab or the end of the line.
fn atx_heading(line: &str) -> Option<(usize, &str)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let hashes = rest.chars().take_while(|c| *c == '#').count();
    let title = &rest[hashes..];
    let separated = title.is_empty() || title.starts_with([' ', '\t']);
    ((1..=6).contains(&hashes) && separated).then_some((hashes, title))
}

/// The 1-based lines inside a section whose heading contains "style". A section
/// ends at the next heading of the same or a higher level.
fn style_section_lines(text: &str) -> Vec<usize> {
    let fenced = fenced_lines(text);
    let mut lines = Vec::new();
    let mut open: Option<usize> = None;
    for (i, line) in text.lines().enumerate() {
        if let (false, Some((level, title))) = (fenced[i], atx_heading(line)) {
            if open.is_some_and(|l| level <= l) {
                open = None;
            }
            if open.is_none() && title.to_ascii_lowercase().contains("style") {
                open = Some(level);
            }
        }
        if open.is_some() {
            lines.push(i + 1);
        }
    }
    lines
}

#[cfg(all(test, unix))]
mod tests {
    use super::{MAX_BYTES, Skip, open_regular, read_checked};
    use std::sync::mpsc;
    use std::time::Duration;

    /// Regression for the open race: a FIFO at the path is what a swap would leave.
    /// A plain `File::open` on it blocks until a writer appears.
    #[test]
    fn opening_a_fifo_returns_promptly_with_not_a_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("pipe");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(open_regular(&fifo).map(|_| ()));
        });
        let result = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("open_regular blocked on a FIFO");
        assert_eq!(result.unwrap_err().to_string(), "not a regular file");
    }

    #[test]
    fn opening_a_regular_file_works() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, "x").unwrap();
        assert!(open_regular(&file).is_ok());
    }

    #[test]
    fn a_file_over_the_cap_is_skipped_with_its_code_and_one_at_the_cap_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let len = usize::try_from(MAX_BYTES).unwrap();
        std::fs::write(dir.path().join("STYLE.md"), "a".repeat(len)).unwrap();
        assert_eq!(read_checked(dir.path(), "STYLE.md").unwrap().len(), len);

        std::fs::write(dir.path().join("STYLE.md"), "a".repeat(len + 1)).unwrap();
        let skip = read_checked(dir.path(), "STYLE.md").unwrap_err();
        assert_eq!(skip, Skip::TooLarge);
        assert_eq!(skip.code(), Some("style-detect-file-too-large"));
    }
}
