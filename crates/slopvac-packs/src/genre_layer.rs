//! A genre as a layer of the config stack.

use serde_json::{Value, json};
use slopvac_config::{Layer, Setting, Source};
use slopvac_context::{Budget, CategorySettings, GenreSpec, RuleLevel};
use slopvac_model::PackId;

/// The file name shown in provenance for a pack file: `plugin:<id>/<path>`.
pub(crate) fn display_path(pack: &PackId, file: &str) -> String {
    format!("plugin:{pack}/{file}")
}

const fn level(l: RuleLevel) -> &'static str {
    match l {
        RuleLevel::Error => "error",
        RuleLevel::Warning => "warning",
        RuleLevel::Suggestion => "suggestion",
        RuleLevel::Off => "off",
    }
}

fn key(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_owned()).collect()
}

/// The settings a genre sets itself: its profile, category dials and rule levels. Rule names
/// in the pack's genre files are bare, so they take the pack id as a prefix.
pub(crate) fn settings(pack: &PackId, file: &str, spec: &GenreSpec) -> Vec<Setting> {
    let source = Source {
        layer: Layer::Genre,
        file: Some(display_path(pack, file)),
        line: None,
        column: None,
    };
    let mut out = Vec::new();
    let mut push = |path: Vec<String>, value: Value| {
        out.push(Setting {
            path,
            value,
            source: source.clone(),
        });
    };
    if let Some(profile) = spec.profile {
        push(key(&["profile", "name"]), json!(profile.as_str()));
    }
    for (category, dials) in &spec.categories {
        let at = |dial: &str| key(&["categories", category, dial]);
        match dials {
            CategorySettings::Level(l) => push(at("severity"), json!(level(*l))),
            CategorySettings::Table(t) => {
                if let Some(l) = t.severity {
                    push(at("severity"), json!(level(l)));
                }
                if let Some(l) = t.minimum_severity {
                    push(at("minimum_severity"), json!(level(l)));
                }
                match t.max_per_100_words {
                    Some(Budget::Limit(n)) => push(at("max_per_100_words"), json!(n)),
                    Some(Budget::None(_)) => push(at("max_per_100_words"), json!("none")),
                    None => {}
                }
                if let Some(w) = t.weight {
                    push(at("weight"), json!(w));
                }
            }
        }
    }
    for (rule, l) in &spec.rules {
        let name = if rule.contains(':') {
            rule.clone()
        } else {
            format!("{pack}:{rule}")
        };
        push(key(&["rules", &name]), json!(level(*l)));
    }
    out
}
