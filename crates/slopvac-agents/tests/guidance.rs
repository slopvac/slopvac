//! `[guidance]` read from real config files by `prime` and `setup`.

use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;
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
    assert_eq!(g.write, slopvac_agents::guidance::Write::Auto);
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

/// Prime output for a project whose `[guidance]` holds `guidance`.
fn prime_with(guidance: &str, args: &[&str]) -> String {
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), "slopvac.toml", guidance);
    prime_out(dir.path(), args).unwrap()
}

const WRITE_STEP: &str = "- `write`:";
const REVIEW_STEP: &str = "run `slopvac prime review`";
const REVIEW_INDEX: &str = "- `review`:";
const REPORT_STEP: &str = "- `report`:";

#[test]
fn write_auto_lists_the_write_topic_in_the_base_guide() {
    assert!(prime_with("write = \"auto\"", &[]).contains(WRITE_STEP));
}

#[test]
fn write_off_drops_the_write_topic_from_the_base_guide() {
    assert!(!prime_with("write = \"off\"", &[]).contains(WRITE_STEP));
}

#[test]
fn write_off_disables_the_write_topic_and_names_the_config_file() {
    assert_eq!(
        prime_with("write = \"off\"", &["write"]).trim_end(),
        "slopvac prime write is disabled by [guidance] write = \"off\" in slopvac.toml."
    );
}

#[test]
fn review_auto_makes_the_hand_off_step_mandatory() {
    let out = prime_with("review = \"auto\"", &[]);
    assert!(out.contains(REVIEW_INDEX) && !out.contains("Ask the user first"));
}

#[test]
fn review_ask_has_the_agent_ask_the_user_first() {
    let out = prime_with("review = \"ask\"", &[]);
    assert!(out.contains(REVIEW_INDEX) && out.contains("Ask the user first."));
}

#[test]
fn review_off_drops_the_hand_off_step_and_disables_the_topic() {
    assert!(!prime_with("review = \"off\"", &[]).contains(REVIEW_INDEX));
    assert_eq!(
        prime_with("review = \"off\"", &["review"]).trim_end(),
        "slopvac prime review is disabled by [guidance] review = \"off\" in slopvac.toml."
    );
}

#[test]
fn review_off_removes_the_hand_off_line_of_the_write_topic() {
    assert!(prime_with("review = \"auto\"", &["write"]).contains(REVIEW_STEP));
    assert!(!prime_with("review = \"off\"", &["write"]).contains(REVIEW_STEP));
}

#[rstest]
#[case::self_review("self", "review the change yourself")]
#[case::adversary("adversary", "Complete the adversary loop")]
fn review_depth_sets_the_depth_line_of_the_review_topic(
    #[case] depth: &str,
    #[case] expected: &str,
) {
    let out = prime_with(&format!("review_depth = \"{depth}\""), &["review"]);
    assert!(out.contains(expected), "{depth}: {out}");
}

#[rstest]
#[case::self_review("self", "Review it yourself, with no separate")]
#[case::subagent("subagent", "Reviewer pass: give one fresh-context reviewer")]
#[case::adversary("adversary", "Give each round")]
fn review_depth_sets_the_reviewer_pass_of_the_review_topic(
    #[case] depth: &str,
    #[case] expected: &str,
) {
    let out = prime_with(
        &format!("review_depth = \"{depth}\""),
        &["review", "--type", "update"],
    );
    assert!(out.contains(expected), "{depth}: {out}");
}

#[test]
fn report_ask_keeps_the_reporting_section_in_the_base_guide() {
    assert!(prime_with("report = \"ask\"", &[]).contains(REPORT_STEP));
}

#[test]
fn report_off_drops_the_section_and_disables_the_topic() {
    assert!(!prime_with("report = \"off\"", &[]).contains(REPORT_STEP));
    assert_eq!(
        prime_with("report = \"off\"", &["report"]).trim_end(),
        "slopvac prime report is disabled by [guidance] report = \"off\" in slopvac.toml."
    );
}

#[test]
fn the_notice_names_the_extended_file_that_set_the_control() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("base.toml"),
        "schema_version = 3\n\n[guidance]\nreport = \"off\"\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("slopvac.toml"),
        "schema_version = 3\nextends = [\"./base.toml\"]\n",
    )
    .unwrap();
    let out = prime_out(dir.path(), &["report"]).unwrap();
    assert!(out.contains("in base.toml."), "{out}");
}

#[test]
fn level_flag_still_fails_on_a_config_that_cannot_load() {
    // The Python reference loads the config for every `prime` run and fails on a broken one,
    // so a project cannot be told to run a step it switched off.
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), "slopvac.toml", "level = \"loud\"");
    let err = prime_out(dir.path(), &["--level", "full"]).unwrap_err();
    assert_eq!(err.code(), "config-invalid-value");
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
fn no_skill_flag_needs_no_valid_config() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("slopvac.toml"), "mode = \"2.x\"\n").unwrap();
    run_setup(dir.path(), &["--no-skill"]).unwrap();
    assert!(dir.path().join("CLAUDE.md").exists());
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
