//! Generated reference pages for the built-in genres.
//!
//! [`reference_pages`] returns `(path, markdown)` pairs relative to the docs
//! site root, like the style reference in `slopvac-styles`.

use std::fmt::Write as _;

use crate::{DetectSignal, GenreRegistry, GenreSpec, builtin_genres};

fn signal(s: &DetectSignal) -> String {
    let weight = s.weight;
    if let Some(h) = &s.heading {
        format!("Heading matches `{h}` (weight {weight})")
    } else if let Some(p) = &s.path {
        format!("Path matches `{p}` (weight {weight})")
    } else if let Some(f) = &s.frontmatter {
        match &f.matches {
            Some(m) => format!("Front matter `{}` matches `{m}` (weight {weight})", f.key),
            None => format!("Front matter `{}` is present (weight {weight})", f.key),
        }
    } else {
        format!("Unspecified signal (weight {weight})")
    }
}

fn index(reg: &GenreRegistry) -> String {
    let mut out = String::from(
        "# Genres\n\n\
         A genre describes what a document is for. slopvac detects it from the path, headings \
         and front matter. Config or front matter can set it instead. Each genre sets a profile, rule \
         severities and context tags. Each page lists the detection signals and \
         the snippet that `slopvac prime write` prints.\n\n",
    );
    for id in reg.ids() {
        let spec = reg.spec(id).expect("ids come from the registry");
        let _ = writeln!(out, "- [`{id}`]({id}.md): {}", spec.description);
    }
    out
}

fn page(reg: &GenreRegistry, spec: &GenreSpec) -> String {
    let id = &spec.id;
    let chain: Vec<String> = reg
        .chain(id)
        .expect("a registry validates every chain")
        .iter()
        .map(|s| format!("`{}`", s.id))
        .collect();
    let mut out = format!("# {id}\n\n{}\n\n## Settings\n\n", spec.description);
    match &spec.extends {
        Some(parent) => {
            let _ = writeln!(out, "- Extends: `{parent}` (chain: {})", chain.join(" → "));
        }
        None => out.push_str("- Extends: none\n"),
    }
    if spec.is_abstract {
        out.push_str("- Abstract: never detected or chosen by default\n");
    }
    match reg.profile(id).expect("a registry validates every chain") {
        Some(p) => {
            let _ = writeln!(out, "- Profile: `{p}`");
        }
        None => out.push_str("- Profile: none (the configured profile applies)\n"),
    }
    let _ = write!(out, "- Detection threshold: {}\n\n", spec.threshold);
    out.push_str("## Detection signals\n\n");
    if spec.detect.is_empty() {
        out.push_str("None. Config, front matter or the default choose this genre.\n");
    }
    for s in &spec.detect {
        let _ = writeln!(out, "- {}", signal(s));
    }
    out.push_str("\n## Rule severities\n\n");
    let tolerances = reg
        .tolerances(id)
        .expect("a registry validates every chain");
    if tolerances.is_empty() {
        out.push_str("This genre sets no rule severities.\n");
    }
    for (rule, severity) in &tolerances {
        let _ = writeln!(out, "- `{rule}`: `{severity}`");
    }
    out.push_str("\n## Context tags\n\n");
    let contexts = reg.contexts(id).expect("a registry validates every chain");
    if contexts.is_empty() {
        out.push_str("This genre sets no context tags.\n");
    }
    for c in &contexts {
        let _ = writeln!(out, "- `{c}`");
    }
    out.push_str("\n## Prime snippet\n\n");
    match &spec.prime {
        Some(p) => {
            let _ = writeln!(out, "> {p}");
        }
        None => out.push_str("This genre has no prime snippet.\n"),
    }
    out
}

/// `genres/index.md` and one `genres/<id>.md` per built-in genre.
#[must_use]
pub fn reference_pages() -> Vec<(String, String)> {
    let reg = builtin_genres().expect("the built-in genres are valid");
    let mut pages = vec![("genres/index.md".to_string(), index(&reg))];
    for id in reg.ids() {
        let spec = reg.spec(id).expect("ids come from the registry");
        pages.push((format!("genres/{id}.md"), page(&reg, spec)));
    }
    pages
}
