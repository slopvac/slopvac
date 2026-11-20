//! Layer order, merge rules, provenance and discovery, through the public interface.

mod common;

use std::fs;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use serde_json::{Value as Json, json};
use slopvac_config::{
    BudgetData, Code, ConfigBudget, ConfigError, ConfigLayer, ConfigSource, Discovery, Inputs, LoadedConfig,
    NoPresets, PackFile, Presets, Setting, bare_source, load,
};
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn root(&self) -> PathBuf {
        fs::canonicalize(self.dir.path()).unwrap()
    }

    fn write(&self, rel: &str, text: &str) -> PathBuf {
        let path = self.root().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }
}

fn inputs(presets: &dyn Presets) -> Inputs<'_> {
    Inputs {
        presets,
        front_matter: &[],
        directives: &[],
        cli: &[],
        env: &[],
    }
}

fn load_at(fx: &Fixture, rel: &str) -> LoadedConfig {
    load(&fx.root().join(rel), &NoPresets).unwrap()
}

fn rejected(err: ConfigError) -> slopvac_config::Rejected {
    match err {
        ConfigError::Rejected(r) => *r,
        other => panic!("expected a rejection, got {other}"),
    }
}

fn src(layer: ConfigLayer, file: &str, line: usize, column: usize) -> ConfigSource {
    ConfigSource {
        layer,
        file: Some(file.into()),
        line: NonZeroUsize::new(line),
        column: NonZeroUsize::new(column),
    }
}

#[test]
fn each_layer_wins_over_the_one_below_and_names_its_origin() {
    let fx = Fixture::new();
    fx.write(
        "slopvac.toml",
        "schema_version = 3\nlocale = \"en-GB\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\nlocale = \"en-US\"\n",
    );
    let cfg = load_at(&fx, "slopvac.toml");
    let front = [Setting {
        path: vec!["locale".into()],
        value: json!("und"),
        source: src(ConfigLayer::FrontMatter, "docs/a.md", 3, 3),
    }];
    let cli = [Setting::bare(ConfigLayer::Cli, &["locale"], json!("en-GB"))];

    let at = |rel: &str, i: &Inputs<'_>| cfg.resolve(Path::new(rel), i);

    let plain = at("README.md", &inputs(&NoPresets));
    assert_eq!(plain.get(&["locale"]), Some(&json!("en-GB")));
    assert_eq!(
        plain.source(&["locale"]),
        Some(&src(ConfigLayer::Project, "slopvac.toml", 2, 1))
    );

    let scoped = at("docs/a.md", &inputs(&NoPresets));
    assert_eq!(scoped.get(&["locale"]), Some(&json!("en-US")));
    assert_eq!(
        scoped.source(&["locale"]),
        Some(&src(ConfigLayer::Override, "slopvac.toml", 6, 1))
    );

    let with_front = at(
        "docs/a.md",
        &Inputs {
            front_matter: &front,
            ..inputs(&NoPresets)
        },
    );
    assert_eq!(with_front.get(&["locale"]), Some(&json!("und")));

    let with_cli = at(
        "docs/a.md",
        &Inputs {
            front_matter: &front,
            cli: &cli,
            ..inputs(&NoPresets)
        },
    );
    assert_eq!(with_cli.get(&["locale"]), Some(&json!("en-GB")));
    assert_eq!(
        with_cli.source(&["locale"]),
        Some(&bare_source(ConfigLayer::Cli))
    );
}

#[test]
fn core_defaults_apply_when_no_layer_sets_a_key() {
    let fx = Fixture::new();
    fx.write("slopvac.toml", "schema_version = 3\n");
    let r = load_at(&fx, "slopvac.toml").resolve(Path::new("a.md"), &inputs(&NoPresets));
    assert_eq!(r.get(&["profile", "name"]), Some(&json!("normal")));
    assert_eq!(
        r.source(&["profile", "name"]),
        Some(&bare_source(ConfigLayer::Core))
    );
    assert_eq!(
        r.get(&["frontmatter", "allow"]),
        Some(&json!(["genre", "locale", "terms"]))
    );
}

#[test]
fn maps_merge_by_key_and_lists_replace() {
    let fx = Fixture::new();
    fx.write(
        "slopvac.toml",
        "schema_version = 3\nexclude = [\"a/**\"]\n\n[rules]\n\"core:one\" = \"warning\"\n\"core:two\" = \"error\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\nexclude_not_allowed = 1\n",
    );
    let err = load(&fx.root().join("slopvac.toml"), &NoPresets).unwrap_err();
    assert_eq!(rejected(err).diagnostics[0].code, Code::UnknownKey);

    fx.write(
        "slopvac.toml",
        "schema_version = 3\nexclude = [\"a/**\"]\n\n[rules]\n\"core:one\" = \"warning\"\n\"core:two\" = \"error\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n\n[overrides.rules]\n\"core:one\" = \"off\"\n\n[overrides.terms]\nallow = [\"x\"]\n",
    );
    let cfg = load_at(&fx, "slopvac.toml");
    let r = cfg.resolve(Path::new("docs/a.md"), &inputs(&NoPresets));
    assert_eq!(r.get(&["rules", "core:one"]), Some(&json!("off")));
    assert_eq!(r.get(&["rules", "core:two"]), Some(&json!("error")));
    assert_eq!(r.get(&["exclude"]), Some(&json!(["a/**"])));
    assert_eq!(
        r.to_json()["rules"],
        json!({"core:one": "off", "core:two": "error"})
    );
}

#[test]
fn an_explicit_threshold_survives_a_profile_change() {
    let fx = Fixture::new();
    fx.write(
        "slopvac.toml",
        "schema_version = 3\nprofile = \"strict\"\n\n[thresholds]\nmax_errors = 4\n\n[[overrides]]\nfiles = [\"docs/**\"]\nprofile = \"relaxed\"\n",
    );
    let r = load_at(&fx, "slopvac.toml").resolve(Path::new("docs/a.md"), &inputs(&NoPresets));
    assert_eq!(r.get(&["profile", "name"]), Some(&json!("relaxed")));
    assert_eq!(r.get(&["thresholds", "max_errors"]), Some(&json!(4)));
}

#[test]
fn bare_and_table_profile_forms_merge_field_by_field() {
    let fx = Fixture::new();
    fx.write(
        "slopvac.toml",
        "schema_version = 3\nprofile = { name = \"strict\", severity_shift = 1 }\n\n[[overrides]]\nfiles = [\"docs/**\"]\nprofile = \"relaxed\"\n",
    );
    let r = load_at(&fx, "slopvac.toml").resolve(Path::new("docs/a.md"), &inputs(&NoPresets));
    assert_eq!(
        r.to_json()["profile"],
        json!({"name": "relaxed", "severity_shift": 1})
    );
}

#[test]
fn overrides_apply_in_file_order_and_the_last_writer_wins() {
    let fx = Fixture::new();
    fx.write(
        "slopvac.toml",
        "schema_version = 3\n\n[[overrides]]\nfiles = [\"docs/**\"]\nlocale = \"en-GB\"\n\n[[overrides]]\nfiles = [\"**/*.md\"]\nlocale = \"en-US\"\n",
    );
    let cfg = load_at(&fx, "slopvac.toml");
    let both = cfg.resolve(Path::new("docs/a.md"), &inputs(&NoPresets));
    assert_eq!(both.get(&["locale"]), Some(&json!("en-US")));
    let history = both.history(&["locale"]);
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].source.line, NonZeroUsize::new(5));
    assert_eq!(history[1].source.line, NonZeroUsize::new(9));
}

#[test]
fn negated_globs_exclude_files_from_a_block() {
    let fx = Fixture::new();
    fx.write(
        "slopvac.toml",
        "schema_version = 3\n\n[[overrides]]\nfiles = [\"docs/**\", \"!docs/generated/**\"]\nlocale = \"en-GB\"\n",
    );
    let cfg = load_at(&fx, "slopvac.toml");
    let locale = |rel: &str| {
        cfg.resolve(Path::new(rel), &inputs(&NoPresets))
            .get(&["locale"])
            .cloned()
    };
    assert_eq!(locale("docs/a.md"), Some(json!("en-GB")));
    assert_eq!(locale("docs/generated/api.md"), None);
}

#[test]
fn a_file_outside_the_config_directory_matches_no_block() {
    let fx = Fixture::new();
    fx.write(
        "slopvac.toml",
        "schema_version = 3\n\n[[overrides]]\nfiles = [\"a.md\"]\nlocale = \"en-GB\"\n",
    );
    let cfg = load_at(&fx, "slopvac.toml");
    let r = cfg.resolve(Path::new("../a.md"), &inputs(&NoPresets));
    assert_eq!(r.get(&["locale"]), None);
}

#[test]
fn extends_apply_in_list_order_and_the_extender_wins() {
    let fx = Fixture::new();
    fx.write(
        "shared/a.toml",
        "schema_version = 3\nlocale = \"en-US\"\n[thresholds]\nmax_errors = 1\nmax_warnings = 1\n",
    );
    fx.write(
        "shared/b.toml",
        "schema_version = 3\n[thresholds]\nmax_errors = 2\n",
    );
    fx.write(
        "proj/slopvac.toml",
        "schema_version = 3\nextends = [\"../shared/a.toml\", \"../shared/b.toml\"]\n[thresholds]\nmax_warnings = 9\n",
    );
    let cfg = load_at(&fx, "proj/slopvac.toml");
    let r = cfg.resolve(Path::new("a.md"), &inputs(&NoPresets));
    assert_eq!(r.get(&["thresholds", "max_errors"]), Some(&json!(2)));
    assert_eq!(r.get(&["thresholds", "max_warnings"]), Some(&json!(9)));
    assert_eq!(
        r.source(&["thresholds", "max_errors"]),
        Some(&src(ConfigLayer::Extends, "../shared/b.toml", 3, 1))
    );
    assert_eq!(r.get(&["locale"]), Some(&json!("en-US")));
}

#[test]
fn blocks_from_an_extended_config_match_from_the_nearest_config_directory() {
    let fx = Fixture::new();
    fx.write(
        "shared/base.toml",
        "schema_version = 3\n\n[[overrides]]\nfiles = [\"docs/**\"]\nlocale = \"en-GB\"\n",
    );
    fx.write(
        "proj/slopvac.toml",
        "schema_version = 3\nextends = [\"../shared/base.toml\"]\n",
    );
    let cfg = load_at(&fx, "proj/slopvac.toml");
    let r = cfg.resolve(Path::new("docs/a.md"), &inputs(&NoPresets));
    assert_eq!(r.get(&["locale"]), Some(&json!("en-GB")));
    let r = cfg.resolve(Path::new("shared/docs/a.md"), &inputs(&NoPresets));
    assert_eq!(r.get(&["locale"]), None);
}

#[test]
fn an_extends_cycle_names_the_chain() {
    let fx = Fixture::new();
    fx.write("a.toml", "schema_version = 3\nextends = [\"b.toml\"]\n");
    fx.write("b.toml", "schema_version = 3\nextends = [\"a.toml\"]\n");
    let err = load(&fx.root().join("a.toml"), &NoPresets).unwrap_err();
    let r = rejected(err);
    assert_eq!(r.file, "b.toml");
    let d = &r.diagnostics[0];
    assert_eq!(d.code, Code::ExtendsCycle);
    assert!(
        d.message.contains("a.toml -> b.toml -> a.toml"),
        "{}",
        d.message
    );
}

#[test]
fn a_missing_extended_file_is_an_error_at_the_entry() {
    let fx = Fixture::new();
    let p = fx.write(
        "slopvac.toml",
        "schema_version = 3\nextends = [\"nope.toml\"]\n",
    );
    let r = rejected(load(&p, &NoPresets).unwrap_err());
    assert_eq!(r.diagnostics[0].code, Code::ExtendsMissing);
    assert_eq!(&r.text[r.diagnostics[0].span.clone()], "\"nope.toml\"");
}

#[test]
fn an_extended_config_must_be_version_three() {
    let fx = Fixture::new();
    fx.write("old.toml", "mode = \"prose\"\n");
    let p = fx.write(
        "slopvac.toml",
        "schema_version = 3\nextends = [\"old.toml\"]\n",
    );
    let err = load(&p, &NoPresets).unwrap_err();
    assert!(err.is_legacy());
    assert_eq!(err.exit_code(), 2);
    assert_eq!(rejected(err).file, "old.toml");
}

#[test]
fn two_blocks_with_the_same_files_are_rejected() {
    let fx = Fixture::new();
    let p = fx.write(
        "slopvac.toml",
        "schema_version = 3\n\n[[overrides]]\nfiles = [\"b\", \"a\"]\n\n[[overrides]]\nfiles = [\" a\", \"b\"]\n",
    );
    let r = rejected(load(&p, &NoPresets).unwrap_err());
    assert_eq!(r.diagnostics[0].code, Code::DuplicateScope);
    assert_eq!(r.diagnostics[0].location(&r.text), (7, 9));
}

#[test]
fn an_override_and_a_genre_block_may_share_files() {
    let fx = Fixture::new();
    let p = fx.write(
        "slopvac.toml",
        "schema_version = 3\n\n[[overrides]]\nfiles = [\"a\"]\n\n[[genres]]\ngenre = \"how-to\"\nfiles = [\"a\"]\n",
    );
    assert!(load(&p, &NoPresets).is_ok());
}

#[test]
fn offline_is_sticky_across_every_layer() {
    let fx = Fixture::new();
    fx.write(
        "slopvac.toml",
        "schema_version = 3\n\n[network]\noffline = true\n",
    );
    let cfg = load_at(&fx, "slopvac.toml");
    let cli = [Setting::bare(
        ConfigLayer::Cli,
        &["network", "offline"],
        json!(false),
    )];
    let r = cfg.resolve(
        Path::new("a.md"),
        &Inputs {
            cli: &cli,
            ..inputs(&NoPresets)
        },
    );
    assert!(r.is_offline());
    assert_eq!(
        r.source(&["network", "offline"]),
        Some(&src(ConfigLayer::Project, "slopvac.toml", 4, 1))
    );
    assert_eq!(r.to_json()["network"]["offline"], json!(true));
}

#[test]
fn an_environment_switch_turns_offline_on_over_a_file_that_says_false() {
    let fx = Fixture::new();
    fx.write(
        "slopvac.toml",
        "schema_version = 3\n[network]\noffline = false\n",
    );
    let cfg = load_at(&fx, "slopvac.toml");
    let env = [Setting::bare(
        ConfigLayer::Env,
        &["network", "offline"],
        json!(true),
    )];
    let r = cfg.resolve(
        Path::new("a.md"),
        &Inputs {
            env: &env,
            ..inputs(&NoPresets)
        },
    );
    assert!(r.is_offline());
    assert_eq!(
        r.source(&["network", "offline"]),
        Some(&bare_source(ConfigLayer::Env))
    );
    assert!(
        !cfg.resolve(Path::new("a.md"), &inputs(&NoPresets))
            .is_offline()
    );
}

#[test]
fn exclude_replaces_the_default_list_and_uses_gitignore_semantics() {
    let fx = Fixture::new();
    fx.write("slopvac.toml", "schema_version = 3\n");
    let defaults = load_at(&fx, "slopvac.toml");
    assert!(defaults.is_excluded(Path::new("node_modules/x/readme.md")));
    assert!(defaults.is_excluded(Path::new("docs/CHANGELOG.md")));
    assert!(!defaults.is_excluded(Path::new("docs/guide.md")));

    fx.write(
        "slopvac.toml",
        "schema_version = 3\nexclude = [\"vendor/**\", \"!vendor/ours/**\"]\n",
    );
    let custom = load_at(&fx, "slopvac.toml");
    assert!(custom.is_excluded(Path::new("vendor/x.md")));
    assert!(!custom.is_excluded(Path::new("vendor/ours/x.md")));
    assert!(!custom.is_excluded(Path::new("node_modules/x/readme.md")));
}

#[test]
fn explain_lists_the_values_a_key_replaced() {
    let fx = Fixture::new();
    fx.write(
        "slopvac.toml",
        "schema_version = 3\nlocale = \"en-GB\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\nlocale = \"en-US\"\n",
    );
    let r = load_at(&fx, "slopvac.toml").resolve(Path::new("docs/a.md"), &inputs(&NoPresets));
    let locale = r
        .explain()
        .into_iter()
        .find(|e| e.path == "locale")
        .unwrap();
    assert_eq!(locale.value, json!("en-US"));
    assert_eq!(
        locale.source,
        src(ConfigLayer::Override, "slopvac.toml", 6, 1)
    );
    assert_eq!(locale.overridden.len(), 1);
    assert_eq!(locale.overridden[0].value, json!("en-GB"));
    assert_eq!(
        locale.overridden[0].source,
        src(ConfigLayer::Project, "slopvac.toml", 2, 1)
    );
}

struct Packs;

impl Presets for Packs {
    fn style(&self, name: &str) -> Option<Vec<Setting>> {
        (name == "acme:house").then(|| {
            vec![
                Setting::bare(ConfigLayer::Style, &["locale"], json!("en-US")),
                Setting::bare(
                    ConfigLayer::Style,
                    &["rules", "core:one"],
                    json!("suggestion"),
                ),
            ]
        })
    }

    fn genre_chain(&self, name: &str) -> Vec<Vec<Setting>> {
        if name == "reference" {
            vec![
                vec![Setting::bare(
                    ConfigLayer::Genre,
                    &["locale"],
                    json!("en-GB"),
                )],
                vec![Setting::bare(
                    ConfigLayer::Genre,
                    &["rules", "core:one"],
                    json!("error"),
                )],
            ]
        } else {
            Vec::new()
        }
    }

    fn pack_config(&self, name: &str) -> Option<PackFile> {
        (name == "acme:base").then(|| PackFile {
            display: "plugin:acme/base.toml".into(),
            text: "schema_version = 3\n[thresholds]\nmax_errors = 7\n".into(),
        })
    }
}

#[test]
fn styles_then_genre_sit_below_extends_and_the_project() {
    let fx = Fixture::new();
    fx.write(
        "slopvac.toml",
        "schema_version = 3\nstyles = [\"acme:house\"]\n\n[rules]\n\"core:other\" = \"off\"\n\n[[genres]]\ngenre = \"reference\"\nfiles = [\"ref/**\"]\n",
    );
    let cfg = load(&fx.root().join("slopvac.toml"), &Packs).unwrap();
    let plain = cfg.resolve(Path::new("a.md"), &inputs(&Packs));
    assert_eq!(plain.get(&["locale"]), Some(&json!("en-US")));
    assert_eq!(
        plain.source(&["locale"]).map(|s| s.layer),
        Some(ConfigLayer::Style)
    );
    assert_eq!(
        plain.get(&["rules", "core:one"]),
        Some(&json!("suggestion"))
    );

    let genre = cfg.resolve(Path::new("ref/a.md"), &inputs(&Packs));
    assert_eq!(genre.get(&["locale"]), Some(&json!("en-GB")));
    assert_eq!(genre.get(&["rules", "core:one"]), Some(&json!("error")));
    assert_eq!(genre.get(&["rules", "core:other"]), Some(&json!("off")));
    assert_eq!(genre.get(&["genre"]), Some(&json!("reference")));
}

#[test]
fn a_front_matter_genre_picks_the_genre_chain() {
    let fx = Fixture::new();
    fx.write("slopvac.toml", "schema_version = 3\n");
    let cfg = load(&fx.root().join("slopvac.toml"), &Packs).unwrap();
    let front = [Setting {
        path: vec!["genre".into()],
        value: json!("reference"),
        source: bare_source(ConfigLayer::FrontMatter),
    }];
    let r = cfg.resolve(
        Path::new("a.md"),
        &Inputs {
            front_matter: &front,
            ..inputs(&Packs)
        },
    );
    assert_eq!(r.get(&["locale"]), Some(&json!("en-GB")));
}

#[test]
fn a_pack_config_can_be_extended() {
    let fx = Fixture::new();
    fx.write(
        "slopvac.toml",
        "schema_version = 3\nextends = [\"acme:base\"]\n",
    );
    let cfg = load(&fx.root().join("slopvac.toml"), &Packs).unwrap();
    let r = cfg.resolve(Path::new("a.md"), &inputs(&Packs));
    assert_eq!(r.get(&["thresholds", "max_errors"]), Some(&json!(7)));
    assert_eq!(
        r.source(&["thresholds", "max_errors"])
            .and_then(|s| s.file.clone()),
        Some("plugin:acme/base.toml".to_string())
    );
}

#[test]
fn an_unknown_pack_config_is_an_error_at_the_entry() {
    let fx = Fixture::new();
    let p = fx.write(
        "slopvac.toml",
        "schema_version = 3\nextends = [\"acme:base\"]\n",
    );
    let r = rejected(load(&p, &NoPresets).unwrap_err());
    assert_eq!(r.diagnostics[0].code, Code::ExtendsMissing);
}

fn git_dir(fx: &Fixture, rel: &str) {
    fs::create_dir_all(fx.root().join(rel).join(".git")).unwrap();
}

#[test]
fn discovery_picks_the_nearest_config_and_does_not_merge_parents() {
    let fx = Fixture::new();
    git_dir(&fx, "");
    fx.write("slopvac.toml", "schema_version = 3\nlocale = \"en-GB\"\n");
    fx.write("pkg/slopvac.toml", "schema_version = 3\n");
    fs::create_dir_all(fx.root().join("pkg/docs")).unwrap();
    fs::create_dir_all(fx.root().join("other")).unwrap();
    let d = Discovery::new(&NoPresets);

    let near = d.nearest(&fx.root().join("pkg/docs")).unwrap().unwrap();
    assert_eq!(near.root, fx.root().join("pkg"));
    let r = near.resolve(Path::new("docs/a.md"), &inputs(&NoPresets));
    assert_eq!(r.get(&["locale"]), None);

    let top = d.nearest(&fx.root().join("other")).unwrap().unwrap();
    assert_eq!(top.root, fx.root());
}

#[test]
fn discovery_stops_at_the_repository_boundary() {
    let fx = Fixture::new();
    fx.write("slopvac.toml", "schema_version = 3\n");
    git_dir(&fx, "repo");
    fs::create_dir_all(fx.root().join("repo/docs")).unwrap();
    let d = Discovery::new(&NoPresets);
    assert!(d.nearest(&fx.root().join("repo/docs")).unwrap().is_none());
}

#[test]
fn discovery_checks_the_config_in_the_repository_root_before_stopping() {
    let fx = Fixture::new();
    git_dir(&fx, "repo");
    fx.write("repo/slopvac.toml", "schema_version = 3\n");
    fs::create_dir_all(fx.root().join("repo/docs")).unwrap();
    let d = Discovery::new(&NoPresets);
    assert!(d.nearest(&fx.root().join("repo/docs")).unwrap().is_some());
}

#[test]
fn a_git_file_also_ends_the_walk() {
    let fx = Fixture::new();
    fx.write("slopvac.toml", "schema_version = 3\n");
    fx.write("wt/.git", "gitdir: elsewhere\n");
    fs::create_dir_all(fx.root().join("wt/docs")).unwrap();
    let d = Discovery::new(&NoPresets);
    assert!(d.nearest(&fx.root().join("wt/docs")).unwrap().is_none());
}

#[test]
fn discovery_loads_each_config_once() {
    let fx = Fixture::new();
    git_dir(&fx, "");
    fx.write("slopvac.toml", "schema_version = 3\n");
    fs::create_dir_all(fx.root().join("a")).unwrap();
    fs::create_dir_all(fx.root().join("b")).unwrap();
    let d = Discovery::new(&NoPresets);
    let first = d.nearest(&fx.root().join("a")).unwrap().unwrap();
    let second = d.nearest(&fx.root().join("b")).unwrap().unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &second));
}

#[test]
fn discovery_treats_old_config_forms_as_2x_configs() {
    let fx = Fixture::new();
    git_dir(&fx, "dot");
    fx.write("dot/.slopvac.toml", "mode = \"prose\"\n");
    git_dir(&fx, "py");
    fx.write(
        "py/pyproject.toml",
        "[project]\nname = \"x\"\n\n[tool.slopvac]\nmode = \"prose\"\n",
    );
    let d = Discovery::new(&NoPresets);

    let dot = d.nearest(&fx.root().join("dot")).unwrap_err();
    assert!(dot.is_legacy());
    assert_eq!(rejected(dot).file, ".slopvac.toml");

    let py = rejected(d.nearest(&fx.root().join("py")).unwrap_err());
    assert_eq!(py.file, "pyproject.toml");
    assert_eq!(&py.text[py.diagnostics[0].span.clone()], "slopvac");
    assert!(py.render().contains("slopvac migrate"));
}

#[test]
fn a_pyproject_without_a_slopvac_table_is_ignored() {
    let fx = Fixture::new();
    git_dir(&fx, "py");
    fx.write("py/pyproject.toml", "[project]\nname = \"x\"\n");
    let d = Discovery::new(&NoPresets);
    assert!(d.nearest(&fx.root().join("py")).unwrap().is_none());
}

#[test]
fn slopvac_toml_wins_over_old_names_in_the_same_directory() {
    let fx = Fixture::new();
    git_dir(&fx, "");
    fx.write("slopvac.toml", "schema_version = 3\n");
    fx.write(".slopvac.toml", "mode = \"prose\"\n");
    let d = Discovery::new(&NoPresets);
    assert!(d.nearest(&fx.root()).unwrap().is_some());
}

#[test]
fn explicit_config_skips_discovery() {
    let fx = Fixture::new();
    let p = fx.write(
        "cfg/custom.toml",
        "schema_version = 3\nlocale = \"en-GB\"\n",
    );
    let d = Discovery::new(&NoPresets);
    let cfg = d.explicit(&p).unwrap();
    assert_eq!(cfg.root, fx.root().join("cfg"));
    assert_eq!(
        cfg.resolve(Path::new("a.md"), &inputs(&NoPresets))
            .get(&["locale"]),
        Some(&Json::from("en-GB"))
    );
}

#[test]
fn explain_text_names_the_root_the_files_and_each_replaced_value() {
    let fx = Fixture::new();
    fx.write(
        "slopvac.toml",
        "schema_version = 3\nlocale = \"en-GB\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\nlocale = \"en-US\"\n",
    );
    let cfg = load_at(&fx, "slopvac.toml");
    let r = cfg.resolve(Path::new("docs/a.md"), &inputs(&NoPresets));
    let text = slopvac_config::render_explain(&cfg, &r.explain());
    let head = format!(
        "config root: {}\nconfig file: slopvac.toml (project)\n",
        fx.root().display()
    );
    assert!(text.starts_with(&head), "{text}");
    assert!(
        text.contains("locale = \"en-US\"  [override slopvac.toml:6:1]\n    replaced \"en-GB\"  [project slopvac.toml:2:1]\n"),
        "{text}"
    );
    assert!(
        text.contains("profile.name = \"normal\"  [core]\n"),
        "{text}"
    );
}

#[test]
fn genre_and_override_blocks_apply_in_the_order_they_are_written() {
    let fx = Fixture::new();
    fx.write(
        "slopvac.toml",
        "schema_version = 3\n\n[[overrides]]\nfiles = [\"docs/**\"]\nlocale = \"en-US\"\n\n[[genres]]\ngenre = \"reference\"\nfiles = [\"docs/**\"]\nlocale = \"en-GB\"\n",
    );
    let r = load_at(&fx, "slopvac.toml").resolve(Path::new("docs/a.md"), &inputs(&NoPresets));
    assert_eq!(r.get(&["locale"]), Some(&json!("en-GB")));

    fx.write(
        "slopvac.toml",
        "schema_version = 3\n\n[[genres]]\ngenre = \"reference\"\nfiles = [\"docs/**\"]\nlocale = \"en-GB\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\nlocale = \"en-US\"\n",
    );
    let r = load_at(&fx, "slopvac.toml").resolve(Path::new("docs/a.md"), &inputs(&NoPresets));
    assert_eq!(r.get(&["locale"]), Some(&json!("en-US")));
}

#[test]
fn a_flag_can_pick_the_genre_before_its_chain_loads() {
    let fx = Fixture::new();
    fx.write("slopvac.toml", "schema_version = 3\n");
    let cfg = load(&fx.root().join("slopvac.toml"), &Packs).unwrap();
    let cli = [Setting::bare(
        ConfigLayer::Cli,
        &["genre"],
        json!("reference"),
    )];
    let r = cfg.resolve(
        Path::new("a.md"),
        &Inputs {
            cli: &cli,
            ..inputs(&Packs)
        },
    );
    assert_eq!(r.get(&["locale"]), Some(&json!("en-GB")));
}

#[test]
fn read_failures_have_a_stable_code_and_no_system_text() {
    let missing = load(Path::new("missing/slopvac.toml"), &NoPresets).unwrap_err();
    assert_eq!(missing.code(), Code::Io);
    assert_eq!(
        missing.to_string(),
        "cannot read missing/slopvac.toml: no such file"
    );

    let fx = Fixture::new();
    let p = fx.write("slopvac.toml", "");
    fs::write(&p, [0xff, 0xfe, 0x41]).unwrap();
    let bad = load(&p, &NoPresets).unwrap_err();
    assert_eq!(bad.code(), Code::Io);
    assert!(bad.to_string().ends_with(": not valid UTF-8"), "{bad}");
    assert_eq!(bad.exit_code(), 2);
}

#[test]
fn a_budget_of_none_clears_the_budget_of_a_lower_layer() {
    let fx = Fixture::new();
    fx.write(
        "slopvac.toml",
        "schema_version = 3\n\n[categories.prose]\nmax_per_100_words = 2.5\n\n[[overrides]]\nfiles = [\"docs/**\"]\n\n[overrides.categories.prose]\nmax_per_100_words = \"none\"\n",
    );
    let cfg = load_at(&fx, "slopvac.toml");
    let plain = cfg.resolve(Path::new("a.md"), &inputs(&NoPresets));
    assert_eq!(plain.budget("prose"), Some(2.5));
    let cleared = cfg.resolve(Path::new("docs/a.md"), &inputs(&NoPresets));
    assert_eq!(cleared.budget("prose"), None);
    assert_eq!(
        cleared.get(&["categories", "prose", "max_per_100_words"]),
        Some(&Json::Null)
    );
    assert_eq!(
        cleared.to_json()["categories"]["prose"]["max_per_100_words"],
        Json::Null
    );
    let history = cleared.history(&["categories", "prose", "max_per_100_words"]);
    assert_eq!(history[0].value, json!(2.5));
    assert_eq!(history[1].source.layer, ConfigLayer::Override);
}

struct Styled(Vec<Setting>);

impl Presets for Styled {
    fn style(&self, name: &str) -> Option<Vec<Setting>> {
        (name == "acme:house").then(|| self.0.clone())
    }

    fn genre_chain(&self, _name: &str) -> Vec<Vec<Setting>> {
        Vec::new()
    }

    fn pack_config(&self, _name: &str) -> Option<PackFile> {
        None
    }
}

fn style_rule(path: &[&str], level: &str) -> Setting {
    Setting::bare(ConfigLayer::Style, path, json!(level))
}

/// A name, the style settings, the config body, and the expected level with its entry kind.
type LevelCase = (
    &'static str,
    Vec<Setting>,
    &'static str,
    Option<(&'static str, slopvac_config::LevelFrom)>,
);

#[test]
fn level_precedence_follows_the_profile_contract() {
    // (style settings, project/override config body, expected level and entry kind)
    use slopvac_config::LevelFrom::{Category, Rule};
    let cases: [LevelCase; 9] = [
        (
            "a rule at off holds against a category in a higher layer",
            vec![],
            "[rules]\n\"core:r\" = \"off\"\n\n[[overrides]]\nfiles = [\"**\"]\n[overrides.categories.cat]\nseverity = \"error\"\n",
            Some(("off", Rule)),
        ),
        (
            "a rule in a higher layer revives it",
            vec![style_rule(&["rules", "core:r"], "off")],
            "[rules]\n\"core:r\" = \"warning\"\n",
            Some(("warning", Rule)),
        ),
        (
            "in one layer a rule beats a category",
            vec![],
            "[rules]\n\"core:r\" = \"warning\"\n[categories.cat]\nseverity = \"error\"\n",
            Some(("warning", Rule)),
        ),
        (
            "a category in a higher layer beats a rule that is not off",
            vec![],
            "[rules]\n\"core:r\" = \"warning\"\n\n[[overrides]]\nfiles = [\"**\"]\n[overrides.categories.cat]\nseverity = \"error\"\n",
            Some(("error", Category)),
        ),
        (
            "a category level rises to the minimum severity",
            vec![],
            "[categories.cat]\nseverity = \"suggestion\"\nminimum_severity = \"warning\"\n",
            Some(("warning", Category)),
        ),
        (
            "off as the minimum severity sets no floor",
            vec![style_rule(
                &["categories", "cat", "minimum_severity"],
                "warning",
            )],
            "[categories.cat]\nseverity = \"suggestion\"\nminimum_severity = \"off\"\n",
            Some(("suggestion", Category)),
        ),
        (
            "a rule entry does not rise to the floor",
            vec![],
            "[rules]\n\"core:r\" = \"suggestion\"\n[categories.cat]\nminimum_severity = \"error\"\n",
            Some(("suggestion", Rule)),
        ),
        (
            "a style category level loses to a project rule",
            vec![style_rule(&["categories", "cat", "severity"], "error")],
            "[rules]\n\"core:r\" = \"suggestion\"\n",
            Some(("suggestion", Rule)),
        ),
        ("a rule no layer names has no level", vec![], "", None),
    ];
    for (name, style, body, want) in cases {
        let fx = Fixture::new();
        fx.write(
            "slopvac.toml",
            &format!("schema_version = 3\nstyles = [\"acme:house\"]\n\n{body}"),
        );
        let presets = Styled(style);
        let cfg = load(&fx.root().join("slopvac.toml"), &presets).unwrap();
        let r = cfg.resolve(Path::new("a.md"), &inputs(&presets));
        let got = r.level("core:r", Some("cat")).map(|l| (l.level, l.from));
        assert_eq!(got, want.map(|(l, f)| (l.to_owned(), f)), "{name}");
    }
}

/// A rule or category may be written qualified (`core:r`) or bare (`r`). The layer that wrote
/// the entry decides, not the spelling of its key.
#[test]
fn level_by_any_key_follows_layers_not_spelling() {
    use slopvac_config::LevelFrom::{Category, Rule};
    type Want = Option<(&'static str, slopvac_config::LevelFrom)>;
    let cases: Vec<(&str, &str, &str, Want)> = vec![
        (
            "a project bare rule key beats an extended qualified one",
            "[rules]\n\"core:r\" = \"error\"\n",
            "[rules]\nr = \"suggestion\"\n",
            Some(("suggestion", Rule)),
        ),
        (
            "a project qualified rule key beats an extended bare one",
            "[rules]\nr = \"error\"\n",
            "[rules]\n\"core:r\" = \"suggestion\"\n",
            Some(("suggestion", Rule)),
        ),
        (
            "a qualified category key is read",
            "",
            "[categories]\n\"core:cat\" = \"warning\"\n",
            Some(("warning", Category)),
        ),
        (
            "a project bare category key beats an extended qualified one",
            "[categories]\n\"core:cat\" = \"error\"\n",
            "[categories]\ncat = \"suggestion\"\n",
            Some(("suggestion", Category)),
        ),
        (
            "a project category beats an extended rule entry",
            "[rules]\nr = \"error\"\n",
            "[categories]\n\"core:cat\" = \"suggestion\"\n",
            Some(("suggestion", Category)),
        ),
        (
            "a rule entry at off holds against a category entry in the same file",
            "",
            "[rules]\nr = \"off\"\n[categories]\n\"core:cat\" = \"error\"\n",
            Some(("off", Rule)),
        ),
    ];
    for (name, base, project, want) in cases {
        let fx = Fixture::new();
        fx.write("base.toml", &format!("schema_version = 3\n\n{base}"));
        fx.write(
            "slopvac.toml",
            &format!("schema_version = 3\nextends = [\"base.toml\"]\n\n{project}"),
        );
        let cfg = load(&fx.root().join("slopvac.toml"), &NoPresets).unwrap();
        let r = cfg.resolve(Path::new("a.md"), &inputs(&NoPresets));
        let got = r
            .level_by_any(&["core:r", "r"], &["core:cat", "cat"])
            .map(|l| (l.level, l.from));
        assert_eq!(got, want.map(|(l, f)| (l.to_owned(), f)), "{name}");
    }
}

/// When one layer writes the same rule or category under both spellings, the spelling listed
/// first in the call wins; and a category floor set under one spelling holds against a level
/// set under the other until a higher layer sets `off` under either.
#[test]
fn level_by_any_breaks_ties_and_merges_floors_across_spellings() {
    use slopvac_config::LevelFrom::{Category, Rule};
    type Want = Option<(&'static str, slopvac_config::LevelFrom)>;
    type Keys = (&'static [&'static str], &'static [&'static str]);
    let qualified_first: Keys = (&["core:r", "r"], &["core:cat", "cat"]);
    let bare_first: Keys = (&["r", "core:r"], &["cat", "core:cat"]);
    let cases: Vec<(&str, &str, &str, Keys, Want)> = vec![
        (
            "same file, rules: the first listed spelling wins (qualified)",
            "",
            "[rules]\nr = \"error\"\n\"core:r\" = \"suggestion\"\n",
            qualified_first,
            Some(("suggestion", Rule)),
        ),
        (
            "same file, rules: the first listed spelling wins (bare)",
            "",
            "[rules]\nr = \"error\"\n\"core:r\" = \"suggestion\"\n",
            bare_first,
            Some(("error", Rule)),
        ),
        (
            "same file, categories: the first listed spelling wins",
            "",
            "[categories]\ncat = \"error\"\n\"core:cat\" = \"warning\"\n",
            qualified_first,
            Some(("warning", Category)),
        ),
        (
            "a floor under the qualified spelling lifts a level set under the bare one",
            "[categories.\"core:cat\"]\nminimum_severity = \"error\"\n",
            "[categories]\ncat = \"suggestion\"\n",
            qualified_first,
            Some(("error", Category)),
        ),
        (
            "a floor under the bare spelling lifts a level set under the qualified one",
            "[categories.cat]\nminimum_severity = \"warning\"\n",
            "[categories]\n\"core:cat\" = \"suggestion\"\n",
            qualified_first,
            Some(("warning", Category)),
        ),
        (
            "off under the other spelling in a higher layer clears the floor",
            "[categories.cat]\nminimum_severity = \"error\"\n",
            "[categories.\"core:cat\"]\nseverity = \"suggestion\"\nminimum_severity = \"off\"\n",
            qualified_first,
            Some(("suggestion", Category)),
        ),
        (
            "off in a lower layer does not cancel a floor from a higher one",
            "[categories.\"core:cat\"]\nminimum_severity = \"off\"\n",
            "[categories.cat]\nseverity = \"suggestion\"\nminimum_severity = \"warning\"\n",
            qualified_first,
            Some(("warning", Category)),
        ),
    ];
    for (name, base, project, (rules, categories), want) in cases {
        let fx = Fixture::new();
        fx.write("base.toml", &format!("schema_version = 3\n\n{base}"));
        fx.write(
            "slopvac.toml",
            &format!("schema_version = 3\nextends = [\"base.toml\"]\n\n{project}"),
        );
        let cfg = load(&fx.root().join("slopvac.toml"), &NoPresets).unwrap();
        let r = cfg.resolve(Path::new("a.md"), &inputs(&NoPresets));
        let got = r.level_by_any(rules, categories).map(|l| (l.level, l.from));
        assert_eq!(got, want.map(|(l, f)| (l.to_owned(), f)), "{name}");
    }
}

#[test]
fn a_missing_extends_target_is_named_without_the_directories_above_the_root() {
    let fx = Fixture::new();
    let outside = tempfile::tempdir().unwrap();
    let outside_file = outside.path().join("my base.toml");
    let inside_file = fx.root().join("sub dir").join("gone.toml");
    for (target, want) in [
        (outside_file.as_path(), "`<external>/my base.toml`"),
        (inside_file.as_path(), "`sub dir/gone.toml`"),
    ] {
        fx.write(
            "slopvac.toml",
            &format!(
                "schema_version = 3\nextends = [{:?}]\n",
                target.display().to_string()
            ),
        );
        let err = load(&fx.root().join("slopvac.toml"), &NoPresets).unwrap_err();
        let diagnostic = &rejected(err).diagnostics[0];
        assert_eq!(diagnostic.code, Code::ExtendsMissing);
        assert!(
            diagnostic
                .message
                .starts_with(&format!("cannot read the extended config {want}")),
            "{}",
            diagnostic.message
        );
        assert!(
            !diagnostic
                .message
                .contains(outside.path().to_str().unwrap())
        );
    }
}

#[test]
fn a_read_failure_renders_with_its_code() {
    let err = load(Path::new("missing/slopvac.toml"), &NoPresets).unwrap_err();
    assert_eq!(
        err.render(),
        "error[config-io]: cannot read missing/slopvac.toml: no such file"
    );
}

#[test]
fn a_missing_explicit_config_is_named_as_the_user_typed_it() {
    let fx = Fixture::new();
    let d = Discovery::new(&NoPresets);
    let err = d
        .explicit_in(&fx.root(), Path::new("conf/team.toml"))
        .unwrap_err();
    assert_eq!(err.to_string(), "cannot read conf/team.toml: no such file");
    assert!(!err.render().contains(fx.root().to_str().unwrap()));
}

/// `canonicalize` yields `\\?\C:\...` on Windows, and a user writes `C:\...`. Both name one
/// drive, so the extended file shows relative to the root and not as external.
#[cfg(windows)]
#[test]
fn an_extends_path_in_plain_drive_form_shows_relative_to_a_verbatim_root() {
    let fx = Fixture::new();
    let base = fx.write(
        "shared/base.toml",
        "schema_version = 3\nlocale = \"en-US\"\n",
    );
    let plain = base.to_string_lossy().replace(r"\\?\", "");
    fx.write(
        "proj/slopvac.toml",
        &format!("schema_version = 3\nextends = ['{plain}']\n"),
    );
    let cfg = load_at(&fx, "proj/slopvac.toml");
    let r = cfg.resolve(Path::new("a.md"), &inputs(&NoPresets));
    assert_eq!(
        r.source(&["locale"]),
        Some(&src(ConfigLayer::Extends, "../shared/base.toml", 2, 1))
    );
}

/// A sparse file of `len` bytes, so a test crosses a cap without writing the bytes.
fn sparse(path: &Path, len: u64) {
    let file = fs::File::create(path).unwrap();
    file.set_len(len).unwrap();
}

#[test]
fn a_config_one_byte_over_the_cap_is_refused_with_its_code() {
    let fx = Fixture::new();
    let p = fx.root().join("slopvac.toml");
    sparse(&p, slopvac_config::MAX_CONFIG_BYTES + 1);
    let err = load(&p, &NoPresets).unwrap_err();
    assert_eq!(err.code(), Code::TooLarge);
    assert_eq!(err.exit_code(), 2);
    assert!(
        err.render().starts_with("error[config-too-large]: "),
        "{}",
        err.render()
    );
}

#[test]
fn a_config_exactly_at_the_cap_is_read() {
    let fx = Fixture::new();
    let p = fx.write("slopvac.toml", "schema_version = 3\n");
    let pad = "#".repeat(usize::try_from(slopvac_config::MAX_CONFIG_BYTES).unwrap() - 19);
    fs::write(&p, format!("schema_version = 3\n{pad}")).unwrap();
    assert!(load(&p, &NoPresets).is_ok());
}

#[test]
fn an_extended_config_over_the_cap_is_refused_with_its_code() {
    let fx = Fixture::new();
    sparse(
        &fx.root().join("big.toml"),
        slopvac_config::MAX_CONFIG_BYTES + 1,
    );
    let p = fx.write(
        "slopvac.toml",
        "schema_version = 3\nextends = [\"big.toml\"]\n",
    );
    let err = load(&p, &NoPresets).unwrap_err();
    assert_eq!(err.code(), Code::TooLarge);
}

#[test]
fn a_legacy_config_file_over_the_cap_is_refused_during_discovery() {
    let fx = Fixture::new();
    for name in [".slopvac.toml", "pyproject.toml"] {
        let dir = name.trim_start_matches('.');
        git_dir(&fx, dir);
        sparse(
            &fx.root().join(dir).join(name),
            slopvac_config::MAX_CONFIG_BYTES + 1,
        );
        let err = Discovery::new(&NoPresets)
            .nearest(&fx.root().join(dir))
            .unwrap_err();
        assert_eq!(err.code(), Code::TooLarge, "{name}");
    }
}

/// Write `levels + 1` files where `d<i>.toml` extends `d<i+1>.toml` twice, so the graph has
/// `levels + 1` files and expands to `2^(levels + 1) - 2` applications below the root.
fn diamond_chain(fx: &Fixture, levels: usize) {
    for i in 0..levels {
        let next = format!("d{}.toml", i + 1);
        fx.write(
            &format!("d{i}.toml"),
            &format!("schema_version = 3\nextends = [\"{next}\", \"{next}\"]\n"),
        );
    }
    fx.write(
        &format!("d{levels}.toml"),
        "schema_version = 3\n[thresholds]\nmax_warnings = 7\n",
    );
}

#[test]
fn a_diamond_chain_expands_in_full_and_keeps_every_application_in_order() {
    let fx = Fixture::new();
    diamond_chain(&fx, 5);
    let cfg = load_at(&fx, "d0.toml");
    // 2 + 4 + 8 + 16 + 32 applications below the root, the leaf twice per parent, plus the root.
    assert_eq!(cfg.files.len(), 2 + 4 + 8 + 16 + 32 + 1);
    let shown: Vec<&str> = cfg.files.iter().map(|f| f.display.as_str()).collect();
    assert_eq!(&shown[..3], ["d5.toml", "d5.toml", "d4.toml"]);
    assert_eq!(shown.last(), Some(&"d0.toml"));
    assert_eq!(
        cfg.project_value(&["thresholds", "max_warnings"]),
        Some(json!(7))
    );
}

/// The budget data of a refused load.
fn budget_data(err: ConfigError) -> BudgetData {
    assert_eq!(err.code(), Code::BudgetExceeded);
    assert_eq!(err.exit_code(), 2);
    let r = rejected(err);
    *r.diagnostics
        .into_iter()
        .next()
        .and_then(|d| d.data)
        .expect("a budget diagnostic carries data")
}

/// Text of exactly `len` bytes that is a valid config: the version line, then a comment.
fn padded(len: usize) -> String {
    let head = "schema_version = 3\n";
    format!("{head}#{}\n", "x".repeat(len - head.len() - 2))
}

/// A flat project config that extends `count` files named `f<i>.toml`, written with `size(i)` bytes.
fn flat_graph(fx: &Fixture, count: usize, size: impl Fn(usize) -> usize) -> PathBuf {
    let names: Vec<String> = (0..count).map(|i| format!("\"f{i}.toml\"")).collect();
    for i in 0..count {
        fx.write(&format!("f{i}.toml"), &padded(size(i)));
    }
    fx.write(
        "slopvac.toml",
        &format!("schema_version = 3\nextends = [{}]\n", names.join(", ")),
    )
}

#[test]
fn an_extends_graph_applying_exactly_the_file_limit_loads() {
    let fx = Fixture::new();
    let root = flat_graph(&fx, slopvac_config::MAX_EXTENDS_FILES, |_| 64);
    let cfg = load(&root, &NoPresets).unwrap();
    assert_eq!(cfg.files.len(), slopvac_config::MAX_EXTENDS_FILES + 1);
}

#[test]
fn an_extends_graph_applying_one_file_past_the_limit_is_refused() {
    let fx = Fixture::new();
    let root = flat_graph(&fx, slopvac_config::MAX_EXTENDS_FILES + 1, |_| 64);
    let data = budget_data(load(&root, &NoPresets).unwrap_err());
    assert_eq!(data.budget, ConfigBudget::ExtendsFiles);
    assert_eq!(data.budget.as_str(), "extends-files");
    assert_eq!(data.limit, slopvac_config::MAX_EXTENDS_FILES as u64);
    assert_eq!(data.path, "slopvac.toml");
}

#[test]
fn a_diamond_chain_past_the_file_limit_is_refused_by_expansion_not_by_distinct_files() {
    let fx = Fixture::new();
    diamond_chain(&fx, 9);
    let data = budget_data(load(&fx.root().join("d0.toml"), &NoPresets).unwrap_err());
    assert_eq!(data.budget, ConfigBudget::ExtendsFiles);
}

/// A graph whose files add up to `MAX_EXTENDS_BYTES + extra` bytes, each file within the per-file cap.
fn byte_graph(fx: &Fixture, extra: usize) -> PathBuf {
    let cap = usize::try_from(slopvac_config::MAX_CONFIG_BYTES).unwrap();
    let files = usize::try_from(slopvac_config::MAX_EXTENDS_BYTES).unwrap() / cap;
    // The project config counts too, so the last file takes what is left of the total.
    let root_len = format!(
        "schema_version = 3\nextends = [{}]\n",
        (0..files)
            .map(|i| format!("\"f{i}.toml\""))
            .collect::<Vec<_>>()
            .join(", ")
    )
    .len();
    flat_graph(fx, files, |i| {
        if i + 1 == files {
            cap - root_len + extra
        } else {
            cap
        }
    })
}

#[test]
fn an_extends_graph_reading_exactly_the_byte_limit_loads() {
    let fx = Fixture::new();
    let root = byte_graph(&fx, 0);
    assert!(load(&root, &NoPresets).is_ok());
}

#[test]
fn an_extends_graph_reading_one_byte_past_the_limit_is_refused() {
    let fx = Fixture::new();
    let root = byte_graph(&fx, 1);
    let data = budget_data(load(&root, &NoPresets).unwrap_err());
    assert_eq!(data.budget, ConfigBudget::ExtendsBytes);
    assert_eq!(data.budget.as_str(), "extends-bytes");
    assert_eq!(data.limit, slopvac_config::MAX_EXTENDS_BYTES);
}

#[test]
fn a_file_extended_from_many_places_counts_once_in_bytes() {
    let fx = Fixture::new();
    // Nine applications of one 1 MiB file would be 9 MiB if each counted, past the 8 MiB limit.
    fx.write(
        "big.toml",
        &padded(usize::try_from(slopvac_config::MAX_CONFIG_BYTES).unwrap()),
    );
    let root = fx.write(
        "slopvac.toml",
        &format!(
            "schema_version = 3\nextends = [{}]\n",
            ["\"big.toml\""; 9].join(", ")
        ),
    );
    let cfg = load(&root, &NoPresets).unwrap();
    assert_eq!(cfg.files.len(), 10);
}

/// A chain `c0 -> c1 -> ... -> c<levels>`: `levels` extended levels below the project config.
fn chain(fx: &Fixture, levels: usize) {
    for i in 0..levels {
        fx.write(
            &format!("c{i}.toml"),
            &format!("schema_version = 3\nextends = [\"c{}.toml\"]\n", i + 1),
        );
    }
    fx.write(&format!("c{levels}.toml"), "schema_version = 3\n");
}

#[test]
fn an_extends_chain_at_the_depth_limit_loads() {
    let fx = Fixture::new();
    let depth = slopvac_config::MAX_EXTENDS_DEPTH;
    chain(&fx, depth);
    assert_eq!(load_at(&fx, "c0.toml").files.len(), depth + 1);
}

#[test]
fn an_extends_chain_one_level_past_the_depth_limit_is_refused() {
    let fx = Fixture::new();
    let depth = slopvac_config::MAX_EXTENDS_DEPTH;
    chain(&fx, depth + 1);
    let data = budget_data(load(&fx.root().join("c0.toml"), &NoPresets).unwrap_err());
    assert_eq!(data.budget, ConfigBudget::ExtendsDepth);
    assert_eq!(data.budget.as_str(), "extends-depth");
    assert_eq!(data.limit, depth as u64);
    assert_eq!(data.path, format!("c{depth}.toml"));
}

#[test]
fn a_budget_message_names_the_relative_path_and_the_limit_only() {
    let fx = Fixture::new();
    chain(&fx, slopvac_config::MAX_EXTENDS_DEPTH + 1);
    let err = load(&fx.root().join("c0.toml"), &NoPresets).unwrap_err();
    let shown = err.render();
    assert!(shown.contains("32 levels"), "{shown}");
    assert!(
        !shown.contains(fx.root().to_str().unwrap()),
        "an absolute path leaked: {shown}"
    );
}

#[test]
fn a_two_file_cycle_reports_the_cycle_and_not_a_budget() {
    let fx = Fixture::new();
    fx.write("a.toml", "schema_version = 3\nextends = [\"b.toml\"]\n");
    fx.write("b.toml", "schema_version = 3\nextends = [\"a.toml\"]\n");
    let err = load(&fx.root().join("a.toml"), &NoPresets).unwrap_err();
    assert_eq!(err.code(), Code::ExtendsCycle);
    assert_eq!(err.exit_code(), 2);
}

fn extends_line(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|n| format!("\"{n}\"")).collect();
    format!("schema_version = 3\nextends = [{}]\n", quoted.join(", "))
}

#[test]
fn applications_waiting_on_an_ancestor_count_against_the_file_limit() {
    let max = slopvac_config::MAX_EXTENDS_FILES;
    // Project config -> one intermediate -> leaves: the intermediate is one more application.
    for (leaves, ok) in [(max - 1, true), (max, false)] {
        let fx = Fixture::new();
        let names: Vec<String> = (0..leaves).map(|i| format!("l{i}.toml")).collect();
        for n in &names {
            fx.write(n, "schema_version = 3\n");
        }
        fx.write("mid.toml", &extends_line(&names));
        let root = fx.write("slopvac.toml", &extends_line(&["mid.toml".to_owned()]));
        let result = load(&root, &NoPresets);
        if ok {
            assert_eq!(result.unwrap().files.len(), max + 1);
        } else {
            let data = budget_data(result.unwrap_err());
            assert_eq!(data.budget, ConfigBudget::ExtendsFiles);
        }
    }
}

/// `slopvac.toml` extends `s.toml` (which extends `t.toml`) first, then a chain of `chain`
/// files whose last one extends `s.toml` again.
fn shared_subtree_below_a_chain(fx: &Fixture, chain: usize) -> PathBuf {
    fx.write("t.toml", "schema_version = 3\n");
    fx.write("s.toml", &extends_line(&["t.toml".to_owned()]));
    for i in 0..chain {
        let next = if i + 1 == chain {
            "s.toml".to_owned()
        } else {
            format!("c{}.toml", i + 1)
        };
        fx.write(&format!("c{i}.toml"), &extends_line(&[next]));
    }
    fx.write(
        "slopvac.toml",
        &extends_line(&["s.toml".to_owned(), "c0.toml".to_owned()]),
    )
}

#[test]
fn a_cached_subtree_is_checked_against_the_depth_limit_where_it_applies_again() {
    let depth = slopvac_config::MAX_EXTENDS_DEPTH;
    // The chain of `n` files puts `s.toml` at level n + 1 and `t.toml` at level n + 2.
    let fx = Fixture::new();
    let root = shared_subtree_below_a_chain(&fx, depth - 2);
    assert!(load(&root, &NoPresets).is_ok());
    let fx = Fixture::new();
    let root = shared_subtree_below_a_chain(&fx, depth - 1);
    let data = budget_data(load(&root, &NoPresets).unwrap_err());
    assert_eq!(data.budget, ConfigBudget::ExtendsDepth);
    assert_eq!(data.limit, depth as u64);
}

/// Text of exactly `len` bytes that starts with `head` and ends in a comment.
fn exact(head: &str, len: usize) -> String {
    format!("{head}#{}\n", "x".repeat(len - head.len() - 2))
}

/// Project config -> `c1` -> `c2` -> `big_user.toml`, which extends the 1 MiB file `big.toml`
/// `times` times. `big_user.toml` is sized so that the applied text of the three extended files
/// besides `big.toml` is exactly 1 MiB plus `extra` bytes.
fn amplification(fx: &Fixture, times: usize, extra: usize) -> PathBuf {
    let cap = usize::try_from(slopvac_config::MAX_CONFIG_BYTES).unwrap();
    fx.write("big.toml", &exact("schema_version = 3\n", cap));
    let c2 = extends_line(&["big_user.toml".to_owned()]);
    let c1 = extends_line(&["c2.toml".to_owned()]);
    fx.write("c2.toml", &c2);
    fx.write("c1.toml", &c1);
    let names = vec!["big.toml".to_owned(); times];
    let user_len = cap - c1.len() - c2.len() + extra;
    fx.write("big_user.toml", &exact(&extends_line(&names), user_len));
    fx.write("slopvac.toml", &extends_line(&["c1.toml".to_owned()]))
}

#[test]
fn applied_text_exactly_at_the_limit_loads_and_shares_one_parse() {
    let fx = Fixture::new();
    // 63 MiB of `big.toml` plus exactly 1 MiB of the other three files is the 64 MiB limit.
    let root = amplification(&fx, 63, 0);
    let cfg = load(&root, &NoPresets).unwrap();
    let bigs: Vec<_> = cfg
        .files
        .iter()
        .filter(|f| f.display == "big.toml")
        .collect();
    assert_eq!(bigs.len(), 63);
    assert!(bigs.windows(2).all(|w| std::sync::Arc::ptr_eq(w[0], w[1])));
}

#[test]
fn applied_text_one_byte_past_the_limit_is_refused() {
    let fx = Fixture::new();
    let root = amplification(&fx, 63, 1);
    let data = budget_data(load(&root, &NoPresets).unwrap_err());
    assert_eq!(data.budget, ConfigBudget::ExtendsAppliedBytes);
    assert_eq!(data.budget.as_str(), "extends-applied-bytes");
    assert_eq!(data.limit, slopvac_config::MAX_EXTENDS_APPLIED_BYTES);
}

#[test]
fn repeating_a_large_file_two_hundred_times_is_refused_without_applying_it() {
    let fx = Fixture::new();
    // 224 applications of a 1 MiB file pass the file and read budgets but would apply 224 MiB.
    let root = amplification(&fx, 224, 0);
    let data = budget_data(load(&root, &NoPresets).unwrap_err());
    assert_eq!(data.budget, ConfigBudget::ExtendsAppliedBytes);
}

fn mkfifo(path: &Path) {
    let made = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .unwrap();
    assert!(made.success());
}

#[test]
fn a_fifo_config_is_refused_with_config_io() {
    let fx = Fixture::new();
    let fifo = fx.root().join("slopvac.toml");
    mkfifo(&fifo);
    let path = fifo.clone();
    let err = common::within_timeout(&fifo, move || load(&path, &NoPresets).unwrap_err());
    assert_eq!(err.code(), Code::Io);
    assert!(
        err.render().starts_with("error[config-io]"),
        "{}",
        err.render()
    );
}

#[test]
fn a_fifo_extends_target_is_refused_at_the_entry() {
    let fx = Fixture::new();
    let fifo = fx.root().join("pipe.toml");
    mkfifo(&fifo);
    let p = fx.write(
        "slopvac.toml",
        "schema_version = 3\nextends = [\"pipe.toml\"]\n",
    );
    let err = common::within_timeout(&fifo, move || load(&p, &NoPresets).unwrap_err());
    let r = rejected(err);
    assert_eq!(r.diagnostics[0].code, Code::ExtendsMissing);
    assert!(
        r.diagnostics[0].message.contains("not a regular file"),
        "{}",
        r.diagnostics[0].message
    );
}

#[test]
fn symlinked_configs_still_load() {
    let fx = Fixture::new();
    fx.write(
        "real/base.toml",
        "schema_version = 3\n[thresholds]\nmax_warnings = 9\n",
    );
    let root = fx.root();
    std::os::unix::fs::symlink(root.join("real/base.toml"), root.join("link-base.toml")).unwrap();
    let p = fx.write(
        "slopvac.toml",
        "schema_version = 3\nextends = [\"link-base.toml\"]\n",
    );
    std::os::unix::fs::symlink(&p, root.join("link.toml")).unwrap();
    let probe = root.join("link.toml");
    let loaded = common::within_timeout(&probe.clone(), move || load(&probe, &NoPresets));
    assert!(loaded.is_ok(), "{:?}", loaded.err().map(|e| e.render()));
}

#[test]
fn a_dangling_extends_link_is_missing_as_before() {
    let fx = Fixture::new();
    let root = fx.root();
    std::os::unix::fs::symlink(root.join("gone.toml"), root.join("dead.toml")).unwrap();
    let p = fx.write(
        "slopvac.toml",
        "schema_version = 3\nextends = [\"dead.toml\"]\n",
    );
    let r = rejected(load(&p, &NoPresets).unwrap_err());
    assert_eq!(r.diagnostics[0].code, Code::ExtendsMissing);
}

/// `count` distinct blocks of `table`, each scoping `<prefix><i>/*.md`.
fn blocks_text(table: &str, prefix: &str, extra: &str, count: usize) -> String {
    (0..count)
        .map(|i| {
            [
                "\n[[",
                table,
                "]]\nfiles = [\"",
                prefix,
                &i.to_string(),
                "/*.md\"]\n",
                extra,
            ]
            .concat()
        })
        .collect()
}

/// A config with `count` distinct `[[overrides]]` blocks.
fn block_config(count: usize) -> String {
    format!(
        "schema_version = 3\n{}",
        blocks_text("overrides", "dir", "", count)
    )
}

#[test]
fn a_file_with_exactly_the_block_limit_loads() {
    let fx = Fixture::new();
    let p = fx.write(
        "slopvac.toml",
        &block_config(slopvac_config::MAX_FILE_BLOCKS),
    );
    let cfg = load(&p, &NoPresets).unwrap();
    assert_eq!(cfg.files[0].blocks.len(), slopvac_config::MAX_FILE_BLOCKS);
}

#[test]
fn a_file_one_block_past_the_limit_is_refused_with_its_budget() {
    let fx = Fixture::new();
    let p = fx.write(
        "slopvac.toml",
        &block_config(slopvac_config::MAX_FILE_BLOCKS + 1),
    );
    let data = budget_data(load(&p, &NoPresets).unwrap_err());
    assert_eq!(data.budget, ConfigBudget::FileBlocks);
    assert_eq!(data.budget.as_str(), "file-blocks");
    assert_eq!(data.limit, slopvac_config::MAX_FILE_BLOCKS as u64);
    assert_eq!(data.path, "slopvac.toml");
}

#[test]
fn blocks_split_across_both_tables_count_together() {
    let fx = Fixture::new();
    let half = slopvac_config::MAX_FILE_BLOCKS / 2 + 1;
    let mut text = block_config(half);
    text.push_str(&blocks_text("genres", "g", "genre = \"docs\"\n", half));
    let p = fx.write("slopvac.toml", &text);
    let data = budget_data(load(&p, &NoPresets).unwrap_err());
    assert_eq!(data.budget, ConfigBudget::FileBlocks);
}
