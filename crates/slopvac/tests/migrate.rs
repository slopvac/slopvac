//! Installed-binary tests for `slopvac migrate`.
#![allow(clippy::expect_used, reason = "test code: a panic is a failed test")]

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

fn slopvac() -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    // Tests stay off the network: the update notice may refresh its cache after lint, doctor and prime.
    cmd.env("SLOPVAC_NO_UPDATE_CHECK", "1");
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "NO_COLOR",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

fn project(config: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join(".slopvac.toml"), config).expect("write");
    dir
}

#[test]
fn preview_prints_the_diff_and_writes_nothing() {
    let dir = project("profile = \"strict\"\n");
    slopvac()
        .arg("migrate")
        .arg(dir.path())
        .assert()
        .code(0)
        .stdout(predicate::str::contains("+++ b/slopvac.toml"))
        .stdout(predicate::str::contains("Preview only"));
    assert!(dir.path().join(".slopvac.toml").exists());
    assert!(!dir.path().join("slopvac.toml").exists());
}

#[test]
fn write_migrates_and_restore_undoes_it() {
    let dir = project("profile = \"strict\"\n");
    let cache = tempfile::tempdir().expect("tempdir");
    slopvac()
        .env("SLOPVAC_CACHE_DIR", cache.path())
        .arg("migrate")
        .arg(dir.path())
        .arg("--write")
        .assert()
        .code(0);
    assert_eq!(
        fs::read_to_string(dir.path().join("slopvac.toml")).expect("migrated"),
        "profile = \"strict\"\nschema_version = 3\n"
    );
    assert!(!dir.path().join(".slopvac.toml").exists());
    slopvac()
        .env("SLOPVAC_CACHE_DIR", cache.path())
        .arg("migrate")
        .arg(dir.path())
        .arg("--restore")
        .assert()
        .code(0);
    assert_eq!(
        fs::read_to_string(dir.path().join(".slopvac.toml")).expect("restored"),
        "profile = \"strict\"\n"
    );
    assert!(!dir.path().join("slopvac.toml").exists());
}

#[test]
fn preview_reports_the_findings_the_engine_counts_on_each_side() {
    let dir = project("profile = \"normal\"\n");
    fs::write(
        dir.path().join("doc.md"),
        "<!-- slopvac-allow: rule=orwell.stale-figure reason=dead-metaphor -->\nPick the low-hanging fruit.\n",
    )
    .expect("write");
    // Before, the directive names a 2.x id: the engine reports it and the stale figure.
    // After, `core:orwell.stale-figure` with its listed `dead-metaphor` exception holds.
    slopvac()
        .arg("migrate")
        .arg(dir.path())
        .assert()
        .code(0)
        .stdout(predicate::str::contains(
            "Findings by rule (before -> after):\n  core:meta.invalid-suppression: 1 -> 0\n  core:orwell.stale-figure: 1 -> 0\n",
        ));
}

#[test]
fn a_document_the_engine_cannot_lint_exits_2_and_writes_nothing() {
    let dir = project("profile = \"strict\"\n");
    fs::write(dir.path().join("doc.mdx"), "Unclosed {expression\n").expect("write");
    slopvac()
        .arg("migrate")
        .arg(dir.path())
        .arg("--write")
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("error[migrate-lint-failed]: "))
        .stderr(predicate::str::contains("doc.mdx"));
    assert!(dir.path().join(".slopvac.toml").exists());
    assert!(!dir.path().join("slopvac.toml").exists());
    assert!(!dir.path().join(".slopvac").exists());
}

#[test]
fn unrepresentable_setting_exits_2_and_writes_nothing() {
    let dir = project("[vale]\nenabled = true\n");
    slopvac()
        .arg("migrate")
        .arg(dir.path())
        .arg("--write")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "error[config-legacy-dropped]: .slopvac.toml:2: `vale.enabled`",
        ));
    assert!(dir.path().join(".slopvac.toml").exists());
    assert!(!dir.path().join("slopvac.toml").exists());
}

#[test]
fn restore_rejects_a_manifest_that_escapes_the_project() {
    let dir = tempfile::tempdir().expect("tempdir");
    let backup = dir.path().join(".slopvac/migrate-backup/20261120T064000Z");
    fs::create_dir_all(&backup).expect("mkdir");
    fs::write(backup.join("manifest.txt"), "C ../x\n").expect("write");
    slopvac()
        .arg("migrate")
        .arg(dir.path())
        .arg("--restore")
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("error[migrate-unsafe-path]: "));
}

#[test]
fn missing_project_reports_a_coded_error_without_the_absolute_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = dir.path().join("no-such-project");
    slopvac()
        .arg("migrate")
        .arg(&missing)
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("error[migrate-io-failed]: "))
        .stderr(predicate::str::contains(dir.path().to_str().expect("utf-8")).not());
}

#[test]
fn conflicting_config_sources_report_a_coded_error() {
    let dir = project("profile = \"strict\"\n");
    fs::write(dir.path().join("slopvac.toml"), "profile = \"normal\"\n").expect("write");
    slopvac()
        .arg("migrate")
        .arg(dir.path())
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with(
            "error[migrate-config-conflict]: ",
        ))
        .stderr(predicate::str::contains(dir.path().to_str().expect("utf-8")).not());
}

#[test]
fn write_rewrites_a_2x_steering_block_to_the_block_setup_writes() {
    use slopvac_agents::content::BlockKind;
    let text = "# Rules\n\nBe kind.\n\n<!-- slopvac:begin -->\n## Slopvac\n\nOld 2.x text.\n<!-- slopvac:end -->\n\nKeep this.\n";
    let migrated = tempfile::tempdir().expect("tempdir");
    let reference = tempfile::tempdir().expect("tempdir");
    for dir in [&migrated, &reference] {
        fs::write(dir.path().join("AGENTS.md"), text).expect("write");
    }
    let cache = tempfile::tempdir().expect("tempdir");
    slopvac()
        .env("SLOPVAC_CACHE_DIR", cache.path())
        .arg("migrate")
        .arg(migrated.path())
        .arg("--write")
        .assert()
        .code(0);
    slopvac_agents::setup::execute(&slopvac_agents::setup::Options {
        harness: Some("agents".into()),
        list: false,
        remove: false,
        check: false,
        root: reference.path().to_owned(),
        block: BlockKind::Standard,
        no_skill: true,
        agent: false,
        reviewer_model: None,
    })
    .expect("setup");
    assert_eq!(
        fs::read(migrated.path().join("AGENTS.md")).expect("read"),
        fs::read(reference.path().join("AGENTS.md")).expect("read"),
    );
}

#[test]
fn a_migrated_project_loads_its_blocklist() {
    let dir = project("[vocabulary]\npath = \"words.toml\"\n");
    fs::write(
        dir.path().join("words.toml"),
        "[[entries]]\nword = \"synergize\"\npos = \"Verb\"\nreason = \"Business jargon.\"\n",
    )
    .expect("blocklist");
    let doc = dir.path().join("doc.md");
    fs::write(&doc, "# Title\n\nManagers synergize the teams on Friday.\n").expect("doc");
    let cache = tempfile::tempdir().expect("cache");
    slopvac()
        .env("SLOPVAC_CACHE_DIR", cache.path())
        .arg("migrate")
        .arg(dir.path())
        .arg("--write")
        .assert()
        .code(0);
    assert_eq!(
        fs::read_to_string(dir.path().join("words.toml")).expect("blocklist"),
        "schema_version = 3\n[[entries]]\nword = \"synergize\"\npos = \"verb\"\nreason = \"Business jargon.\"\n"
    );
    slopvac()
        .env("NO_COLOR", "1")
        .args(["lint", "--format", "json"])
        .arg(&doc)
        .assert()
        .stdout(predicate::str::contains("vocabulary"));
}
#[test]
fn a_blocklist_path_with_a_symlink_before_a_parent_step_is_refused() {
    // `a/link/../words.toml` is `a/words.toml` when read as text. The OS follows the link
    // first, so the project loads `other/words.toml`. Migrate cannot tell, so it refuses.
    let dir = project("[vocabulary]\npath = \"a/link/../words.toml\"\n");
    let old = "[[entries]]\nword = \"synergize\"\npos = \"verb\"\nreason = \"Jargon.\"\n";
    for name in ["a/words.toml", "other/words.toml"] {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        fs::write(path, old).expect("blocklist");
    }
    fs::create_dir_all(dir.path().join("other/deep")).expect("dirs");
    std::os::unix::fs::symlink("../other/deep", dir.path().join("a/link")).expect("symlink");
    slopvac()
        .arg("migrate")
        .arg(dir.path())
        .arg("--write")
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("error[migrate-unsafe-path]: "))
        .stderr(predicate::str::contains("crosses a symbolic link"));
    for name in ["a/words.toml", "other/words.toml"] {
        assert_eq!(
            fs::read_to_string(dir.path().join(name)).expect("blocklist"),
            old
        );
    }
    assert!(dir.path().join(".slopvac.toml").exists());
    assert!(!dir.path().join("slopvac.toml").exists());
    assert!(!dir.path().join(".slopvac").exists());
}

#[test]
fn an_absolute_blocklist_path_inside_the_project_migrates_from_the_default_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().expect("canonical");
    let words = root.join("words.toml");
    fs::write(
        root.join("slopvac.toml"),
        format!("[vocabulary]\npath = \"{}\"\n", words.display()),
    )
    .expect("config");
    fs::write(
        &words,
        "[[entries]]\nword = \"synergize\"\npos = \"Verb\"\nreason = \"Business jargon.\"\n",
    )
    .expect("blocklist");
    // No path argument: the project root is the default `.`.
    let cache = tempfile::tempdir().expect("cache");
    slopvac()
        .env("SLOPVAC_CACHE_DIR", cache.path())
        .current_dir(&root)
        .args(["migrate", "--write"])
        .assert()
        .code(0);
    assert_eq!(
        fs::read_to_string(&words).expect("blocklist"),
        "schema_version = 3\n[[entries]]\nword = \"synergize\"\npos = \"verb\"\nreason = \"Business jargon.\"\n"
    );
}

/// A 2.x config that names a removed category and a removed rule, at the top level.
const REMOVED_CONFIG: &str = "profile = \"normal\"\n[categories]\norwell-voice = \"off\"\n[rules]\n\"ste-words.inconsistent-term-for-same-thing\" = \"error\"\n";

#[test]
fn removed_names_print_their_codes_and_the_pointer_and_exit_0() {
    let dir = project(REMOVED_CONFIG);
    slopvac()
        .arg("migrate")
        .arg(dir.path())
        .assert()
        .code(0)
        .stdout(predicate::str::contains(
            "migrate-category-removed: .slopvac.toml:3: `categories.orwell-voice`: category no longer exists in 3.0; see removed-rules.toml at https://github.com/slopvac/slopvac/blob/main/docs/contract/compat/removed-rules.toml",
        ))
        .stdout(predicate::str::contains(
            "migrate-rule-removed: .slopvac.toml:5: `rules.ste-words.inconsistent-term-for-same-thing`: rule no longer exists in 3.0; see removed-rules.toml at https://github.com/slopvac/slopvac/blob/main/docs/contract/compat/removed-rules.toml",
        ));
}

#[test]
fn a_config_with_removed_names_loads_after_migration() {
    let dir = project(REMOVED_CONFIG);
    fs::write(dir.path().join("doc.md"), "Plain text.\n").expect("write");
    let cache = tempfile::tempdir().expect("tempdir");
    slopvac()
        .env("SLOPVAC_CACHE_DIR", cache.path())
        .arg("migrate")
        .arg(dir.path())
        .arg("--write")
        .assert()
        .code(0);
    slopvac()
        .current_dir(dir.path())
        .args(["lint", "doc.md"])
        .assert()
        .code(0);
}
