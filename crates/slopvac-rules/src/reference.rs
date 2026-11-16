//! Generated reference pages for a loaded rule set.
//!
//! [`reference_pages`] returns `(path, markdown)` pairs relative to the docs site root:
//! `rules/index.md`, `rules/<category>/index.md` and `rules/<category>/<rule>.md`. The
//! published URLs are `rules/<category>/` and `rules/<category>/<rule>/`, the addresses the
//! report crate links to.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

use crate::{Citation, Example, Exception, LoadedCategory, LoadedRule, Match, Rule, RuleSet};

/// The levels one style preset assigns, by category id and by `<category>.<rule>` key.
///
/// The caller resolves the preset, so this crate does not depend on the styles crate.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PresetLevels {
    /// The preset name.
    pub name: String,
    /// Category level overrides.
    pub categories: BTreeMap<String, String>,
    /// Rule level overrides. They win over a category override.
    pub rules: BTreeMap<String, String>,
}

/// A serde value as plain text: a string as is, anything else as compact JSON.
fn word<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Catalogue prose as one Markdown line: HTML specials escaped, Unicode dashes as `--`.
fn prose(text: &str) -> String {
    one_line(text)
        .replace(['\u{2014}', '\u{2013}'], "--")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A message template with each `{slot}` shown as `<slot>` text.
fn message(text: &str) -> String {
    let text = prose(text);
    let mut out = String::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 1..];
        match tail.split_once('}') {
            Some((name, after))
                if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') =>
            {
                let _ = write!(out, "&lt;{name}&gt;");
                rest = after;
            }
            _ => {
                out.push('{');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

fn longest_run(text: &str, ch: char) -> usize {
    let mut best = 0;
    let mut run = 0;
    for c in text.chars() {
        run = if c == ch { run + 1 } else { 0 };
        best = best.max(run);
    }
    best
}

/// Inline code that survives backticks and a pipe inside a table.
fn code(text: &str) -> String {
    let text = text.replace('\n', " ");
    let fence = "`".repeat(longest_run(&text, '`') + 1);
    let pad = if text.starts_with(['`', ' ']) || text.ends_with(['`', ' ']) {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{text}{pad}{fence}")
}

/// A specimen as a fenced block longer than any backtick run it holds.
fn fenced(text: &str) -> String {
    let fence = "`".repeat((longest_run(text, '`') + 1).max(3));
    format!("{fence}text\n{}\n{fence}", text.trim_matches('\n'))
}

/// Rule data the page quotes. The data names the bad text on purpose, and the docs gate
/// lints the generated pages, so each quoted value sits in its own lint-disable region.
/// Narration around it stays outside. Callers leave a blank line before the region.
///
/// A blank line precedes the closing marker. The scanner carries unmatched backtick runs
/// from line to line and drops them at a blank line, so a code span that holds a lone
/// backtick cannot make it ignore the marker.
fn quoted(out: &mut String, body: &str) {
    let _ = writeln!(
        out,
        "<!-- slopvac-disable -->\n{}\n\n<!-- slopvac-enable -->",
        body.trim_end_matches('\n')
    );
}

/// A bullet list of inline-code items.
fn code_list<I: IntoIterator<Item = String>>(items: I) -> String {
    items
        .into_iter()
        .map(|i| format!("- {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn comparison_phrase(comparison: &str) -> &str {
    match comparison {
        "gt" => "greater than",
        "gte" => "at least",
        "lt" => "less than",
        "lte" => "at most",
        other => other,
    }
}

fn kind_name(m: &Match) -> String {
    format!("{:?}", m.kind()).to_lowercase()
}

fn names<T: Serialize>(items: &[T]) -> String {
    items
        .iter()
        .map(|i| code(&word(i)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn flags(ignore_case: bool, all_caps: bool) -> String {
    let mut parts = vec![if ignore_case {
        "ignores case"
    } else {
        "case-sensitive"
    }];
    if all_caps {
        parts.push("reports all-capitals text");
    }
    parts.join(", ")
}

fn match_section(out: &mut String, m: &Match) {
    let _ = writeln!(out, "\n## Match\n\nKind: `{}`.\n", kind_name(m));
    match m {
        Match::Tokens(t) => {
            let _ = writeln!(
                out,
                "Literal phrases ({}):\n",
                flags(t.ignore_case, t.match_all_caps)
            );
            let items = t.tokens.as_slice().iter().map(|token| code(&word(token)));
            quoted(out, &code_list(items));
        }
        Match::Pattern(p) => {
            let _ = writeln!(
                out,
                "PCRE2 pattern ({}):\n",
                flags(p.ignore_case, p.match_all_caps)
            );
            quoted(out, &code(p.pattern.as_str()));
            if let Some(placeholders) = &p.placeholders {
                let _ = writeln!(out, "\nPlaceholders and where their values come from:\n");
                let items = placeholders.iter().map(|(name, source)| {
                    format!("{}: {}", code(&word(name)), code(&word(source)))
                });
                quoted(out, &code_list(items));
            }
        }
        Match::Substitution(s) => {
            let _ = writeln!(
                out,
                "Replacements, each written as pattern -> replacement ({}):\n",
                flags(s.ignore_case, s.match_all_caps)
            );
            let items = s
                .substitutions
                .entries()
                .iter()
                .map(|(from, to)| format!("{} -> {}", code(from.as_str()), code(to)));
            quoted(out, &code_list(items));
        }
        Match::Metric(m) => {
            let _ = writeln!(
                out,
                "Reports metric {} when it is {} {}.",
                code(&word(&m.metric)),
                comparison_phrase(&word(&m.comparison)),
                m.threshold
            );
        }
        Match::Structure(s) => {
            let _ = writeln!(out, "Structure check {}.", code(&word(&s.check)));
        }
        Match::Capitalization(c) => {
            let _ = writeln!(out, "Capitalization style {}.", code(&word(&c.style)));
            if let Some(allowed) = &c.allowed_words {
                let _ = writeln!(out, "\nAllowed words:\n");
                let items = allowed.as_slice().iter().map(|w| code(&word(w)));
                quoted(out, &code_list(items));
            }
        }
        Match::Vocabulary(v) => {
            let _ = writeln!(out, "Vocabulary check {}.", code(&word(&v.check)));
        }
        Match::Judgement(j) => {
            let _ = writeln!(out, "Reviewer question:\n");
            quoted(out, &prose(&word(&j.question)));
        }
    }
}

fn example(out: &mut String, number: usize, ex: &Example) {
    let note = ex.note.as_deref().map(one_line).filter(|n| !n.is_empty());
    let keep = note.as_deref().is_some_and(|n| n.starts_with("preserve:"));
    let label = if keep { "kept as written" } else { "flagged" };
    let _ = writeln!(out, "\n### Example {number}: {label}\n");
    quoted(out, &fenced(&ex.bad));
    match ex.good.as_deref() {
        _ if keep => {}
        None => out.push_str("\nNo rewrite is given.\n"),
        Some(good) if good.trim().is_empty() => out.push_str("\nRewrite: delete the text.\n"),
        Some(good) => {
            out.push_str("\nRewrite:\n\n");
            quoted(out, &fenced(good));
        }
    }
    if let Some(note) = note {
        let (label, text) = match note.strip_prefix("preserve:") {
            Some(rest) => ("Reason", rest.trim()),
            None => ("Note", note.as_str()),
        };
        let _ = writeln!(out, "\n{label}:\n");
        quoted(out, &prose(text));
    }
}

fn exception(e: &Exception) -> String {
    let mut line = format!("- **{}**", word(&e.name));
    if let Some(d) = e.description.as_deref().filter(|d| !d.trim().is_empty()) {
        let _ = write!(line, ": {}", prose(d));
    }
    if let Some(detector) = &e.detector {
        let _ = write!(line, " {}", code(&format!("detector: {}", word(detector))));
    }
    line
}

fn citation(out: &mut String, c: &Citation) {
    out.push_str("\n## Citation\n\nSource:\n\n");
    quoted(out, &prose(&word(&c.source)));
    if let Some(r) = &c.ste_ref {
        let _ = writeln!(out, "\nSTE rule: {}", code(&word(r)));
    }
    if let Some(r) = &c.orwell_ref {
        let _ = writeln!(out, "\nOrwell rule:\n");
        quoted(out, &prose(r));
    }
    if let Some(url) = &c.url {
        out.push('\n');
        quoted(out, &format!("<{url}>"));
    }
    if let Some(note) = c.note.as_deref().filter(|n| !n.trim().is_empty()) {
        let _ = writeln!(out, "\nNote:\n");
        quoted(out, &prose(note));
    }
}

fn severity_section(out: &mut String, category: &str, rule: &Rule, presets: &[PresetLevels]) {
    if presets.is_empty() {
        return;
    }
    let key = format!("{category}.{}", word(&rule.id));
    out.push_str("\n## Severity by style\n\nThe shipped severity applies unless a style preset sets another level. A rule entry beats a category entry.\n\n| Style | Severity |\n| --- | --- |\n");
    let _ = writeln!(out, "| none (shipped) | `{}` |", word(&rule.severity));
    for preset in presets {
        let level = preset
            .rules
            .get(&key)
            .or_else(|| preset.categories.get(category))
            .cloned()
            .unwrap_or_else(|| word(&rule.severity));
        let _ = writeln!(out, "| `{}` | `{level}` |", preset.name);
    }
}

fn rule_page(category: &LoadedCategory, loaded: &LoadedRule, presets: &[PresetLevels]) -> String {
    let rule = loaded.rule();
    let cat = word(category.id());
    let mut out = format!(
        "# {}\n\nRule `{cat}.{}` in the [{}](index.md) category.\n\n| Field | Value |\n| --- | --- |\n",
        prose(&word(&rule.name)),
        word(&rule.id),
        prose(&word(category.title()))
    );
    let _ = writeln!(out, "| Kind | {} |", kind_name(&rule.match_));
    let _ = writeln!(out, "| Severity | `{}` |", word(&rule.severity));
    let _ = writeln!(out, "| Scope | {} |", word(&rule.scope));
    let _ = writeln!(out, "| Text type | {} |", word(&rule.text_type));
    let _ = writeln!(
        out,
        "| AI signal | {} (from {}) |",
        word(&rule.ai_signal),
        word(&rule.ai_signal_source)
    );
    if let Some(blocks) = &rule.exclude_blocks {
        let _ = writeln!(out, "| Skips blocks | {} |", names(blocks.as_slice()));
    }
    if let Some(gate) = &rule.contexts {
        if let Some(only) = &gate.only {
            let _ = writeln!(out, "| Only in contexts | {} |", names(only.as_slice()));
        }
        if let Some(skip) = &gate.skip {
            let _ = writeln!(out, "| Skips contexts | {} |", names(skip.as_slice()));
        }
    }
    if let Some(applies) = &rule.applies_to {
        if let Some(modes) = &applies.modes {
            let _ = writeln!(out, "| Applies in modes | {} |", names(modes.as_slice()));
        }
        if let Some(genres) = &applies.genres {
            let _ = writeln!(out, "| Applies to genres | {} |", names(genres.as_slice()));
        }
    }
    severity_section(&mut out, &cat, rule, presets);
    out.push_str("\n## Message\n\n");
    quoted(&mut out, &message(&word(&rule.message)));
    if let Some(fix) = rule.fix.as_deref().filter(|f| !f.trim().is_empty()) {
        out.push_str("\n## Fix\n\n");
        quoted(&mut out, &prose(fix));
    }
    match_section(&mut out, &rule.match_);
    if !rule.examples().is_empty() {
        out.push_str("\n## Examples\n");
        for (i, ex) in rule.examples().iter().enumerate() {
            example(&mut out, i + 1, ex);
        }
    }
    if !rule.exceptions().is_empty() {
        out.push_str("\n## Exceptions\n\n");
        quoted(
            &mut out,
            &rule
                .exceptions()
                .iter()
                .map(exception)
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    if !rule.allowlist().is_empty() {
        out.push_str("\n## Allowlist\n\nThese strings never fire:\n\n");
        let items = rule.allowlist().iter().map(|w| code(&word(w)));
        quoted(&mut out, &code_list(items));
    }
    citation(&mut out, &rule.citation);
    out
}

fn category_index(category: &LoadedCategory) -> String {
    let mut out = format!(
        "# {}\n\n{}\n\n",
        prose(&word(category.title())),
        prose(&word(category.description()))
    );
    let _ = writeln!(out, "- Weight: {}", category.weight().get());
    if !category.recommended_for().is_empty() {
        let _ = writeln!(
            out,
            "- Recommended for: {}",
            names(category.recommended_for())
        );
    }
    out.push('\n');
    for rule in sorted_rules(category) {
        let r = rule.rule();
        let _ = writeln!(
            out,
            "- [{}]({}.md): `{}.{}`",
            prose(&word(&r.name)),
            word(&r.id),
            word(category.id()),
            word(&r.id)
        );
    }
    out
}

fn sorted_rules(category: &LoadedCategory) -> Vec<&LoadedRule> {
    let mut rules: Vec<_> = category.rules().iter().collect();
    rules.sort_by_key(|r| word(&r.rule().id));
    rules
}

/// One page per rule and category, plus `rules/index.md`.
///
/// `presets` adds a per-style severity table to each rule page; pass none to show only the
/// shipped severity.
#[must_use]
pub fn reference_pages(set: &RuleSet, presets: &[PresetLevels]) -> Vec<(String, String)> {
    let mut categories: Vec<_> = set.categories().iter().collect();
    categories.sort_by_key(|c| word(c.id()));
    let mut index = String::from("# Rules\n\nOne page per rule, grouped by category.\n\n");
    let mut pages = Vec::new();
    for category in categories {
        let cat = word(category.id());
        for rule in sorted_rules(category) {
            pages.push((
                format!("rules/{cat}/{}.md", word(&rule.rule().id)),
                rule_page(category, rule, presets),
            ));
        }
        pages.push((format!("rules/{cat}/index.md"), category_index(category)));
        let _ = writeln!(
            index,
            "- [{}]({cat}/index.md): {} rules",
            prose(&word(category.title())),
            category.rules().len()
        );
    }
    pages.push(("rules/index.md".to_owned(), index));
    pages
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use slopvac_model::PackId;

    use super::*;
    use crate::CompileOptions;

    const DEMO: &str = r#"{
      "schema_version": 3, "id": "demo", "title": "Demo <category>", "description": "A demo.",
      "weight": 1.0,
      "rules": [{
        "id": "no-foo", "name": "Avoid foo", "message": "foo found: {match} -- use {replacement}",
        "severity": "error", "scope": "sentence",
        "match": {"kind": "pattern", "pattern": "foo"},
        "examples": [
          {"bad": "Use foo here.", "good": "Use bar here."},
          {"bad": "Choose foo, not baz.", "good": "Choose bar, not baz.", "note": "preserve: names a real alternative"},
          {"bad": "Remove foo", "good": "", "note": "The ending is dropped."}
        ],
        "exceptions": [
          {"name": "quoted", "description": "Inside a quotation.", "detector": "quotation"},
          {"name": "bare"}
        ],
        "ai_signal": "none", "ai_signal_source": "unmeasured",
        "citation": {"source": "Hand written \u2014 draft", "url": "https://example.test/foo"}
      }]
    }"#;

    /// A quoted value as the generator writes it: its own lint-disable region.
    fn q(body: &str) -> String {
        format!("<!-- slopvac-disable -->\n{body}\n\n<!-- slopvac-enable -->\n")
    }

    fn pages(presets: &[PresetLevels]) -> BTreeMap<String, String> {
        let set = RuleSet::from_sources(
            &PackId::new("demo").unwrap(),
            [(PathBuf::from("demo.json"), DEMO.to_owned())],
            &CompileOptions::default(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        reference_pages(&set, presets).into_iter().collect()
    }

    #[test]
    fn writes_a_root_index_a_category_index_and_a_page_per_rule() {
        let pages = pages(&[]);
        let paths: Vec<_> = pages.keys().map(String::as_str).collect();
        assert_eq!(
            paths,
            [
                "rules/demo/index.md",
                "rules/demo/no-foo.md",
                "rules/index.md"
            ]
        );
        assert!(pages["rules/demo/index.md"].contains("[Avoid foo](no-foo.md): `demo.no-foo`"));
        assert!(pages["rules/index.md"].contains("(demo/index.md): 1 rules"));
    }

    #[test]
    fn message_slots_and_titles_are_escaped_text_not_html() {
        let pages = pages(&[]);
        let page = &pages["rules/demo/no-foo.md"];
        assert!(page.contains("foo found: &lt;match&gt; -- use &lt;replacement&gt;"));
        assert!(!page.contains("<match>"));
        assert!(pages["rules/demo/index.md"].contains("# Demo &lt;category&gt;"));
    }

    #[test]
    fn preserved_empty_and_noted_examples_render_their_own_labels() {
        let page = &pages(&[])["rules/demo/no-foo.md"];
        assert!(page.contains("### Example 1: flagged"));
        let kept = page
            .split("### Example 2: kept as written")
            .nth(1)
            .and_then(|t| t.split("### Example 3").next())
            .expect("example 2 is kept as written");
        assert!(kept.contains(&format!("Reason:\n\n{}", q("names a real alternative"))));
        assert!(!kept.contains("Rewrite"));
        assert!(page.contains("Rewrite: delete the text."));
        assert!(page.contains(&format!("Note:\n\n{}", q("The ending is dropped."))));
    }

    #[test]
    fn exceptions_list_name_description_and_detector() {
        let page = &pages(&[])["rules/demo/no-foo.md"];
        assert!(page.contains(&q(
            "- **quoted**: Inside a quotation. `detector: quotation`\n- **bare**"
        )));
    }

    #[test]
    fn citation_dashes_become_ascii_and_the_url_is_linked() {
        let page = &pages(&[])["rules/demo/no-foo.md"];
        assert!(page.contains(&format!(
            "## Citation\n\nSource:\n\n{}",
            q("Hand written -- draft")
        )));
        assert!(page.contains(&q("<https://example.test/foo>")));
        assert!(!page.contains('\u{2014}'));
    }

    #[test]
    fn a_rule_entry_beats_a_category_entry_beats_the_shipped_severity() {
        let preset = |name: &str, cat: Option<&str>, rule: Option<&str>| PresetLevels {
            name: name.to_owned(),
            categories: cat
                .map(|l| ("demo".to_owned(), l.to_owned()))
                .into_iter()
                .collect(),
            rules: rule
                .map(|l| ("demo.no-foo".to_owned(), l.to_owned()))
                .into_iter()
                .collect(),
        };
        let page = &pages(&[
            preset("plain", None, None),
            preset("cat", Some("suggestion"), None),
            preset("both", Some("suggestion"), Some("off")),
        ])["rules/demo/no-foo.md"];
        assert!(page.contains("| `plain` | `error` |"));
        assert!(page.contains("| `cat` | `suggestion` |"));
        assert!(page.contains("| `both` | `off` |"));
    }

    #[test]
    fn without_presets_only_the_shipped_severity_is_shown() {
        let page = &pages(&[])["rules/demo/no-foo.md"];
        assert!(page.contains("| Severity | `error` |"));
        assert!(!page.contains("Severity by style"));
    }

    const KINDS: &str = r#"{
      "schema_version": 3, "id": "kinds", "title": "Kinds", "description": "Every match kind.",
      "weight": 2.5, "recommended_for": ["reference", "internal"],
      "rules": [
        {"id": "tok", "name": "Tokens", "message": "m", "severity": "warning", "scope": "prose",
         "match": {"kind": "tokens", "tokens": ["as noted", "recall that"]},
         "ai_signal": "none", "ai_signal_source": "unmeasured", "citation": {"source": "s"}},
        {"id": "pat", "name": "Pattern", "message": "m", "severity": "warning", "scope": "prose",
         "match": {"kind": "pattern", "pattern": "a`b|c", "ignore_case": false, "match_all_caps": true},
         "examples": [{"bad": "Wrap ``` and a`b | c", "good": "Wrap it"}],
         "fix": "Delete the `a`b` run, or use `x|y`.",
         "ai_signal": "none", "ai_signal_source": "unmeasured", "citation": {"source": "s"}},
        {"id": "sub", "name": "Substitution", "message": "m", "severity": "warning", "scope": "prose",
         "match": {"kind": "substitution", "substitutions": {"\\bfoo\\b": "bar", "baz": "q|r"}},
         "ai_signal": "none", "ai_signal_source": "unmeasured", "citation": {"source": "s"}},
        {"id": "met", "name": "Metric", "message": "m", "severity": "warning", "scope": "document",
         "match": {"kind": "metric", "metric": "adjectives_per_noun", "threshold": 0.5, "comparison": "gte"},
         "ai_signal": "none", "ai_signal_source": "unmeasured", "citation": {"source": "s"}},
        {"id": "str", "name": "Structure", "message": "m", "severity": "warning", "scope": "document",
         "match": {"kind": "structure", "check": "heading-hierarchy"},
         "ai_signal": "none", "ai_signal_source": "unmeasured", "citation": {"source": "s"}},
        {"id": "voc", "name": "Vocabulary", "message": "m", "severity": "warning", "scope": "prose",
         "match": {"kind": "vocabulary", "check": "verb-form-not-listed"},
         "ai_signal": "none", "ai_signal_source": "unmeasured", "citation": {"source": "s"}},
        {"id": "jud", "name": "Judgement", "message": "m", "severity": "suggestion", "scope": "document",
         "match": {"kind": "judgement", "question": "Is a < b & c?"},
         "ai_signal": "weak", "ai_signal_source": "catalog", "citation": {"source": "s"}},
        {"id": "plc", "name": "Placeholder", "message": "m", "severity": "warning", "scope": "prose",
         "match": {"kind": "pattern", "pattern": "\\b(?!(?:ALLOWED)\\b)[A-Z][a-z]+",
                   "placeholders": {"ALLOWED": "allowed-words", "OTHER": "allowed-words"},
                   "ignore_case": false},
         "ai_signal": "none", "ai_signal_source": "unmeasured", "citation": {"source": "s"}},
        {"id": "cap", "name": "Capitalization", "message": "m", "severity": "warning", "scope": "heading",
         "match": {"kind": "capitalization", "style": "sentence", "allowed_words": ["iPhone", "API"]},
         "ai_signal": "none", "ai_signal_source": "unmeasured", "citation": {"source": "s"}},
        {"id": "cit", "name": "Cited", "message": "m", "severity": "warning", "scope": "prose",
         "text_type": "procedural",
         "match": {"kind": "tokens", "tokens": ["x"]},
         "ai_signal": "strong", "ai_signal_source": "measured",
         "citation": {"source": "Src book", "ste_ref": "9:4.2", "orwell_ref": "Rule 3",
                      "url": "https://example.test/cit", "note": "Why this exists."}},
        {"id": "gate", "name": "Gated", "message": "m", "severity": "warning", "scope": "prose",
         "match": {"kind": "tokens", "tokens": ["x"]},
         "exclude_blocks": ["code", "table"],
         "contexts": {"only": ["docs"], "skip": ["quote", "legal"]},
         "applies_to": {"modes": ["code-comments"], "genres": ["reference"]},
         "allowlist": ["alpha", "be`ta"],
         "ai_signal": "none", "ai_signal_source": "unmeasured", "citation": {"source": "s"}}
      ]
    }"#;

    fn kinds_pages() -> BTreeMap<String, String> {
        let set = RuleSet::from_sources(
            &PackId::new("demo").unwrap(),
            [(PathBuf::from("kinds.json"), KINDS.to_owned())],
            &CompileOptions::default(),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        reference_pages(&set, &[]).into_iter().collect()
    }

    fn kinds(rule: &str) -> String {
        kinds_pages()
            .remove(&format!("rules/kinds/{rule}.md"))
            .unwrap()
    }

    /// The lines of `page` outside and inside lint-disable regions.
    fn split_regions(page: &str) -> (String, String) {
        let (mut outside, mut inside) = (String::new(), String::new());
        let mut open = false;
        for line in page.lines() {
            match line {
                "<!-- slopvac-disable -->" => {
                    assert!(!open, "nested disable in:\n{page}");
                    open = true;
                }
                "<!-- slopvac-enable -->" => {
                    assert!(open, "enable without disable in:\n{page}");
                    open = false;
                }
                _ => {
                    let side = if open { &mut inside } else { &mut outside };
                    side.push_str(line);
                    side.push('\n');
                }
            }
        }
        assert!(!open, "unclosed disable in:\n{page}");
        (outside, inside)
    }

    #[test]
    fn template_narration_stays_outside_every_disable_region() {
        let narration = [
            ("pat", "## Message"),
            ("pat", "## Fix"),
            (
                "pat",
                "PCRE2 pattern (case-sensitive, reports all-capitals text):",
            ),
            ("pat", "### Example 1: flagged"),
            ("pat", "Rewrite:"),
            ("tok", "Literal phrases (ignores case):"),
            (
                "sub",
                "Replacements, each written as pattern -> replacement (ignores case):",
            ),
            ("plc", "Placeholders and where their values come from:"),
            ("cap", "Capitalization style `sentence`."),
            ("cap", "Allowed words:"),
            ("jud", "Reviewer question:"),
            ("gate", "These strings never fire:"),
            ("gate", "## Allowlist"),
            ("tok", "Source:"),
            ("cit", "Orwell rule:"),
            ("cit", "STE rule: `9:4.2`"),
            ("cit", "Note:"),
            ("tok", "Rule `kinds.tok` in the"),
        ];
        for (rule, text) in narration {
            let (outside, inside) = split_regions(&kinds(rule));
            assert!(outside.contains(text), "{text:?} not outside in {rule}");
            assert!(!inside.contains(text), "{text:?} inside a region in {rule}");
        }
        let (outside, inside) = split_regions(&pages(&[])["rules/demo/no-foo.md"]);
        for text in [
            "Note:",
            "Reason:",
            "Rewrite: delete the text.",
            "### Example 3: flagged",
        ] {
            assert!(outside.contains(text) && !inside.contains(text), "{text:?}");
        }
    }

    #[test]
    fn quoted_rule_data_sits_inside_a_disable_region() {
        let quoted = [
            ("pat", "a`b|c"),
            ("pat", "Delete the `a`b` run, or use `x|y`."),
            ("pat", "Wrap ``` and a`b | c"),
            ("pat", "Wrap it"),
            ("tok", "as noted"),
            ("sub", "q|r"),
            ("plc", "ALLOWED"),
            ("cap", "iPhone"),
            ("jud", "Is a &lt; b &amp; c?"),
            ("gate", "be`ta"),
            ("cit", "Src book"),
            ("cit", "Rule 3"),
            ("cit", "<https://example.test/cit>"),
            ("cit", "Why this exists."),
        ];
        for (rule, text) in quoted {
            let (outside, inside) = split_regions(&kinds(rule));
            assert!(inside.contains(text), "{text:?} not inside in {rule}");
            assert!(
                !outside.contains(text),
                "{text:?} outside a region in {rule}"
            );
        }
        let (outside, inside) = split_regions(&pages(&[])["rules/demo/no-foo.md"]);
        for text in [
            "foo found: &lt;match&gt;",
            "Hand written -- draft",
            "https://example.test/foo",
            "The ending is dropped.",
            "names a real alternative",
            "Inside a quotation.",
        ] {
            assert!(inside.contains(text) && !outside.contains(text), "{text:?}");
        }
    }

    #[test]
    fn citation_references_and_note_have_their_label_outside_and_value_quoted() {
        let page = kinds("cit");
        let (outside, inside) = split_regions(&page);
        // The STE reference is an identifier the template formats, so label and value stay
        // outside every region.
        assert!(outside.contains("\nSTE rule: `9:4.2`\n"));
        assert!(!inside.contains("9:4.2"));
        assert!(page.contains(&format!("Orwell rule:\n\n{}", q("Rule 3"))));
        assert!(page.contains(&format!("Note:\n\n{}", q("Why this exists."))));
    }

    #[test]
    fn rule_page_shows_text_type_and_ai_signal_with_its_source() {
        let page = kinds("cit");
        assert!(page.contains("| Text type | procedural |\n"));
        assert!(page.contains("| AI signal | strong (from measured) |\n"));
        // A rule that sets neither shows the defaults.
        let plain = kinds("tok");
        assert!(plain.contains("| Text type | any |\n"));
        assert!(plain.contains("| AI signal | none (from unmeasured) |\n"));
    }

    #[test]
    fn category_index_shows_weight_and_recommended_genres() {
        let index = &kinds_pages()["rules/kinds/index.md"];
        assert!(index.contains("- Weight: 2.5\n"));
        assert!(index.contains("- Recommended for: `reference`, `internal`\n"));
        // A category without genres omits the line.
        assert!(!pages(&[])["rules/demo/index.md"].contains("Recommended for"));
        assert!(pages(&[])["rules/demo/index.md"].contains("- Weight: 1\n"));
    }

    #[test]
    fn every_closing_marker_follows_a_blank_line() {
        for page in kinds_pages().values().chain(pages(&[]).values()) {
            let lines: Vec<_> = page.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if *line == "<!-- slopvac-enable -->" {
                    assert_eq!(
                        lines[i - 1],
                        "",
                        "no blank line before the marker in:\n{page}"
                    );
                }
            }
        }
    }

    #[test]
    fn pattern_placeholders_list_each_name_with_its_source() {
        assert!(kinds("plc").contains(&q(
            "- `ALLOWED`: `allowed-words`\n- `OTHER`: `allowed-words`"
        )));
        assert!(!kinds("pat").contains("Placeholders"));
    }

    #[test]
    fn capitalization_match_shows_the_style_and_each_allowed_word() {
        let page = kinds("cap");
        assert!(page.contains("Kind: `capitalization`.\n\nCapitalization style `sentence`.\n"));
        assert!(page.contains(&format!("Allowed words:\n\n{}", q("- `iPhone`\n- `API`"))));
    }

    #[test]
    fn fix_guidance_is_shown_in_its_own_section() {
        let page = kinds("pat");
        assert!(page.contains(&format!(
            "\n## Fix\n\n{}",
            q("Delete the `a`b` run, or use `x|y`.")
        )));
        assert!(!kinds("tok").contains("## Fix"));
    }

    #[test]
    fn tokens_match_lists_each_phrase() {
        let page = kinds("tok");
        assert!(page.contains("Kind: `tokens`."));
        assert!(page.contains(&format!(
            "Literal phrases (ignores case):\n\n{}",
            q("- `as noted`\n- `recall that`")
        )));
    }

    #[test]
    fn pattern_match_shows_the_pattern_and_flags_with_backticks_and_pipes_intact() {
        assert!(kinds("pat").contains(&format!(
            "PCRE2 pattern (case-sensitive, reports all-capitals text):\n\n{}",
            q("``a`b|c``")
        )));
    }

    #[test]
    fn substitution_match_lists_each_replacement() {
        assert!(kinds("sub").contains(&format!(
            "Replacements, each written as pattern -> replacement (ignores case):\n\n{}",
            q("- `\\bfoo\\b` -> `bar`\n- `baz` -> `q|r`")
        )));
    }

    #[test]
    fn metric_match_states_the_threshold_in_words() {
        assert!(
            kinds("met")
                .contains("Reports metric `adjectives_per_noun` when it is at least 0.5.\n")
        );
    }

    #[test]
    fn structure_vocabulary_and_judgement_matches_name_their_check_or_question() {
        assert!(kinds("str").contains("Structure check `heading-hierarchy`.\n"));
        assert!(kinds("voc").contains("Vocabulary check `verb-form-not-listed`.\n"));
        assert!(kinds("jud").contains(&format!(
            "Reviewer question:\n\n{}",
            q("Is a &lt; b &amp; c?")
        )));
    }

    #[test]
    fn gating_metadata_appears_as_field_rows() {
        let page = kinds("gate");
        for row in [
            "| Skips blocks | `code`, `table` |\n",
            "| Only in contexts | `docs` |\n",
            "| Skips contexts | `quote`, `legal` |\n",
            "| Applies in modes | `code-comments` |\n",
            "| Applies to genres | `reference` |\n",
        ] {
            assert!(page.contains(row), "missing {row:?} in:\n{page}");
        }
        assert!(!kinds("tok").contains("Skips blocks"));
    }

    #[test]
    fn allowlist_entries_are_listed_as_code() {
        assert!(kinds("gate").contains(&format!(
            "## Allowlist\n\nThese strings never fire:\n\n{}",
            q("- `alpha`\n- ``be`ta``")
        )));
        assert!(!kinds("tok").contains("## Allowlist"));
    }

    #[test]
    fn a_specimen_with_backticks_is_fenced_by_a_longer_run() {
        let page = kinds("pat");
        assert!(page.contains(&q("````text\nWrap ``` and a`b | c\n````")));
    }
}
