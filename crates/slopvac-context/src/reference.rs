//! Generated reference pages for the built-in genres.
//!
//! [`reference_pages`] returns `(path, markdown)` pairs relative to the docs
//! site root, like the style reference in `slopvac-styles`.

use std::fmt::Write as _;

use crate::{
    Budget, CategoryTable, GenreRegistry, GenreSpec, RuleLevel, Scalar, Signal, SignalKind,
    builtin_genres,
};

fn level(l: RuleLevel) -> &'static str {
    match l {
        RuleLevel::Error => "error",
        RuleLevel::Warning => "warning",
        RuleLevel::Suggestion => "suggestion",
        RuleLevel::Off => "off",
    }
}

fn scalar(s: &Scalar) -> String {
    match s {
        Scalar::Bool(b) => b.to_string(),
        Scalar::Int(i) => i.to_string(),
        Scalar::Float(f) => f.to_string(),
        Scalar::Str(s) => s.clone(),
    }
}

fn signal(s: &Signal) -> String {
    let weight = s.weight;
    let glob = s.glob.as_deref().unwrap_or_default();
    let pattern = s.pattern.as_deref().unwrap_or_default();
    match s.kind {
        SignalKind::Path => format!("Path matches the glob `{glob}` (weight {weight})"),
        SignalKind::Heading => format!("Heading matches `{pattern}` (weight {weight})"),
        SignalKind::Text => format!("Text matches `{pattern}` (weight {weight})"),
        SignalKind::FrontMatter => {
            let key = s.key.as_deref().unwrap_or_default();
            s.equals.as_ref().map_or_else(
                || format!("Front matter `{key}` is present (weight {weight})"),
                |v| {
                    format!(
                        "Front matter `{key}` equals `{}` (weight {weight})",
                        scalar(v)
                    )
                },
            )
        }
    }
}

fn category(name: &str, t: &CategoryTable) -> String {
    let mut dials = Vec::new();
    if let Some(l) = t.severity {
        dials.push(format!("severity `{}`", level(l)));
    }
    if let Some(l) = t.minimum_severity {
        dials.push(format!("minimum severity `{}`", level(l)));
    }
    match t.max_per_100_words {
        Some(Budget::Limit(n)) => dials.push(format!("budget {n} findings per 100 words")),
        Some(Budget::None(_)) => dials.push("no budget".to_owned()),
        None => {}
    }
    if let Some(w) = t.weight {
        dials.push(format!("weight {w}"));
    }
    format!("- `{name}`: {}", dials.join(", "))
}

#[expect(
    clippy::let_underscore_must_use,
    reason = "writing to a String cannot fail"
)]
fn index(reg: &GenreRegistry) -> String {
    let mut out = String::from(
        "# Genres\n\n\
         A genre describes what a document is for. slopvac detects the genre from signals in \
         the document. Config or front matter can set it instead. A genre can set a profile and \
         styles. It can also set category settings, rule levels and context tags. A genre can \
         extend other genres and inherit their settings.\n\n",
    );
    // Ids come from the registry, so every one has a spec.
    for (id, spec) in reg
        .ids()
        .into_iter()
        .filter_map(|id| Some((id, reg.spec(id)?)))
    {
        let _ = write!(out, "- [`{id}`]({id}.md)");
        if let Some(d) = &spec.description {
            let _ = write!(out, ": {d}");
        }
        out.push('\n');
    }
    out
}

#[expect(
    clippy::let_underscore_must_use,
    reason = "writing to a String cannot fail"
)]
fn list_or(out: &mut String, items: &[String], none: &str) {
    if items.is_empty() {
        let _ = writeln!(out, "{none}");
    }
    for i in items {
        let _ = writeln!(out, "{i}");
    }
}

/// The page of one genre, or `None` when the registry cannot resolve it. A registry
/// validates every chain when it is built, so that does not happen.
#[expect(
    clippy::let_underscore_must_use,
    reason = "writing to a String cannot fail"
)]
fn page(reg: &GenreRegistry, spec: &GenreSpec) -> Option<String> {
    let id = &spec.id;
    let title = spec.title.as_deref().unwrap_or(id);
    let mut out = format!("# {title}\n\n");
    if let Some(d) = &spec.description {
        let _ = write!(out, "{d}\n\n");
    }
    if let Some(s) = &spec.surfaces {
        let _ = write!(out, "Typical documents: {s}\n\n");
    }
    out.push_str("## Settings\n\n");
    let _ = writeln!(out, "- Id: `{id}`");
    if spec.extends.is_empty() {
        out.push_str("- Extends: none\n");
    } else {
        let direct: Vec<String> = spec.extends.iter().map(|e| format!("`{e}`")).collect();
        let _ = writeln!(out, "- Extends: {}", direct.join(", "));
        let chain: Vec<String> = reg
            .chain(id)
            .ok()?
            .iter()
            .map(|s| format!("`{}`", s.id))
            .collect();
        let _ = writeln!(out, "- Resolved chain, lowest first: {}", chain.join(" → "));
    }
    match reg.profile(id).ok()? {
        Some(p) => {
            let _ = writeln!(out, "- Profile: `{}`", p.as_str());
        }
        None => out.push_str("- Profile: none (the configured profile applies)\n"),
    }
    out.push_str("\n## Detection signals\n\n");
    match &spec.detect {
        None => out.push_str("None. Config or front matter choose this genre.\n"),
        Some(d) => {
            let _ = write!(out, "Minimum score: {}\n\n", d.min_score);
            for s in &d.signals {
                let _ = writeln!(out, "- {}", signal(s));
            }
        }
    }
    out.push_str("\n## Styles\n\n");
    let styles: Vec<String> = spec.styles.iter().map(|s| format!("- `{s}`")).collect();
    list_or(&mut out, &styles, "This genre sets no styles.");
    out.push_str("\n## Category settings\n\n");
    let categories: Vec<String> = reg
        .categories(id)
        .ok()?
        .iter()
        .map(|(name, t)| category(name, t))
        .collect();
    list_or(
        &mut out,
        &categories,
        "This genre sets no category settings.",
    );
    out.push_str("\n## Rule levels\n\n");
    let rules: Vec<String> = reg
        .rules(id)
        .ok()?
        .iter()
        .map(|(rule, l)| format!("- `{rule}`: `{}`", level(*l)))
        .collect();
    list_or(&mut out, &rules, "This genre sets no rule levels.");
    out.push_str("\n## Context tags\n\n");
    let contexts: Vec<String> = reg
        .contexts(id)
        .ok()?
        .iter()
        .map(|c| format!("- `{c}`"))
        .collect();
    list_or(&mut out, &contexts, "This genre sets no context tags.");
    Some(out)
}

/// `genres/index.md` and one `genres/<id>.md` per built-in genre.
#[must_use]
pub fn reference_pages() -> Vec<(String, String)> {
    // The tests validate the built-in genres, so the empty fallback does not occur.
    let Ok(reg) = builtin_genres() else {
        return Vec::new();
    };
    let mut pages = vec![("genres/index.md".to_owned(), index(&reg))];
    for id in reg.ids() {
        if let Some(spec) = reg.spec(id)
            && let Some(text) = page(&reg, spec)
        {
            pages.push((format!("genres/{id}.md"), text));
        }
    }
    pages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_genres;

    // The built-in genres set no categories or styles, so a custom registry covers them.
    #[test]
    fn a_page_shows_styles_and_category_dials() {
        let specs = parse_genres(
            "- schema_version: 3\n  id: base\n  title: Base genre\n  surfaces: READMEs\n  \
               styles: [ste, google]\n  categories:\n    ai-register: warning\n    \
               ste-verbs: { minimum_severity: suggestion, max_per_100_words: 0.4, weight: 2 }\n    \
               ste-words: { max_per_100_words: none }\n",
        )
        .unwrap();
        let reg = GenreRegistry::new(specs, "base").unwrap();
        let page = page(&reg, reg.spec("base").unwrap()).unwrap();
        assert!(page.starts_with("# Base genre\n"));
        assert!(page.contains("Typical documents: READMEs\n"));
        assert!(page.contains("## Styles\n\n- `ste`\n- `google`\n"));
        assert!(page.contains("- `ai-register`: severity `warning`\n"));
        assert!(page.contains(
            "- `ste-verbs`: minimum severity `suggestion`, budget 0.4 findings per 100 words, \
             weight 2\n"
        ));
        assert!(page.contains("- `ste-words`: no budget\n"));
    }
}
