//! Rule ids that a config layer names and no loaded rule has.
//!
//! An unknown rule id is a warning, never an error: the run goes on without that entry.
//! The same check serves every layer that carries a `rules` map, so a config file, an
//! `extends` target, an `[[overrides]]` or `[[genres]]` block and a preset all report the id the
//! same way. The caller knows the rules, so it passes the test for a known id.

use std::path::Path;

use slopvac_messages::{Catalog, render};
use slopvac_model::ConfigSource;

use crate::messages::ConfigText;
use crate::source::{Setting, layer_name};

/// A rule id, as a layer wrote it, that no loaded rule has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownRule {
    rule: String,
    source: ConfigSource,
}

impl UnknownRule {
    /// The id as the layer wrote it, qualified or bare.
    #[must_use]
    pub fn rule(&self) -> &str {
        &self.rule
    }

    /// The layer, file, line and column of the key.
    #[must_use]
    pub fn source(&self) -> &ConfigSource {
        &self.source
    }

    /// The config file of the key as a report may show it: relative to the project root, or
    /// `<external>/<file name>` for a file outside it (policies.md section 1.7). The source
    /// keeps the fuller relative form that provenance needs, which can climb out of the root
    /// with `..`.
    #[must_use]
    pub fn shown_file(&self) -> Option<String> {
        self.source
            .file
            .as_deref()
            .map(|file| slopvac_fs::display_under_root(Path::new(file), Path::new(".")))
    }

    /// The stable notice code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.text().code()
    }

    /// The notice message. It names the file as [`UnknownRule::shown_file`] does, and the line
    /// and column when the layer has them.
    #[must_use]
    pub fn message(&self) -> String {
        render(&self.text())
    }

    fn text(&self) -> ConfigText {
        ConfigText::UnknownRule {
            place: self.place(),
            rule: self.rule.clone(),
        }
    }

    /// The layer, then the shown file with its line and column: `project slopvac.toml:4:1`.
    fn place(&self) -> String {
        let mut place = layer_name(self.source.layer).to_owned();
        if let Some(file) = self.shown_file() {
            place.push(' ');
            place.push_str(&file);
            if let (Some(line), Some(column)) = (self.source.line, self.source.column) {
                place = format!("{place}:{line}:{column}");
            }
        }
        place
    }
}

/// The `rules` keys in `settings` for which `is_known` is false, in the order the settings
/// list them.
pub fn unknown_rules(settings: &[Setting], is_known: impl Fn(&str) -> bool) -> Vec<UnknownRule> {
    settings
        .iter()
        .filter_map(|setting| match setting.path.as_slice() {
            [head, key, ..] if head == "rules" && !is_known(key) => Some(UnknownRule {
                rule: key.clone(),
                source: setting.source.clone(),
            }),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use serde_json::json;
    use std::num::NonZeroUsize;

    use slopvac_model::ConfigLayer;

    fn setting(path: &[&str], layer: ConfigLayer, file: Option<&str>) -> Setting {
        let mut setting = Setting::bare(layer, path, json!("off"));
        setting.source.file = file.map(str::to_owned);
        setting.source.line = file.and_then(|_| NonZeroUsize::new(3));
        setting.source.column = file.and_then(|_| NonZeroUsize::new(1));
        setting
    }

    #[rstest]
    #[case::config_file(ConfigLayer::Project, Some("slopvac.toml"), "project slopvac.toml:3:1")]
    #[case::extended_file(ConfigLayer::Extends, Some("base.toml"), "extends base.toml:3:1")]
    #[case::preset(ConfigLayer::Style, None, "style")]
    #[case::extended_file_outside_the_root(
        ConfigLayer::Extends,
        Some("../private/base.toml"),
        "extends <external>/base.toml:3:1"
    )]
    #[case::extended_file_below_a_parent(
        ConfigLayer::Extends,
        Some("../../private/deeper/base.toml"),
        "extends <external>/base.toml:3:1"
    )]
    fn an_unknown_rule_names_its_layer_and_location(
        #[case] layer: ConfigLayer,
        #[case] file: Option<&str>,
        #[case] place: &str,
    ) {
        let settings = [setting(&["rules", "a.b"], layer, file)];
        let found = unknown_rules(&settings, |_| false);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].rule(), "a.b");
        assert_eq!(found[0].code(), "config-unknown-rule");
        assert_eq!(
            found[0].message(),
            format!("{place}: unknown rule a.b; use a rule id from `slopvac rules`")
        );
    }

    #[rstest]
    #[case::inside_the_root("docs/base.toml", "docs/base.toml")]
    #[case::outside_the_root("../private/base.toml", "<external>/base.toml")]
    #[case::already_redacted("<external>/base.toml", "<external>/base.toml")]
    fn the_shown_file_never_climbs_out_of_the_root(#[case] file: &str, #[case] shown: &str) {
        let settings = [setting(&["rules", "a.b"], ConfigLayer::Extends, Some(file))];
        let found = unknown_rules(&settings, |_| false);
        assert_eq!(found[0].shown_file().as_deref(), Some(shown));
    }

    #[test]
    fn only_rules_keys_that_the_caller_does_not_know_are_reported() {
        let settings = [
            setting(&["rules", "known.rule"], ConfigLayer::Project, None),
            setting(&["rules", "gone.rule"], ConfigLayer::Project, None),
            setting(&["categories", "gone.rule"], ConfigLayer::Project, None),
            setting(&["thresholds", "min_score"], ConfigLayer::Project, None),
        ];
        let found = unknown_rules(&settings, |key| key == "known.rule");
        let ids: Vec<_> = found.iter().map(UnknownRule::rule).collect();
        assert_eq!(ids, ["gone.rule"]);
    }
}
