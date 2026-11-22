//! Contract test M1 for `--preserve-behavior`: the migrated config, loaded and
//! resolved by `slopvac-config`, gives every rule the level the 2.x engine gave it.
//!
//! The 2.x levels are measured. `tests/fixtures/generate.py` runs the 2.x engine on
//! each scenario's config and files and writes the result to `measured_2x` in
//! `tests/fixtures/preserve/vectors.json`; `tests/contract/test_migrate_fixtures.py`
//! fails when that file is stale.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use slopvac_config::{ConfigLayer, Inputs, PackFile, Presets, Setting};
use slopvac_migrate::{Options, RenameTable, migrate_config};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_toml(rel: &str) -> toml::Table {
    fs::read_to_string(crate_dir().join(rel))
        .unwrap()
        .parse()
        .unwrap()
}

fn to_json(v: &toml::Value) -> Value {
    serde_json::to_value(v).unwrap()
}

/// The `core:legacy-<profile>` styles, as the core pack ships them: the bundled
/// legacy data with each 2.x name qualified through the rename table.
struct LegacyStyles(HashMap<String, Vec<Setting>>);

impl LegacyStyles {
    fn bundled(renames: &RenameTable) -> Self {
        let mut styles = HashMap::new();
        for profile in ["strict", "normal", "relaxed"] {
            let style = read_toml(&format!("data/legacy/legacy-{profile}.toml"));
            let mut settings = Vec::new();
            for (name, dials) in style["categories"].as_table().unwrap() {
                let cat = renames.category(name).unwrap();
                for (dial, v) in dials.as_table().unwrap() {
                    settings.push(Setting::bare(
                        ConfigLayer::Style,
                        &["categories", &cat, dial],
                        to_json(v),
                    ));
                }
            }
            for (qid, level) in style["rules"].as_table().unwrap() {
                if let Some(id) = renames.rule(qid) {
                    settings.push(Setting::bare(
                        ConfigLayer::Style,
                        &["rules", &id],
                        to_json(level),
                    ));
                }
            }
            styles.insert(format!("core:legacy-{profile}"), settings);
        }
        Self(styles)
    }
}

impl Presets for LegacyStyles {
    fn style(&self, name: &str) -> Option<Vec<Setting>> {
        self.0.get(name).cloned()
    }

    fn genre_chain(&self, _name: &str) -> Vec<Vec<Setting>> {
        Vec::new()
    }

    fn pack_config(&self, _name: &str) -> Option<PackFile> {
        None
    }
}

/// A measured map for one file: the base of its profile with the file's differences.
fn overlay(base: &Value, diff: &Value) -> BTreeMap<String, Value> {
    let mut out: BTreeMap<String, Value> = base
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for (k, v) in diff.as_object().unwrap() {
        out.insert(k.clone(), v.clone());
    }
    out
}

/// Scenarios whose migrated config 3.0 refuses for a repeated scope: a guard that must
/// follow a later block and so cannot merge into the block with its files
/// (docs/contract/profiles.md, E4, known limit). `migrate` stops on them before it
/// writes. Their 2.x levels stay measured for the change that lifts the limit.
const KNOWN_LIMIT: &[&str] = &["guard-held-back-by-a-later-category-level"];

#[test]
fn every_migrated_config_resolves_to_the_measured_2x_levels() {
    let vectors: Value = serde_json::from_str(
        &fs::read_to_string(crate_dir().join("tests/fixtures/preserve/vectors.json")).unwrap(),
    )
    .unwrap();
    let measured = &vectors["measured_2x"];
    let renames = RenameTable::bundled();
    let presets = LegacyStyles::bundled(&renames);
    let catalogue = read_toml("data/legacy/catalogue.toml");
    let judgement = |qid: &str| catalogue["rules"][qid]["kind"].as_str() == Some("judgement");
    let inputs = Inputs {
        presets: &presets,
        front_matter: &[],
        directives: &[],
        cli: &[],
        env: &[],
    };

    let scenarios = measured["files"].as_object().unwrap();
    assert!(scenarios.len() >= 29, "{}", scenarios.len());
    let mut wrong: Vec<String> = Vec::new();
    for (name, files) in scenarios {
        let input = vectors["expansions"][name]["input"].as_str().unwrap();
        let text = migrate_config(
            input,
            "slopvac.toml",
            &renames,
            Options {
                preserve_behavior: true,
            },
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"))
        .text;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("slopvac.toml");
        fs::write(&path, &text).unwrap();
        if KNOWN_LIMIT.contains(&name.as_str()) {
            match slopvac_config::load(&path, &presets) {
                Err(e) if e.code() == slopvac_config::Code::DuplicateScope => {}
                Err(e) => wrong.push(format!("{name}: refused with {}", e.code().as_str())),
                Ok(_) => wrong.push(format!("{name}: loads; remove it from KNOWN_LIMIT")),
            }
            continue;
        }
        let config =
            slopvac_config::load(&path, &presets).unwrap_or_else(|e| panic!("{name}: {e}"));

        for (file, want) in files.as_object().unwrap() {
            let profile = want["profile"].as_str().unwrap();
            let base = &measured["base"][profile];
            let resolved = config.resolve(Path::new(file), &inputs);
            for (qid, level) in overlay(&base["levels"], &want["levels"]) {
                let Some(id) = renames.rule(&qid) else {
                    continue;
                };
                let category = qid.split_once('.').unwrap().0;
                let category = (!judgement(&qid)).then(|| renames.category(category).unwrap());
                let got = resolved
                    .level(&id, category.as_deref())
                    .map(|l| l.level.as_str());
                if got != level.as_str() {
                    wrong.push(format!("{name} {file} {qid}: {got:?}, 2.x {level}"));
                }
            }
            for (cat, dials) in overlay(&base["dials"], &want["dials"]) {
                if dials == json!("off") {
                    continue;
                }
                // A category that 3.0 removed has nothing to resolve.
                let Some(id) = renames.category(&cat) else {
                    continue;
                };
                let budget = resolved.budget(&id);
                // With no weight set, 3.0 scores at the category's own weight.
                let weight = resolved
                    .get(&["categories", &id, "weight"])
                    .and_then(Value::as_f64)
                    .or_else(|| catalogue["weights"][&cat].as_float());
                if budget != dials["budget"].as_f64() || weight != dials["weight"].as_f64() {
                    wrong.push(format!(
                        "{name} {file} {cat}: budget {budget:?} weight {weight:?}, 2.x {dials}"
                    ));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} differences:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}
