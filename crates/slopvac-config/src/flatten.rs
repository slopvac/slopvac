//! Turn a validated tree into the leaf settings a layer contributes.

use slopvac_model::ConfigLayer;

use crate::source::{Setting, located_source};
use crate::tree::{LineIndex, Node, Value};

/// Where the settings of one document live.
#[derive(Debug, Clone, Copy)]
pub struct Origin<'a> {
    /// The layer the settings belong to.
    pub layer: ConfigLayer,
    /// Path shown in provenance, relative to the project root.
    pub file: &'a str,
    /// The full text the tree's spans index into.
    pub text: &'a str,
}

/// Flatten the table `node`, leaving out the top-level keys named in `skip`.
///
/// Scalars and lists are leaves; a list replaces the lower list as a whole. A bare
/// `profile` and a bare category level expand to the table form, so both forms merge
/// field by field.
#[must_use]
pub fn flatten(node: &Node, origin: Origin<'_>, skip: &[&str]) -> Vec<Setting> {
    flatten_with(node, origin, skip, &LineIndex::new(origin.text))
}

/// [`flatten`] with a line index of `origin.text` that the caller built, so many trees of one
/// document share one index.
#[must_use]
pub fn flatten_with(
    node: &Node,
    origin: Origin<'_>,
    skip: &[&str],
    lines: &LineIndex<'_>,
) -> Vec<Setting> {
    let mut out = Vec::new();
    walk(node, &mut Vec::new(), skip, origin, lines, &mut out);
    out
}

fn walk(
    node: &Node,
    path: &mut Vec<String>,
    skip: &[&str],
    origin: Origin<'_>,
    lines: &LineIndex<'_>,
    out: &mut Vec<Setting>,
) {
    let Value::Table(entries) = &node.value else {
        return;
    };
    let data_keys = matches!(path.as_slice(), [only] if only == "rules" || only == "categories");
    for e in entries {
        if path.is_empty() && skip.contains(&e.key.as_str()) {
            continue;
        }
        if !data_keys && e.key.starts_with("x-") {
            continue;
        }
        path.push(e.key.clone());
        let (line, column) = lines.line_col(e.key_span.start);
        let source = located_source(origin.layer, Some(origin.file.to_owned()), line, column);
        let bare_profile = path.as_slice() == ["profile"];
        let bare_category = matches!(path.as_slice(), [c, _] if c == "categories");
        match &e.node.value {
            Value::Table(_) => walk(&e.node, path, skip, origin, lines, out),
            _ if bare_profile || bare_category => {
                let mut full = path.clone();
                full.push(if bare_profile { "name" } else { "severity" }.to_owned());
                out.push(Setting {
                    path: full,
                    value: e.node.to_json(),
                    source,
                });
            }
            _ => out.push(Setting {
                path: path.clone(),
                value: e.node.to_json(),
                source,
            }),
        }
        path.pop();
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use super::*;
    use crate::tree::parse_toml;
    use serde_json::json;

    fn settings(text: &str) -> Vec<Setting> {
        let root = parse_toml(text).unwrap();
        flatten(
            &root,
            Origin {
                layer: ConfigLayer::Project,
                file: "slopvac.toml",
                text,
            },
            &["schema_version", "overrides", "genres", "extends"],
        )
    }

    fn find<'a>(s: &'a [Setting], path: &[&str]) -> &'a Setting {
        s.iter()
            .find(|x| x.path == path)
            .unwrap_or_else(|| panic!("no setting at {path:?}"))
    }

    #[test]
    fn bare_forms_expand_to_the_table_form() {
        let s = settings(
            "profile = \"strict\"\n[categories]\nprose = \"warning\"\n[categories.ste]\nweight = 0.5\n",
        );
        assert_eq!(find(&s, &["profile", "name"]).value, json!("strict"));
        assert_eq!(
            find(&s, &["categories", "prose", "severity"]).value,
            json!("warning")
        );
        assert_eq!(find(&s, &["categories", "ste", "weight"]).value, json!(0.5));
    }

    #[test]
    fn lists_are_leaves_and_skipped_keys_vanish() {
        let s = settings("schema_version = 3\nextends = [\"a.toml\"]\nexclude = [\"a\", \"b\"]\n");
        assert_eq!(s.len(), 1);
        assert_eq!(find(&s, &["exclude"]).value, json!(["a", "b"]));
    }

    #[test]
    fn x_keys_are_ignored_except_as_rule_names() {
        let s =
            settings("x-owner = \"me\"\n[rules]\n\"x-rule\" = \"off\"\n[thresholds]\nx-note = 1\n");
        assert_eq!(s.len(), 1);
        assert_eq!(find(&s, &["rules", "x-rule"]).value, json!("off"));
    }

    #[test]
    fn source_points_at_the_key() {
        let s = settings("locale = \"en-GB\"\n\n[thresholds]\nmax_errors = 2\n");
        let src = &find(&s, &["thresholds", "max_errors"]).source;
        assert_eq!(
            (src.line, src.column),
            (NonZeroUsize::new(4), NonZeroUsize::new(1))
        );
        assert_eq!(src.file.as_deref(), Some("slopvac.toml"));
    }
}
