//! `slopvac rules` and `slopvac explain`: list and show the shipped rules
//! (compat rows `cmd:rules`, `cmd:explain`, `opt:rules:*`, `opt:explain:*`,
//! `behavior:inspection-filters`, `behavior:inspection-formats`).
//!
//! Output order is deterministic: category, then rule, as the pack lists them.

use std::fmt::Write as _;
use std::path::PathBuf;

use clap::ArgMatches;
use serde::Serialize;
use serde_json::{Value, json};
use slopvac_model::GenreId;
use slopvac_report::RenderOptions;
use slopvac_rules::{Example, LoadedCategory, LoadedRule, Match, MatchKind, RuleSet};

use crate::errors::CliError;
use crate::exit;
use crate::redact;
use crate::rule_catalog;

/// Generated catalogue prose is not written to the lint profile, so Markdown output
/// wraps it in these directives, as 2.x did.
const DISABLE: &str = "<!-- slopvac-disable -->";
const ENABLE: &str = "<!-- slopvac-enable -->";

/// The `--kind` values and the match kinds they select.
pub const KINDS: [(&str, MatchKind); 8] = [
    ("tokens", MatchKind::Tokens),
    ("pattern", MatchKind::Pattern),
    ("substitution", MatchKind::Substitution),
    ("metric", MatchKind::Metric),
    ("structure", MatchKind::Structure),
    ("capitalization", MatchKind::Capitalization),
    ("vocabulary", MatchKind::Vocabulary),
    ("judgement", MatchKind::Judgement),
];

/// The `--kind` names, for the clap value parser.
#[must_use]
pub fn kind_names() -> Vec<&'static str> {
    KINDS.iter().map(|(name, _)| *name).collect()
}

fn kind_name(kind: MatchKind) -> &'static str {
    KINDS
        .iter()
        .find(|(_, k)| *k == kind)
        .map_or("unknown", |(name, _)| *name)
}

/// The serde name of a plain enum value such as a severity.
fn word<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(s)) => s,
        _ => String::new(),
    }
}

/// The part of a message a reader sees: slots read as `<name>`, not `{name}`.
fn message_text(message: &str) -> String {
    message.replace('{', "<").replace('}', ">")
}

fn clean(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn load(m: &ArgMatches) -> Result<RuleSet, CliError> {
    if let Some(path) = m.get_one::<PathBuf>("config")
        && !path.is_file()
    {
        return Err(CliError::new(
            "config-not-found",
            format!(
                "--config {}: no such file",
                redact::shown_path(path, &std::env::current_dir().unwrap_or_default())
            ),
        ));
    }
    rule_catalog::core()
}

fn url_of(rule: &LoadedRule) -> Option<String> {
    RenderOptions::default().rule_url(rule.category().as_str(), rule.id().local())
}

fn source_of(rule: &LoadedRule) -> String {
    let c = &rule.rule().citation;
    c.ste_ref
        .as_ref()
        .map(ToString::to_string)
        .or_else(|| c.orwell_ref.clone())
        .unwrap_or_else(|| c.source.to_string())
}

struct Filter {
    categories: Vec<String>,
    kind: Option<MatchKind>,
    genre: Option<GenreId>,
    include_excluded: bool,
}

impl Filter {
    fn from_matches(m: &ArgMatches) -> Result<Self, CliError> {
        let genre = m
            .get_one::<String>("genre")
            .map(|g| {
                GenreId::new(g.clone()).map_err(|_| {
                    CliError::new("usage", format!("--genre {g:?} is not a genre id."))
                })
            })
            .transpose()?;
        let kind = m
            .get_one::<String>("kind")
            .and_then(|k| KINDS.iter().find(|(name, _)| name == k).map(|(_, k)| *k));
        Ok(Self {
            categories: m
                .get_many::<String>("category")
                .map(|v| v.cloned().collect())
                .unwrap_or_default(),
            kind,
            genre,
            include_excluded: m.get_flag("include-excluded"),
        })
    }

    /// A rule is kept when its category is recommended for the genre. A rule that
    /// names other genres in `applies_to` is dropped too, unless `--include-excluded`.
    fn keeps(&self, category: &LoadedCategory, rule: &LoadedRule) -> bool {
        if !self.categories.is_empty()
            && !self.categories.iter().any(|c| c == category.id().as_str())
        {
            return false;
        }
        if self.kind.is_some_and(|k| k != rule.kind()) {
            return false;
        }
        let Some(genre) = &self.genre else {
            return true;
        };
        if !category.recommended_for().contains(genre) {
            return false;
        }
        let excluded = rule
            .rule()
            .applies_to
            .as_ref()
            .and_then(|a| a.genres.as_ref())
            .is_some_and(|genres| !genres.iter().any(|g| g == genre));
        self.include_excluded || !excluded
    }
}

fn selected<'a>(set: &'a RuleSet, filter: &Filter) -> Vec<(&'a LoadedCategory, &'a LoadedRule)> {
    set.categories()
        .iter()
        .flat_map(|c| c.rules().iter().map(move |r| (c, r)))
        .filter(|(c, r)| filter.keeps(c, r))
        .collect()
}

/// `slopvac rules`.
///
/// # Errors
/// A usage error for a bad `--genre`, or a load error.
pub fn rules(m: &ArgMatches, out: &mut dyn std::io::Write) -> Result<u8, CliError> {
    let filter = Filter::from_matches(m)?;
    let set = load(m)?;
    let picked = selected(&set, &filter);
    let text = match format_of(m) {
        "json" => rules_json(&set, &picked),
        "md" => rules_md(&picked),
        _ => rules_text(&picked),
    };
    emit(out, &text)
}

fn format_of(m: &ArgMatches) -> &str {
    m.get_one::<String>("format").map_or("text", String::as_str)
}

fn emit(out: &mut dyn std::io::Write, text: &str) -> Result<u8, CliError> {
    out.write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|err| {
            CliError::new(
                "output-failed",
                format!("cannot write output: {}", err.kind()),
            )
        })?;
    Ok(exit::PASS)
}

fn rules_text(picked: &[(&LoadedCategory, &LoadedRule)]) -> String {
    let rows: Vec<[String; 5]> = picked
        .iter()
        .map(|(_, r)| {
            [
                r.id().to_string(),
                kind_name(r.kind()).to_owned(),
                word(&r.rule().severity),
                word(&r.rule().scope),
                source_of(r),
            ]
        })
        .collect();
    let mut widths = [0usize; 4];
    for row in &rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let mut out = String::new();
    for row in &rows {
        let _ = writeln!(
            out,
            "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {}",
            row[0],
            row[1],
            row[2],
            row[3],
            row[4],
            w0 = widths[0],
            w1 = widths[1],
            w2 = widths[2],
            w3 = widths[3],
        );
    }
    let categories = picked
        .iter()
        .map(|(c, _)| c.id())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let _ = writeln!(
        out,
        "{} rule(s) in {} category(ies)",
        rows.len(),
        categories
    );
    out
}

fn rule_json(category: &LoadedCategory, rule: &LoadedRule) -> Value {
    let mut value = serde_json::to_value(rule.rule()).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut value {
        map.insert("id".into(), json!(rule.id().as_str()));
        map.insert("category".into(), json!(category.id().as_str()));
        map.insert("kind".into(), json!(kind_name(rule.kind())));
        if let Some(url) = url_of(rule) {
            map.insert("url".into(), json!(url));
        }
    }
    value
}

fn rules_json(set: &RuleSet, picked: &[(&LoadedCategory, &LoadedRule)]) -> String {
    let categories: Vec<Value> = set
        .categories()
        .iter()
        .map(|c| {
            json!({
                "id": c.id().as_str(),
                "title": c.title().as_str(),
                "description": c.description().as_str(),
                "weight": c.weight(),
                "recommended_for": c.recommended_for().iter().map(GenreId::as_str).collect::<Vec<_>>(),
            })
        })
        .collect();
    let rules: Vec<Value> = picked.iter().map(|(c, r)| rule_json(c, r)).collect();
    let payload = json!({"pack": set.pack().as_str(), "categories": categories, "rules": rules});
    pretty(&payload)
}

fn pretty(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(value).unwrap_or_default();
    text.push('\n');
    text
}

fn rules_md(picked: &[(&LoadedCategory, &LoadedRule)]) -> String {
    let mut out = format!(
        "# Slopvac rules\n\nThis list has {} {}, with the severities each rule ships with.\n",
        picked.len(),
        if picked.len() == 1 { "rule" } else { "rules" },
    );
    let mut body = Vec::new();
    let mut last: Option<&str> = None;
    for (category, rule) in picked {
        if last != Some(category.id().as_str()) {
            let genres: Vec<_> = category
                .recommended_for()
                .iter()
                .map(GenreId::as_str)
                .collect();
            let genres = if genres.is_empty() {
                "no genre".to_owned()
            } else {
                genres.join(", ")
            };
            body.push(format!(
                "## {} (`{}`), recommended for: {genres}",
                category.title(),
                category.id()
            ));
            body.push(String::new());
            last = Some(category.id().as_str());
        }
        body.extend(rule_block(rule));
        body.push(String::new());
    }
    if !body.is_empty() {
        body.pop();
        let _ = write!(out, "\n{DISABLE}\n{}\n{ENABLE}\n", body.join("\n"));
    }
    out
}

fn fenced(text: &str) -> Vec<String> {
    let longest = text
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat((longest + 1).max(3));
    let mut lines = vec![format!("  {fence}text")];
    lines.extend(text.trim_matches('\n').lines().map(|l| {
        if l.is_empty() {
            String::new()
        } else {
            format!("  {l}")
        }
    }));
    lines.push(format!("  {fence}"));
    lines
}

fn example_lines(number: usize, example: &Example) -> Vec<String> {
    let mut lines = vec![format!("- Example {number}, `bad`:"), String::new()];
    lines.extend(fenced(&example.bad));
    lines.push(String::new());
    match example.good.as_deref() {
        None => lines.push("  `good`: none.".into()),
        Some(g) if g.trim().is_empty() => {
            lines.push("  `good`: empty, so the fix deletes the text.".into());
        }
        Some(g) => {
            lines.extend(["  `good`:".to_owned(), String::new()]);
            lines.extend(fenced(g));
        }
    }
    if let Some(note) = &example.note {
        lines.extend([String::new(), format!("  Note: {}", clean(note))]);
    }
    lines
}

fn question_of(rule: &LoadedRule) -> Option<&str> {
    match &rule.rule().match_ {
        Match::Judgement(j) => Some(j.question.as_str()),
        _ => None,
    }
}

fn source_line(rule: &LoadedRule) -> String {
    let c = &rule.rule().citation;
    let mut parts = vec![clean(c.source.as_str())];
    if let Some(v) = &c.ste_ref {
        parts.push(format!("ste_ref `{v}`"));
    }
    if let Some(v) = &c.orwell_ref {
        parts.push(format!("orwell_ref `{v}`"));
    }
    if let Some(v) = &c.url {
        parts.push(format!("url <{v}>"));
    }
    parts.join("; ")
}

fn rule_block(rule: &LoadedRule) -> Vec<String> {
    let r = rule.rule();
    let mut lines = vec![
        format!("### `{}`: {}", rule.id(), clean(r.name.as_str())),
        String::new(),
        format!("- Category: `{}`", rule.category()),
        format!("- Scope: `{}`", word(&r.scope)),
        format!("- Kind: `{}`", kind_name(rule.kind())),
        format!("- Severity: `{}`", word(&r.severity)),
    ];
    match question_of(rule) {
        Some(q) => lines.push(format!("- Question: {}", clean(q))),
        None => lines.push(format!(
            "- Message: {}",
            clean(&message_text(r.message.as_str()))
        )),
    }
    if let Some(fix) = &r.fix {
        lines.push(format!("- Fix: {}", clean(fix)));
    }
    for (i, example) in r.examples().iter().enumerate() {
        lines.extend(example_lines(i + 1, example));
    }
    if let Some(exceptions) = &r.exceptions {
        let names: Vec<_> = exceptions.iter().map(|e| format!("`{}`", e.name)).collect();
        lines.push(format!("- Exceptions: {}", names.join(", ")));
    }
    lines.push(format!("- Source: {}", source_line(rule)));
    if let Some(note) = &r.citation.note {
        lines.push(format!("- Source note: {}", clean(note)));
    }
    if let Some(url) = url_of(rule) {
        lines.push(format!("- Page: <{url}>"));
    }
    lines
}

fn suppression(rule: &LoadedRule) -> Option<String> {
    let first = rule.rule().exceptions.as_ref()?.iter().next()?;
    Some(format!(
        "<!-- slopvac-allow: rule={} reason={} -->",
        rule.id(),
        first.name
    ))
}

/// `slopvac explain RULE`.
///
/// # Errors
/// `unknown-rule` when the ID matches no shipped rule; a bare 2.x ID gets a pointer to
/// `slopvac migrate`.
pub fn explain(m: &ArgMatches, out: &mut dyn std::io::Write) -> Result<u8, CliError> {
    let id = m.get_one::<String>("rule").expect("required");
    let set = load(m)?;
    let found = slopvac_model::RuleId::new(id.clone())
        .ok()
        .and_then(|rid| set.get(&rid));
    let Some(rule) = found else {
        let hint = if id.contains(':') {
            ""
        } else {
            " Rule IDs carry a pack prefix in 3.0, such as `core:ste-words.spelling`; `slopvac migrate` rewrites 2.x IDs."
        };
        return Err(CliError::new(
            "unknown-rule",
            format!("unknown rule: {id}.{hint}"),
        ));
    };
    let category = set
        .categories()
        .iter()
        .find(|c| c.id() == rule.category())
        .expect("a rule has a category");
    let text = match format_of(m) {
        "json" => {
            let mut value = rule_json(category, rule);
            if let (Value::Object(map), Some(s)) = (&mut value, suppression(rule)) {
                map.insert("suppression".into(), json!(s));
            }
            pretty(&value)
        }
        "md" => format!("{DISABLE}\n{}\n{ENABLE}\n", rule_block(rule).join("\n")),
        _ => explain_text(rule),
    };
    emit(out, &text)
}

fn explain_text(rule: &LoadedRule) -> String {
    let r = rule.rule();
    let mut out = String::new();
    let _ = writeln!(out, "{} -- {}", rule.id(), clean(r.name.as_str()));
    let _ = writeln!(
        out,
        "kind: {}   severity: {}   scope: {}",
        kind_name(rule.kind()),
        word(&r.severity),
        word(&r.scope)
    );
    let _ = writeln!(out, "\n{}", message_text(r.message.as_str()));
    if let Some(fix) = &r.fix
        && !message_text(r.message.as_str())
            .to_lowercase()
            .contains(&fix.trim_end_matches('.').to_lowercase())
    {
        let _ = writeln!(out, "\nFix: {fix}");
    }
    if let Some(q) = question_of(rule) {
        let _ = writeln!(out, "\nDecide by asking: {q}");
        let _ = writeln!(
            out,
            "Lint never reports this rule; `slopvac prime review` applies it."
        );
    }
    if let Some(exceptions) = &r.exceptions {
        let _ = writeln!(out, "\nNamed exceptions (a suppression must cite one):");
        for e in exceptions {
            let _ = writeln!(out, "  - {}", e.name);
        }
        if let Some(s) = suppression(rule) {
            let _ = writeln!(out, "\n  {s}");
        }
    }
    if !r.examples().is_empty() {
        let _ = writeln!(out, "\nExamples");
        for e in r.examples() {
            let _ = writeln!(out, "  - {}", e.bad);
            if let Some(good) = e.good.as_deref().filter(|g| !g.is_empty()) {
                let _ = writeln!(out, "  + {good}");
            }
            if let Some(note) = &e.note {
                let _ = writeln!(out, "    {note}");
            }
        }
    }
    let _ = writeln!(out, "\nSource: {}", source_line(rule));
    if let Some(note) = &r.citation.note {
        let _ = writeln!(out, "  {note}");
    }
    if let Some(url) = url_of(rule) {
        let _ = writeln!(out, "Page: {url}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use slopvac_model::PackId;
    use slopvac_rules::CompileOptions;
    use std::path::PathBuf;

    /// A category recommended for `consumer`, with one rule limited to `reference`.
    fn set() -> RuleSet {
        let rule = |id: &str, applies: &str| {
            format!(
                r#"{{"id":"{id}","name":"n","message":"m","severity":"warning","scope":"prose",
                "match":{{"kind":"tokens","tokens":["zzz"]}},{applies}
                "ai_signal":"none","ai_signal_source":"unmeasured","citation":{{"source":"s"}}}}"#
            )
        };
        let text = format!(
            r#"{{"schema_version":3,"id":"cat","title":"T","description":"D","weight":1.0,
            "recommended_for":["consumer"],"rules":[{},{}]}}"#,
            rule("open", ""),
            rule("narrow", r#""applies_to":{"genres":["reference"]},"#)
        );
        RuleSet::from_sources(
            &PackId::new("t").unwrap(),
            [(PathBuf::from("cat.json"), text)],
            &CompileOptions::default(),
        )
        .expect("loads")
    }

    fn count(include_excluded: bool) -> usize {
        let filter = Filter {
            categories: vec![],
            kind: None,
            genre: Some(GenreId::new("consumer").unwrap()),
            include_excluded,
        };
        selected(&set(), &filter).len()
    }

    #[test]
    fn a_rule_limited_to_other_genres_is_dropped_by_the_genre_filter() {
        assert_eq!(count(false), 1);
    }

    #[test]
    fn include_excluded_brings_back_rules_the_genre_excludes() {
        assert_eq!(count(true), 2);
    }
}
