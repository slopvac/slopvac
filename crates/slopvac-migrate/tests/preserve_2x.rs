//! Contract test M1 for `--preserve-behavior`: the migrated config, loaded and
//! resolved by `slopvac-config`, gives every rule the level the 2.x engine gave it.
//!
//! The 2.x levels are measured. `tests/fixtures/generate.py` runs the 2.x engine on
//! each scenario's config and files and writes the result to `measured_2x` in
//! `tests/fixtures/preserve/vectors.json`; `tests/contract/test_migrate_fixtures.py`
//! fails when that file is stale. A scenario that `tests/fixtures/preserve/refused.json`
//! lists must stop with `migrate-output-invalid` instead.
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use slopvac_config::{ConfigLayer, Inputs, PackFile, Presets, Setting};
use slopvac_migrate::{Options, RenameTable, migrate_config};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_toml(rel: &str) -> Result<toml::Table> {
    Ok(fs::read_to_string(crate_dir().join(rel))?.parse()?)
}

fn to_json(v: &toml::Value) -> Result<Value> {
    Ok(serde_json::to_value(v)?)
}

/// The table at `key` of `t`, or an error that names `what`.
fn table<'a>(t: &'a toml::Table, key: &str, what: &str) -> Result<&'a toml::Table> {
    t.get(key)
        .and_then(toml::Value::as_table)
        .ok_or_else(|| format!("{what} has no `{key}` table").into())
}

/// The `core:legacy-<profile>` styles, as the core pack ships them: the bundled
/// legacy data with each 2.x name qualified through the rename table.
struct LegacyStyles(HashMap<String, Vec<Setting>>);

impl LegacyStyles {
    fn bundled(renames: &RenameTable) -> Result<Self> {
        let mut styles = HashMap::new();
        for profile in ["strict", "normal", "relaxed"] {
            let file = format!("data/legacy/legacy-{profile}.toml");
            let style = read_toml(&file)?;
            let mut settings = Vec::new();
            for (name, dials) in table(&style, "categories", &file)? {
                let cat = renames
                    .category(name)
                    .ok_or_else(|| format!("{file}: no rename for category {name}"))?;
                let dials = dials
                    .as_table()
                    .ok_or_else(|| format!("{file}: category {name} is not a table"))?;
                for (dial, v) in dials {
                    settings.push(Setting::bare(
                        ConfigLayer::Style,
                        &["categories", &cat, dial],
                        to_json(v)?,
                    ));
                }
            }
            for (qid, level) in table(&style, "rules", &file)? {
                if let Some(id) = renames.rule(qid) {
                    settings.push(Setting::bare(
                        ConfigLayer::Style,
                        &["rules", &id],
                        to_json(level)?,
                    ));
                }
            }
            styles.insert(format!("core:legacy-{profile}"), settings);
        }
        Ok(Self(styles))
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
fn overlay(base: &Value, diff: &Value) -> Result<BTreeMap<String, Value>> {
    let object = |v: &Value| {
        v.as_object()
            .cloned()
            .ok_or_else(|| format!("measured map is not an object: {v}"))
    };
    let mut out: BTreeMap<String, Value> = object(base)?.into_iter().collect();
    out.extend(object(diff)?);
    Ok(out)
}

/// The scenarios that `--preserve-behavior` refuses, from `tests/fixtures/preserve/refused.json`.
fn refused() -> Result<Vec<String>> {
    let text = fs::read_to_string(crate_dir().join("tests/fixtures/preserve/refused.json"))?;
    let data: Value = serde_json::from_str(&text)?;
    data["refused"]
        .as_array()
        .ok_or("refused.json has no `refused` list")?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("refused.json lists a non-string: {v}").into())
        })
        .collect()
}

#[test]
fn every_migrated_config_resolves_to_the_measured_2x_levels() -> Result<()> {
    let vectors: Value = serde_json::from_str(&fs::read_to_string(
        crate_dir().join("tests/fixtures/preserve/vectors.json"),
    )?)?;
    let measured = &vectors["measured_2x"];
    let renames = RenameTable::bundled();
    let presets = LegacyStyles::bundled(&renames)?;
    let catalogue = read_toml("data/legacy/catalogue.toml")?;
    let judgement = |qid: &str| catalogue["rules"][qid]["kind"].as_str() == Some("judgement");
    let inputs = Inputs {
        presets: &presets,
        front_matter: &[],
        directives: &[],
        cli: &[],
        env: &[],
    };

    let refused = refused()?;
    let scenarios = measured["files"]
        .as_object()
        .ok_or("vectors.json has no `measured_2x.files` map")?;
    for name in &refused {
        assert!(scenarios.contains_key(name), "refused.json names {name}");
    }
    assert!(scenarios.len() >= 29, "{}", scenarios.len());
    let mut wrong: Vec<String> = Vec::new();
    for (name, files) in scenarios {
        let input = vectors["expansions"][name]["input"]
            .as_str()
            .ok_or_else(|| format!("{name}: vectors.json has no input"))?;
        let migrated = migrate_config(
            input,
            "slopvac.toml",
            &renames,
            Options {
                preserve_behavior: true,
            },
        );
        if refused.contains(name) {
            match migrated {
                Err(e) if e.code() == "migrate-output-invalid" => {}
                Err(e) => wrong.push(format!("{name}: refused with {}", e.code())),
                Ok(_) => wrong.push(format!("{name}: not refused")),
            }
            continue;
        }
        let text = migrated.map_err(|e| format!("{name}: {e}"))?.text;
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("slopvac.toml");
        fs::write(&path, &text)?;
        let config = slopvac_config::load(&path, &presets).map_err(|e| format!("{name}: {e}"))?;

        let files = files
            .as_object()
            .ok_or_else(|| format!("{name}: the measured files are not a map"))?;
        for (file, want) in files {
            let profile = want["profile"]
                .as_str()
                .ok_or_else(|| format!("{name} {file}: no measured profile"))?;
            let base = &measured["base"][profile];
            let resolved = config.resolve(Path::new(file), &inputs);
            for (qid, level) in overlay(&base["levels"], &want["levels"])? {
                let Some(id) = renames.rule(&qid) else {
                    continue;
                };
                let category = match qid.split_once('.') {
                    Some((category, _)) if !judgement(&qid) => Some(
                        renames
                            .category(category)
                            .ok_or_else(|| format!("{qid}: no rename for its category"))?,
                    ),
                    Some(_) => None,
                    None => return Err(format!("{qid} is not a qualified rule id").into()),
                };
                let got = resolved
                    .level(&id, category.as_deref())
                    .map(|l| l.level.as_str());
                if got != level.as_str() {
                    wrong.push(format!("{name} {file} {qid}: {got:?}, 2.x {level}"));
                }
            }
            for (cat, dials) in overlay(&base["dials"], &want["dials"])? {
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
    Ok(())
}
