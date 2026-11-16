//! Generated reference pages for the bundled style presets.
//!
//! [`reference_pages`] returns `(path, markdown)` pairs relative to the docs
//! site root. The docs builder writes them under `styles/`.

use std::fmt::Write as _;

use crate::source::BUILTIN;
use crate::{Budget, CategoryDials, Level, Style};

fn level(l: Level) -> &'static str {
    match l {
        Level::Error => "error",
        Level::Warning => "warning",
        Level::Suggestion => "suggestion",
        Level::Off => "off",
    }
}

/// The leading `#` comment block of a preset file, with the markers removed.
fn notes(text: &str) -> String {
    text.lines()
        .take_while(|l| l.starts_with('#') || l.is_empty())
        .filter(|l| !l.is_empty())
        .map(|l| l.trim_start_matches('#').trim_start())
        .collect::<Vec<_>>()
        .join("\n")
}

fn dials(d: &CategoryDials) -> String {
    let mut parts = Vec::new();
    if let Some(s) = d.severity {
        parts.push(format!("severity `{}`", level(s)));
    }
    if let Some(b) = d.minimum_severity {
        parts.push(format!("minimum severity `{}`", level(b)));
    }
    if let Some(n) = d.max_per_100_words {
        match n {
            Budget::Limit(n) => parts.push(format!("max per 100 words `{n}`")),
            Budget::None => parts.push("max per 100 words `none`".to_owned()),
        }
    }
    if let Some(w) = d.weight {
        parts.push(format!("weight `{w}`"));
    }
    parts.join(", ")
}

fn names(list: &[String]) -> String {
    if list.is_empty() {
        "none".to_owned()
    } else {
        list.iter()
            .map(|n| format!("[`{n}`]({n}.md)"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn page(name: &str, style: &Style, text: &str) -> String {
    let mut out = format!("# Style: {name}\n\n");
    if let Some(description) = &style.description {
        let _ = writeln!(out, "{description}\n");
    }
    let _ = writeln!(out, "- Extends: {}", names(&style.extends));
    let _ = writeln!(out, "- Conflicts with: {}", names(&style.conflicts));
    let _ = writeln!(
        out,
        "- Enable it with `styles = [\"{name}\"]` in `slopvac.toml`.\n"
    );
    let notes = notes(text);
    if !notes.is_empty() {
        let _ = writeln!(out, "## Notes\n\n{notes}\n");
    }
    let cats = &style.categories;
    if !cats.is_empty() {
        out.push_str("## Categories\n\n| Category | Settings |\n| --- | --- |\n");
        for (k, c) in cats {
            let _ = writeln!(out, "| `{k}` | {} |", dials(&c.dials()));
        }
        out.push('\n');
    }
    let rules = &style.rules;
    if !rules.is_empty() {
        out.push_str("## Rules\n\n| Rule | Level |\n| --- | --- |\n");
        for (k, l) in rules {
            let _ = writeln!(out, "| `{k}` | `{}` |", level(*l));
        }
        out.push('\n');
    }
    out
}

/// One page per bundled preset plus `styles/index.md`.
///
/// # Panics
/// If a bundled preset is malformed; the crate's tests prevent that.
#[must_use]
pub fn reference_pages() -> Vec<(String, String)> {
    let mut pages = Vec::new();
    let mut index = String::from(
        "# Styles\n\nA style preset sets rule levels, category dials and terms settings. \
         List presets in order in `styles`; the last listed wins per setting. A preset that \
         declares a conflict with another active preset produces a warning that names the \
         winner, and `--strict-styles` turns it into an error.\n\n| Style | Description |\n| --- | --- |\n",
    );
    for (name, text) in BUILTIN {
        let style = Style::parse(name, text).expect("bundled preset is valid");
        let description = style.description.as_deref().unwrap_or_default();
        let _ = writeln!(index, "| [`{name}`]({name}.md) | {description} |");
        pages.push((format!("styles/{name}.md"), page(name, &style, text)));
    }
    pages.insert(0, ("styles/index.md".to_owned(), index));
    pages
}
