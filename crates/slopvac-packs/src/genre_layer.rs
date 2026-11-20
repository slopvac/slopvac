//! A genre as a layer of the config stack.

use serde_json::{Value, json};
use slopvac_config::tree::{LineIndex, Node};
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

/// A genre file's parsed form, its syntax tree and its text, which the key locations come from.
#[derive(Debug, Clone)]
pub(crate) struct GenreFile {
    pub(crate) spec: GenreSpec,
    text: String,
    root: Node,
}

impl GenreFile {
    pub(crate) const fn new(spec: GenreSpec, text: String, root: Node) -> Self {
        Self { spec, text, root }
    }

    /// The line and column of the deepest key on `path` that the file has: a path the file
    /// lacks (a short form such as `prose: error`) resolves to its longest prefix.
    pub(crate) fn locate(&self, path: &[&str]) -> (Option<usize>, Option<usize>) {
        let mut node = &self.root;
        let mut found = None;
        for part in path {
            let Some(entry) = node.entry(part) else {
                break;
            };
            found = Some(entry.key_span.start);
            node = &entry.node;
        }
        found.map_or((None, None), |offset| {
            let (line, column) = LineIndex::new(&self.text).line_col(offset);
            (Some(line), Some(column))
        })
    }
}

/// The settings a genre sets itself: its profile, category dials and rule levels, each with the
/// line and column of its key. Rule names in the pack's genre files are bare, so they take the
/// pack id as a prefix.
pub(crate) fn settings(pack: &PackId, file: &str, genre: &GenreFile) -> Vec<Setting> {
    let spec = &genre.spec;
    let mut out = Vec::new();
    let mut push = |path: Vec<String>, value: Value, written: &[&str]| {
        let (line, column) = genre.locate(written);
        out.push(Setting {
            path,
            value,
            source: Source {
                layer: Layer::Genre,
                file: Some(display_path(pack, file)),
                line,
                column,
            },
        });
    };
    if let Some(profile) = spec.profile {
        push(
            key(&["profile", "name"]),
            json!(profile.as_str()),
            &["profile"],
        );
    }
    for (category, dials) in &spec.categories {
        let at = |dial: &str| key(&["categories", category, dial]);
        let written = |dial: &'static str| ["categories", category.as_str(), dial];
        match dials {
            CategorySettings::Level(l) => {
                push(at("severity"), json!(level(*l)), &written("severity"));
            }
            CategorySettings::Table(t) => {
                if let Some(l) = t.severity {
                    push(at("severity"), json!(level(l)), &written("severity"));
                }
                if let Some(l) = t.minimum_severity {
                    push(
                        at("minimum_severity"),
                        json!(level(l)),
                        &written("minimum_severity"),
                    );
                }
                match t.max_per_100_words {
                    Some(Budget::Limit(n)) => push(
                        at("max_per_100_words"),
                        json!(n),
                        &written("max_per_100_words"),
                    ),
                    Some(Budget::None(_)) => push(
                        at("max_per_100_words"),
                        json!("none"),
                        &written("max_per_100_words"),
                    ),
                    None => {}
                }
                if let Some(w) = t.weight {
                    push(at("weight"), json!(w), &written("weight"));
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
        push(
            key(&["rules", &name]),
            json!(level(*l)),
            &["rules", rule.as_str()],
        );
    }
    out
}
