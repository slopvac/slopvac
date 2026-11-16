use std::collections::BTreeMap;

use serde_json::Value as Json;
use slopvac_config::Resolved;

use crate::{Style, StyleError, StyleSource};

/// A declared conflict between two active styles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// The earlier style, which loses.
    pub loser: String,
    /// The later style, which wins.
    pub winner: String,
    /// Keys where both set different values, as dotted paths such as `rules.core:list`.
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
#[derive(Debug, Clone, Default)]
pub struct Resolution {
    /// Styles in application order, `extends` first, each once.
    pub order: Vec<String>,
    /// The style layers merged by the config stack: the effective value of each key and the
    /// history of values, each with the preset and line that set it.
    pub resolved: Resolved,
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
    let loaded = apply_order(names, source)?;

    let mut res = Resolution {
        order: loaded.iter().map(|(n, _)| n.clone()).collect(),
        resolved: Resolved::from_layers(loaded.iter().map(|(_, s)| s.layer().to_vec())),
        conflicts: Vec::new(),
    };
    for (i, (later, style)) in loaded.iter().enumerate() {
        for (earlier, other) in &loaded[..i] {
            if style.conflicts.contains(earlier) || other.conflicts.contains(later) {
                res.conflicts.push(Conflict {
                    loser: earlier.clone(),
                    winner: later.clone(),
                    keys: differing_keys(other, style),
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

/// The styles that `names` activate, in application order: each style after its `extends`,
/// each once.
pub(crate) fn apply_order(
    names: &[String],
    source: &dyn StyleSource,
) -> Result<Vec<(String, Style)>, StyleError> {
    let mut loaded: Vec<(String, Style)> = Vec::new();
    for name in names {
        expand(name, source, &mut Vec::new(), &mut loaded)?;
        // A style listed explicitly takes the position of its listing even if
        // an earlier style already pulled it in as an ancestor.
        if let Some(pos) = loaded.iter().position(|(n, _)| n == name)
            && pos + 1 != loaded.len()
        {
            let entry = loaded.remove(pos);
            loaded.push(entry);
        }
    }
    Ok(loaded)
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

fn differing_keys(a: &Style, b: &Style) -> Vec<String> {
    let last = |s: &Style| -> BTreeMap<Vec<String>, Json> {
        s.layer()
            .iter()
            .map(|x| (x.path.clone(), x.value.clone()))
            .collect()
    };
    let (va, vb) = (last(a), last(b));
    va.iter()
        .filter(|(path, value)| vb.get(*path).is_some_and(|other| other != *value))
        .map(|(path, _)| path.join("."))
        .collect()
}
