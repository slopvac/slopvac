//! Migrating a 2.x steering block writes what `slopvac setup` writes.

use std::fs;
use std::path::Path;

use slopvac_agents::content::{BlockKind, steering_block};
use slopvac_agents::setup;
use slopvac_migrate::{Options, RenameTable, apply, plan};

const V2_BLOCK: &str = "<!-- slopvac:begin -->\n## Slopvac\n\nRun `slopvac prime` before prose.\n<!-- slopvac:end -->\n";

fn migrate(root: &Path) {
    let block = steering_block(BlockKind::Standard);
    let p = plan(root, &RenameTable::bundled(), Options::default(), &block).unwrap();
    let cache = tempfile::tempdir().unwrap();
    apply(root, &p, cache.path()).unwrap();
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
