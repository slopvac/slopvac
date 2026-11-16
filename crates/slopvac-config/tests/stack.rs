//! Layer order, merge rules, provenance and discovery, through the public interface.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value as Json, json};
use slopvac_config::{
    Code, ConfigError, Discovery, Inputs, Layer, LoadedConfig, NoPresets, PackFile, Presets,
    Setting, Source, load,
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
        other @ ConfigError::Io { .. } => panic!("expected a rejection, got {other}"),
    }
}

fn src(layer: Layer, file: &str, line: usize, column: usize) -> Source {
    Source {
        layer,
        file: Some(file.into()),
        line: Some(line),
        column: Some(column),
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
        source: src(Layer::FrontMatter, "docs/a.md", 3, 3),
    }];
    let cli = [Setting::bare(Layer::Cli, &["locale"], json!("en-GB"))];

    let at = |rel: &str, i: &Inputs<'_>| cfg.resolve(Path::new(rel), i);

    let plain = at("README.md", &inputs(&NoPresets));
    assert_eq!(plain.get(&["locale"]), Some(&json!("en-GB")));
    assert_eq!(
        plain.source(&["locale"]),
        Some(&src(Layer::Project, "slopvac.toml", 2, 1))
    );

    let scoped = at("docs/a.md", &inputs(&NoPresets));
    assert_eq!(scoped.get(&["locale"]), Some(&json!("en-US")));
    assert_eq!(
        scoped.source(&["locale"]),
        Some(&src(Layer::Override, "slopvac.toml", 6, 1))
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
        Some(&Source::bare(Layer::Cli))
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
        Some(&Source::bare(Layer::Core))
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
    assert_eq!(history[0].source.line, Some(5));
    assert_eq!(history[1].source.line, Some(9));
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
        Some(&src(Layer::Extends, "../shared/b.toml", 3, 1))
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
        Layer::Cli,
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
        Some(&src(Layer::Project, "slopvac.toml", 4, 1))
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
        Layer::Env,
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
        Some(&Source::bare(Layer::Env))
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
    assert_eq!(locale.source, src(Layer::Override, "slopvac.toml", 6, 1));
    assert_eq!(locale.overridden.len(), 1);
    assert_eq!(locale.overridden[0].value, json!("en-GB"));
    assert_eq!(
        locale.overridden[0].source,
        src(Layer::Project, "slopvac.toml", 2, 1)
    );
}

struct Packs;

impl Presets for Packs {
    fn style(&self, name: &str) -> Option<Vec<Setting>> {
        (name == "acme:house").then(|| {
            vec![
                Setting::bare(Layer::Style, &["locale"], json!("en-US")),
                Setting::bare(Layer::Style, &["rules", "core:one"], json!("suggestion")),
            ]
        })
    }

    fn genre_chain(&self, name: &str) -> Vec<Vec<Setting>> {
        if name == "reference" {
            vec![
                vec![Setting::bare(Layer::Genre, &["locale"], json!("en-GB"))],
                vec![Setting::bare(
                    Layer::Genre,
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
            display: "pack:acme/base.toml".into(),
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
        Some(Layer::Style)
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
        source: Source::bare(Layer::FrontMatter),
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
        Some("pack:acme/base.toml".to_string())
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
    let cli = [Setting::bare(Layer::Cli, &["genre"], json!("reference"))];
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
    assert_eq!(history[1].source.layer, Layer::Override);
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
    Setting::bare(Layer::Style, path, json!(level))
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
        Some(&src(Layer::Extends, "../shared/base.toml", 2, 1))
    );
}
