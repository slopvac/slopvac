//! Migrating a 2.x steering block writes what `slopvac setup` writes.

use std::fs;
use std::path::Path;

use slopvac_agents::block::{State, block_state};
use slopvac_agents::content::{BlockKind, steering_block};
use slopvac_agents::fsx::Project;
use slopvac_agents::setup;
use slopvac_migrate::{Options, RenameTable, apply, plan, rewrite_steering};

const V2_BLOCK: &str = "<!-- slopvac:begin -->\n## Slopvac\n\nRun `slopvac prime` before prose.\n<!-- slopvac:end -->\n";

fn migrate(root: &Path) {
    let block = steering_block(BlockKind::Standard);
    let p = plan(root, &RenameTable::bundled(), Options::default(), &block).unwrap();
    apply(root, &p).unwrap();
}

fn setup_opts(root: &Path, harness: &str) -> setup::Options {
    setup::Options {
        harness: Some(harness.into()),
        list: false,
        remove: false,
        check: false,
        root: root.to_owned(),
        block: BlockKind::Standard,
        no_skill: true,
        agent: false,
        reviewer_model: None,
    }
}

#[test]
fn a_2x_block_becomes_the_block_setup_writes() {
    for (file, harness) in [("AGENTS.md", "agents"), ("CLAUDE.md", "claude")] {
        let text = format!("# Rules\n\nBe kind.\n\n{V2_BLOCK}\n## After\n\nKeep this.\n");
        let migrated = tempfile::tempdir().unwrap();
        let reference = tempfile::tempdir().unwrap();
        fs::write(migrated.path().join(file), &text).unwrap();
        fs::write(reference.path().join(file), &text).unwrap();

        migrate(migrated.path());
        setup::execute(&setup_opts(reference.path(), harness)).unwrap();

        let got = fs::read(migrated.path().join(file)).unwrap();
        let want = fs::read(reference.path().join(file)).unwrap();
        assert_eq!(got, want, "{file}");
        let got = String::from_utf8(got).unwrap();
        assert!(got.starts_with("# Rules\n\nBe kind.\n\n"), "{got}");
        assert!(got.ends_with("\n\n## After\n\nKeep this.\n"), "{got}");
    }
}

#[test]
fn migrating_again_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("AGENTS.md"), format!("Intro\n\n{V2_BLOCK}")).unwrap();
    migrate(dir.path());
    let once = fs::read(dir.path().join("AGENTS.md")).unwrap();
    let block = steering_block(BlockKind::Standard);
    let again = plan(
        dir.path(),
        &RenameTable::bundled(),
        Options::default(),
        &block,
    )
    .unwrap();
    assert!(again.is_empty());
    assert_eq!(fs::read(dir.path().join("AGENTS.md")).unwrap(), once);
}

/// What migrate makes of `text`, in setup's vocabulary.
fn migrate_verdict(text: &str) -> State {
    match rewrite_steering(text, &steering_block(BlockKind::Standard)) {
        Ok(None) => State::Missing,
        Ok(Some(_)) => State::Stale,
        Err(_) => State::Malformed,
    }
}

/// What `slopvac setup --check` makes of `text`.
fn setup_verdict(text: &str) -> State {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("AGENTS.md"), text).unwrap();
    let project = Project::open(dir.path()).unwrap();
    match block_state(
        &project,
        Path::new("AGENTS.md"),
        &steering_block(BlockKind::Standard),
    ) {
        State::Current => State::Stale,
        other => other,
    }
}

#[test]
fn migrate_and_setup_agree_on_which_markers_count() {
    let cases = [
        ("prose", format!("Intro\n\n{V2_BLOCK}"), State::Stale),
        ("no markers", "Intro\n".to_owned(), State::Missing),
        (
            "fenced",
            "```\n<!-- slopvac:begin -->\nx\n<!-- slopvac:end -->\n```\n".to_owned(),
            State::Missing,
        ),
        (
            "indented",
            "Intro\n\n    <!-- slopvac:begin -->\n    x\n    <!-- slopvac:end -->\n".to_owned(),
            State::Missing,
        ),
        (
            "tab indented",
            "Intro\n\n\t<!-- slopvac:begin -->\n\t<!-- slopvac:end -->\n".to_owned(),
            State::Missing,
        ),
        (
            "prose begin, indented end",
            "<!-- slopvac:begin -->\n\n    <!-- slopvac:end -->\n".to_owned(),
            State::Malformed,
        ),
        (
            "block in prose, example in code",
            format!("{V2_BLOCK}\n    <!-- slopvac:begin -->\n"),
            State::Stale,
        ),
    ];
    for (name, text, want) in cases {
        assert_eq!(migrate_verdict(&text), want, "migrate: {name}");
        assert_eq!(setup_verdict(&text), want, "setup: {name}");
    }
}

#[test]
fn a_marker_shown_in_indented_code_is_left_alone() {
    let text = "Intro\n\n    <!-- slopvac:begin -->\n    old\n    <!-- slopvac:end -->\n";
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("AGENTS.md"), text).unwrap();
    migrate(dir.path());
    assert_eq!(
        fs::read_to_string(dir.path().join("AGENTS.md")).unwrap(),
        text
    );
}
