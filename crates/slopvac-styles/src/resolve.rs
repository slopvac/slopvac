use std::collections::BTreeMap;

use crate::{CategoryDials, CategoryPatch, SettingsPatch, Style, StyleError, StyleSource};

/// One value a style set for a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setting {
    /// Style that set it.
    pub style: String,
    /// Rendered value.
    pub value: String,
}

/// A declared conflict between two active styles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// The earlier style, which loses.
    pub loser: String,
    /// The later style, which wins.
    pub winner: String,
    /// Keys where both set different values.
    pub keys: Vec<String>,
}

impl Conflict {
    /// A one-line warning that names the winner.
    #[must_use]
    pub fn message(&self) -> String {
        let on = if self.keys.is_empty() {
            String::new()
        } else {
            format!(" on {}", self.keys.join(", "))
        };
        format!(
            "style `{}` conflicts with `{}`; `{}` wins{on}",
            self.loser, self.winner, self.winner
        )
    }
}

/// The outcome of resolving a `styles` list.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resolution {
    /// Styles in application order, `extends` first, each once.
    pub order: Vec<String>,
    /// The merged settings.
    pub patch: SettingsPatch,
    /// Every value per key in application order; the last is effective.
    pub history: BTreeMap<String, Vec<Setting>>,
    /// Declared conflicts between active styles.
    pub conflicts: Vec<Conflict>,
}

/// Resolve `names` against `source`. With `strict`, a declared conflict is an
/// error instead of a warning.
///
/// # Errors
/// Missing or malformed styles, `extends` cycles, and in strict mode conflicts.
pub fn resolve(
    names: &[String],
    source: &dyn StyleSource,
    strict: bool,
) -> Result<Resolution, StyleError> {
    let mut loaded: Vec<(String, Style)> = Vec::new();
    for name in names {
        expand(name, source, &mut Vec::new(), &mut loaded)?;
    }

    let mut res = Resolution::default();
    for (name, style) in &loaded {
        res.order.push(name.clone());
        for (key, value) in style.settings.entries() {
            res.history.entry(key).or_default().push(Setting {
                style: name.clone(),
                value,
            });
        }
        merge(&mut res.patch, &style.settings);
    }

    for (i, (later, style)) in loaded.iter().enumerate() {
        for (earlier, other) in &loaded[..i] {
            if style.conflicts.contains(earlier) || other.conflicts.contains(later) {
                res.conflicts.push(Conflict {
                    loser: earlier.clone(),
                    winner: later.clone(),
                    keys: differing_keys(&res.history, earlier, later),
                });
            }
        }
    }
    if strict && let Some(c) = res.conflicts.first() {
        return Err(StyleError::Conflict {
            loser: c.loser.clone(),
            winner: c.winner.clone(),
        });
    }
    Ok(res)
}

fn expand(
    name: &str,
    source: &dyn StyleSource,
    stack: &mut Vec<String>,
    out: &mut Vec<(String, Style)>,
) -> Result<(), StyleError> {
    if stack.iter().any(|s| s == name) {
        let mut cycle = stack.clone();
        cycle.push(name.to_owned());
        return Err(StyleError::Cycle(cycle));
    }
    if out.iter().any(|(n, _)| n == name) {
        return Ok(());
    }
    let style = source.load(name)?;
    stack.push(name.to_owned());
    for parent in &style.extends {
        expand(parent, source, stack, out)?;
    }
    stack.pop();
    out.push((name.to_owned(), style));
    Ok(())
}

fn merge(into: &mut SettingsPatch, from: &SettingsPatch) {
    into.rules
        .extend(from.rules.iter().map(|(k, v)| (k.clone(), *v)));
    for (k, c) in &from.categories {
        let new = c.dials();
        let cur = into
            .categories
            .entry(k.clone())
            .or_insert_with(|| CategoryPatch::Dials(CategoryDials::default()));
        let mut d = cur.dials();
        d.severity = new.severity.or(d.severity);
        d.max_per_100_words = new.max_per_100_words.or(d.max_per_100_words);
        d.weight = new.weight.or(d.weight);
        *cur = CategoryPatch::Dials(d);
    }
    if from.terms.allow.is_some() {
        into.terms.allow.clone_from(&from.terms.allow);
    }
    if from.terms.blocklists.is_some() {
        into.terms.blocklists.clone_from(&from.terms.blocklists);
    }
}

fn differing_keys(history: &BTreeMap<String, Vec<Setting>>, a: &str, b: &str) -> Vec<String> {
    history
        .iter()
        .filter(|(_, settings)| {
            let va = settings.iter().rev().find(|s| s.style == a);
            let vb = settings.iter().rev().find(|s| s.style == b);
            matches!((va, vb), (Some(x), Some(y)) if x.value != y.value)
        })
        .map(|(k, _)| k.clone())
        .collect()
}
