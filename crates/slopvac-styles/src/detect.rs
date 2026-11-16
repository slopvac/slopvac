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

fn read(root: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(root.join(name)).ok()
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
        for name in value.split(',').map(str::trim).filter(|n| !n.is_empty()) {
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
    for (i, line) in text.lines().enumerate() {
        if !wanted(i + 1) {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        for g in GUIDES {
            if g.phrases.iter().any(|p| lower.contains(p)) {
                out.push(suggestion(g, file, i + 1, line));
            }
        }
    }
    out
}

/// The 1-based lines inside a section whose heading contains "style". A section
/// ends at the next heading of the same or a higher level.
fn style_section_lines(text: &str) -> Vec<usize> {
    let mut lines = Vec::new();
    let mut open: Option<usize> = None;
    for (i, line) in text.lines().enumerate() {
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
            if open.is_some_and(|level| hashes <= level) {
                open = None;
            }
            if open.is_none() && line.to_ascii_lowercase().contains("style") {
                open = Some(hashes);
            }
        }
        if open.is_some() {
            lines.push(i + 1);
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_ends_at_a_heading_of_the_same_level() {
        let text = "# Intro\nx\n## Style\ny\n### Detail\nz\n## Tests\nw\n";
        assert_eq!(style_section_lines(text), [3, 4, 5, 6]);
    }
}
