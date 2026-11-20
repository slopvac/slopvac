//! The layer stack reads styles, genres and `<pack>:<name>` extends targets from real packs.
//!
//! The pack loader lives above `slopvac-config`, so these tests sit in the first crate that
//! wires the two together.

use std::fs;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use serde_json::json;
use slopvac_config::{ConfigLayer, ConfigSource, Inputs, LoadedConfig, Resolved, load};
use slopvac_packs::{DirSource, LoadedPack, MANIFEST_FILE, Packs, Source, core_source};
use tempfile::TempDir;

const FUTURE_TENSE: &str = "core:prose-craft.future-tense";

struct Project {
    dir: TempDir,
}

impl Project {
    fn new(config: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("slopvac.toml"), config).unwrap();
        Self { dir }
    }

    fn root(&self) -> PathBuf {
        fs::canonicalize(self.dir.path()).unwrap()
    }

    fn load(&self, packs: &Packs) -> LoadedConfig {
        load(&self.root().join("slopvac.toml"), packs).unwrap()
    }
}

fn resolve(config: &LoadedConfig, packs: &Packs) -> Resolved {
    config.resolve(
        Path::new("a.md"),
        &Inputs {
            presets: packs,
            front_matter: &[],
            directives: &[],
            cli: &[],
            env: &[],
        },
    )
}

fn at(layer: ConfigLayer, file: &str, line: usize, column: usize) -> ConfigSource {
    ConfigSource {
        layer,
        file: Some(file.into()),
        line: NonZeroUsize::new(line),
        column: NonZeroUsize::new(column),
    }
}

fn core() -> Packs {
    Packs::core().unwrap()
}

#[test]
fn a_project_rule_beats_the_core_genre_which_beats_the_core_style() {
    let project = Project::new(&format!(
        "schema_version = 3\nstyles = [\"ste\"]\n\n[rules]\n\"{FUTURE_TENSE}\" = \"warning\"\n\n[[genres]]\ngenre = \"tutorial\"\nfiles = [\"**\"]\n"
    ));
    let packs = core();
    let resolved = resolve(&project.load(&packs), &packs);

    let layers: Vec<ConfigLayer> = resolved
        .history(&["rules", FUTURE_TENSE])
        .iter()
        .map(|c| c.source.layer)
        .collect();
    assert_eq!(
        layers,
        [ConfigLayer::Genre, ConfigLayer::Project],
        "the tutorial genre sets the rule off, the project file overrides it"
    );
    assert_eq!(resolved.get(&["rules", FUTURE_TENSE]), Some(&json!("warning")));

    let style_rule = resolved.source(&["rules", "ste-nouns.multiword-noun-too-long"]);
    assert_eq!(style_rule.map(|s| s.layer), Some(ConfigLayer::Style));
}

#[test]
fn pack_supplied_values_keep_their_file_line_and_column() {
    let project = Project::new(
        "schema_version = 3\nstyles = [\"ste\"]\n\n[[genres]]\ngenre = \"tutorial\"\nfiles = [\"**\"]\n",
    );
    let packs = core();
    let resolved = resolve(&project.load(&packs), &packs);

    assert_eq!(
        resolved.source(&["rules", FUTURE_TENSE]),
        Some(&at(
            ConfigLayer::Genre,
            "plugin:core/genres/tutorial.yaml",
            15,
            3
        ))
    );
    assert_eq!(
        resolved.source(&["rules", "ste-nouns.multiword-noun-too-long"]),
        Some(&at(ConfigLayer::Style, "preset:ste", 20, 1))
    );
}

#[test]
fn a_pack_config_resolves_as_an_extends_target() {
    let pack_dir = tempfile::tempdir().unwrap();
    fs::write(
        pack_dir.path().join(MANIFEST_FILE),
        "schema_version = 3\nid = \"acme\"\nversion = \"1.0.0\"\nslopvac_api = 1\nkind = \"data\"\n\n[provides]\nconfigs = [\"configs/base.toml\"]\n",
    )
    .unwrap();
    fs::create_dir(pack_dir.path().join("configs")).unwrap();
    fs::write(
        pack_dir.path().join("configs/base.toml"),
        "schema_version = 3\n[thresholds]\nmax_errors = 7\n",
    )
    .unwrap();
    let acme = LoadedPack::load(&DirSource::new(pack_dir.path()), Source::Pack).unwrap();
    let core = LoadedPack::load(&core_source(), Source::Core).unwrap();
    let packs = Packs::new(vec![core, acme]).unwrap();

    let project = Project::new("schema_version = 3\nextends = [\"acme:base\"]\n");
    let resolved = resolve(&project.load(&packs), &packs);

    assert_eq!(resolved.get(&["thresholds", "max_errors"]), Some(&json!(7)));
    assert_eq!(
        resolved.source(&["thresholds", "max_errors"]),
        Some(&at(
            ConfigLayer::Extends,
            "plugin:acme/configs/base.toml",
            3,
            1
        ))
    );
}
