//! `[guidance]` read from real config files by `prime` and `setup`.

use std::fs;
use std::path::{Path, PathBuf};

use slopvac_agents::AgentsError;
use slopvac_agents::guidance::Guidance;
use slopvac_agents::harness::SKILL_NAME;
use slopvac_agents::prime::{self, Level};
use slopvac_agents::setup;

fn write_config(dir: &Path, name: &str, guidance: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(
        &path,
        format!("schema_version = 3\n\n[guidance]\n{guidance}\n"),
    )
    .unwrap();
    path
}

fn prime_out(root: &Path, args: &[&str]) -> Result<String, AgentsError> {
    let words = std::iter::once("prime").chain(args.iter().copied());
    let m = prime::command().try_get_matches_from(words).unwrap();
    prime::run(&m, &clap::Command::new("slopvac"), root)
}

#[test]
fn no_config_gives_the_schema_defaults() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    let g = Guidance::load(dir.path(), None).unwrap();
    assert_eq!(g.level, Level::Default);
    assert!(g.write);
}

#[test]
fn nearest_config_sets_the_level() {
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), "slopvac.toml", "level = \"brief\"");
    assert_eq!(
        Guidance::load(dir.path(), None).unwrap().level,
        Level::Minimal
    );
}

#[test]
fn explicit_config_path_beats_the_nearest_config() {
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), "slopvac.toml", "level = \"brief\"");
    let other = write_config(dir.path(), "other.toml", "level = \"full\"");
    assert_eq!(
        Guidance::load(dir.path(), Some(&other)).unwrap().level,
        Level::Full
    );
}

#[test]
fn level_flag_overrides_the_config() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = write_config(dir.path(), "slopvac.toml", "level = \"full\"");
    let cfg = cfg.to_str().unwrap();
    let from_config = prime_out(dir.path(), &["--config", cfg]).unwrap();
    let flagged = prime_out(dir.path(), &["--config", cfg, "--level", "minimal"]).unwrap();
    let plain_minimal = prime_out(dir.path(), &["--level", "minimal"]).unwrap();
    assert_ne!(from_config, flagged);
    assert_eq!(flagged, plain_minimal);
    assert_eq!(
        from_config,
        prime_out(dir.path(), &["--level", "full"]).unwrap()
    );
}

#[test]
fn invalid_level_is_a_config_error_with_the_config_code() {
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), "slopvac.toml", "level = \"loud\"");
    let err = prime_out(dir.path(), &[]).unwrap_err();
    assert_eq!(err.code(), "config-invalid-value");
}

#[test]
fn missing_explicit_config_is_a_config_io_error() {
    let dir = tempfile::tempdir().unwrap();
    let err = prime_out(dir.path(), &["--config", "nope/slopvac.toml"]).unwrap_err();
    assert_eq!(err.code(), "config-io");
}

#[test]
fn level_flag_skips_loading_the_config() {
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), "slopvac.toml", "level = \"loud\"");
    assert!(prime_out(dir.path(), &["--level", "full"]).is_ok());
}

fn run_setup(root: &Path, extra: &[&str]) -> Result<setup::Outcome, AgentsError> {
    let root = root.to_str().unwrap();
    let words = ["setup", "claude", "--root", root]
        .into_iter()
        .chain(extra.iter().copied());
    setup::run(&setup::command().try_get_matches_from(words).unwrap())
}

fn skill_installed(root: &Path) -> bool {
    root.join(".claude/skills").join(SKILL_NAME).exists()
}

#[test]
fn skill_follows_write_and_review() {
    for (write, review, wanted) in [
        ("off", "off", false),
        ("off", "auto", true),
        ("auto", "off", true),
        ("auto", "auto", true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        write_config(
            dir.path(),
            "slopvac.toml",
            &format!("write = \"{write}\"\nreview = \"{review}\""),
        );
        run_setup(dir.path(), &[]).unwrap();
        assert!(dir.path().join("CLAUDE.md").exists());
        assert_eq!(skill_installed(dir.path()), wanted, "{write}/{review}");
    }
}

#[test]
fn no_skill_flag_wins_over_enabled_guidance() {
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), "slopvac.toml", "write = \"auto\"");
    run_setup(dir.path(), &["--no-skill"]).unwrap();
    assert!(!skill_installed(dir.path()));
}

#[test]
fn list_and_remove_work_with_a_broken_config() {
    let dir = tempfile::tempdir().unwrap();
    run_setup(dir.path(), &[]).unwrap();
    fs::write(dir.path().join("slopvac.toml"), "mode = \"2.x\"\n").unwrap();
    assert!(run_setup(dir.path(), &["--check"]).is_err());
    let list = setup::command()
        .try_get_matches_from(["setup", "--list", "--root", dir.path().to_str().unwrap()])
        .unwrap();
    assert!(setup::run(&list).is_ok());
    run_setup(dir.path(), &["--remove"]).unwrap();
    let skill = dir.path().join(".claude/skills").join(SKILL_NAME);
    assert!(!skill.exists());
}
