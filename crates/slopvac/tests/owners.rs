//! Installed-binary smoke tests for the commands whose logic lives in owner crates:
//! `terms`, `genre`, `validate`, `setup` and `prime`. Each gets its help and one real
//! behaviour, plus path redaction where the command prints a user path.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;

fn slopvac(dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd.env_remove(var);
    }
    cmd.env("NO_COLOR", "1").current_dir(dir);
    cmd
}

#[test]
fn every_wired_command_has_help() {
    let dir = tempfile::tempdir().unwrap();
    for (name, about) in [
        ("terms", "Manage project terms"),
        ("genre", "Print the genre of each file"),
        (
            "validate",
            "Check config, front matter and inline directives",
        ),
        ("setup", "Install or remove project-local steering"),
        ("prime", "Print guidance for agents"),
    ] {
        slopvac(dir.path())
            .args([name, "--help"])
            .assert()
            .code(0)
            .stdout(predicate::str::contains(about));
    }
}

#[test]
fn terms_lists_the_built_in_terms() {
    let dir = tempfile::tempdir().unwrap();
    slopvac(dir.path())
        .args(["terms", "list", "--builtin"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("Python\tlanguage\tbuiltin\n"));
}

#[test]
fn terms_add_writes_the_project_terms_file() {
    let dir = tempfile::tempdir().unwrap();
    slopvac(dir.path())
        .args(["terms", "add", "Foo"])
        .assert()
        .code(0)
        .stdout(predicate::str::starts_with("added Foo"));
    assert!(dir.path().join(".slopvac/terms.toml").is_file());
}

#[test]
fn terms_error_names_an_external_file_without_its_directories() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::Builder::new()
        .prefix("SENTINEL ext")
        .tempdir()
        .unwrap();
    let file = outside.path().join("t.toml");
    fs::write(&file, "not = [valid").unwrap();
    let out = slopvac(dir.path())
        .args(["terms", "list", "--files"])
        .arg(&file)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        err.starts_with("error[terms-parse]: cannot parse <external>/t.toml"),
        "{err}"
    );
    assert!(!err.contains("SENTINEL"), "{err}");
}

#[test]
fn genre_prints_the_genre_and_its_scores() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.md"), "# Title\n\nWord.\n").unwrap();
    slopvac(dir.path())
        .args(["genre", "a.md"])
        .assert()
        .code(0)
        .stdout(predicate::str::starts_with("a.md: consumer (default)\n"));
}

#[test]
fn genre_assign_makes_the_choice_explicit() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.md"), "# Title\n").unwrap();
    slopvac(dir.path())
        .args(["genre", "a.md", "--assign", "*.md=tutorial"])
        .assert()
        .code(0)
        .stdout(predicate::str::starts_with("a.md: tutorial (explicit)\n"));
}

#[test]
fn validate_passes_a_good_config_and_names_the_count() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("slopvac.toml"), "schema_version = 3\n").unwrap();
    slopvac(dir.path())
        .arg("validate")
        .assert()
        .code(0)
        .stdout("checked 1 file(s): 0 problem(s)\n");
}

#[test]
fn validate_exits_2_with_a_stable_code_for_a_bad_config() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("slopvac.toml"),
        "schema_version = 3\nlocale = \"klingon\"\n",
    )
    .unwrap();
    slopvac(dir.path())
        .arg("validate")
        .assert()
        .code(2)
        .stdout(predicate::str::starts_with("error[config-invalid-value]: "))
        .stdout(predicate::str::contains("1 problem(s)"));
}

#[test]
fn validate_prints_a_schema_for_editors() {
    let dir = tempfile::tempdir().unwrap();
    let out = slopvac(dir.path())
        .args(["validate", "--schema", "config"])
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(
        value["$id"]
            .as_str()
            .unwrap()
            .ends_with("config.schema.json")
    );
}

#[test]
fn setup_list_names_every_harness() {
    let dir = tempfile::tempdir().unwrap();
    slopvac(dir.path())
        .args(["setup", "--list"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("agents ").and(predicate::str::contains("claude ")));
}

#[test]
fn setup_writes_steering_and_check_then_passes() {
    let dir = tempfile::tempdir().unwrap();
    slopvac(dir.path())
        .args(["setup", "agents"])
        .assert()
        .code(0);
    assert!(dir.path().join("AGENTS.md").is_file());
    slopvac(dir.path())
        .args(["setup", "agents", "--check"])
        .assert()
        .code(0);
}

#[test]
fn setup_check_exits_1_when_steering_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    slopvac(dir.path())
        .args(["setup", "agents", "--check"])
        .assert()
        .code(1);
}

#[test]
fn setup_under_an_external_root_prints_no_absolute_path() {
    let work = tempfile::tempdir().unwrap();
    let outside = tempfile::Builder::new()
        .prefix("SENTINEL ext")
        .tempdir()
        .unwrap();
    let out = slopvac(work.path())
        .args(["setup", "agents", "--root"])
        .arg(outside.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(text.contains("<external>/AGENTS.md"), "{text}");
    assert!(!text.contains("SENTINEL"), "{text}");
}

#[test]
fn prime_prints_the_base_guide_and_topics() {
    let dir = tempfile::tempdir().unwrap();
    slopvac(dir.path())
        .arg("prime")
        .assert()
        .code(0)
        .stdout(predicate::str::starts_with("# Slopvac agent guidance"));
    slopvac(dir.path())
        .args(["prime", "write"])
        .assert()
        .code(0)
        .stdout(predicate::str::starts_with("# Prime: write"));
}

#[test]
fn prime_refuses_an_unknown_topic_as_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    slopvac(dir.path())
        .args(["prime", "nonsense"])
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("error[usage]: "));
}

/// A directory whose absolute path holds a space and a sentinel.
fn external() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("SENTINEL ext")
        .tempdir()
        .unwrap()
}

fn assert_clean(text: &str, outside: &tempfile::TempDir) {
    for leaked in ["SENTINEL", &outside.path().display().to_string()] {
        assert!(!text.contains(leaked), "{leaked} leaked: {text}");
    }
}

#[test]
fn setup_error_under_an_external_root_names_the_file_without_its_directories() {
    let work = tempfile::tempdir().unwrap();
    let outside = external();
    // A lone begin marker makes the steering file malformed.
    fs::write(outside.path().join("AGENTS.md"), "<!-- slopvac:begin -->\n").unwrap();
    let out = slopvac(work.path())
        .args(["setup", "agents", "--root"])
        .arg(outside.path())
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        err.starts_with("error[steering-malformed]: malformed or ambiguous slopvac markers in <external>/AGENTS.md;"),
        "{err}"
    );
    assert_clean(&err, &outside);
}

#[test]
fn genre_error_for_an_unreadable_external_file_hides_its_directories() {
    let work = tempfile::tempdir().unwrap();
    let outside = external();
    let missing = outside.path().join("gone.md");
    let out = slopvac(work.path())
        .arg("genre")
        .arg(&missing)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        err.starts_with("error[genre-failed]: <external>/gone.md: "),
        "{err}"
    );
    assert_clean(&err, &outside);
}

#[test]
fn validate_names_a_missing_external_document_without_its_directories() {
    let work = tempfile::tempdir().unwrap();
    fs::write(work.path().join("slopvac.toml"), "schema_version = 3\n").unwrap();
    let outside = external();
    let out = slopvac(work.path())
        .arg("validate")
        .arg(outside.path().join("gone.md"))
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        text.contains("error[config-io]: cannot read <external>/gone.md: no such file"),
        "{text}"
    );
    assert_clean(&text, &outside);
}

#[test]
fn validate_names_an_external_directory_entry_without_its_directories() {
    let work = tempfile::tempdir().unwrap();
    fs::write(work.path().join("slopvac.toml"), "schema_version = 3\n").unwrap();
    let outside = external();
    fs::write(
        outside.path().join("doc.md"),
        "---\nslopvac:\n  locale: klingon\n---\n\nText.\n",
    )
    .unwrap();
    let out = slopvac(work.path())
        .arg("validate")
        .arg(outside.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains(" --> <external>/doc.md:3:3"), "{text}");
    assert_clean(&text, &outside);
}

#[test]
fn prime_with_a_missing_absolute_config_hides_its_directories_and_prefix_repeat() {
    let work = tempfile::tempdir().unwrap();
    let outside = external();
    let out = slopvac(work.path())
        .args(["prime", "--config"])
        .arg(outside.path().join("nope.toml"))
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        err.starts_with("error[config-io]: cannot read <external>/nope.toml: no such file"),
        "{err}"
    );
    assert_clean(&err, &outside);
}

#[test]
fn prime_with_a_missing_config_under_the_working_directory_shows_it_relative() {
    let work = tempfile::Builder::new()
        .prefix("SENTINEL cwd")
        .tempdir()
        .unwrap();
    let out = slopvac(work.path())
        .args(["prime", "--config"])
        .arg(
            work.path()
                .canonicalize()
                .unwrap()
                .join("sub dir")
                .join("nope.toml"),
        )
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        err.contains("cannot read sub dir/nope.toml: no such file"),
        "{err}"
    );
    assert_clean(&err, &work);
}

#[test]
fn setup_with_a_nonexistent_absolute_root_hides_its_directories() {
    let work = tempfile::Builder::new()
        .prefix("SENTINEL cwd")
        .tempdir()
        .unwrap();
    // The working directory as the process reports it, so the root sits under it.
    let root = work
        .path()
        .canonicalize()
        .unwrap()
        .join("no dir")
        .join("deeper");
    let out = slopvac(work.path())
        .args(["setup", "agents", "--root"])
        .arg(&root)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2));
    assert!(err.contains("no dir/deeper"), "{err}");
    assert_clean(&err, &work);
}

#[test]
fn prime_with_a_bad_guidance_level_never_prints_url_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("slopvac.toml");
    fs::write(
        &config,
        "schema_version = 3\n\n[guidance]\nlevel = \"https://user:secret@example.com\"\n",
    )
    .unwrap();
    let out = slopvac(dir.path())
        .args(["prime", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2));
    assert!(err.starts_with("error[config-invalid-value]: "), "{err}");
    assert!(!err.contains("secret"), "credentials leaked: {err}");
    assert!(err.contains("https://example.com"), "the host stays: {err}");
}

#[test]
fn a_quote_inside_an_unquoted_path_does_not_end_it() {
    let work = tempfile::tempdir().unwrap();
    let outside = external();
    let missing = outside
        .path()
        .join("a\"b")
        .join("private dir")
        .join("nope.toml");
    let out = slopvac(work.path())
        .args(["prime", "--config"])
        .arg(&missing)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        err.starts_with("error[config-io]: cannot read <external>/nope.toml: no such file"),
        "{err}"
    );
    for leaked in ["private", "a\"b", "SENTINEL"] {
        assert!(!err.contains(leaked), "{leaked} leaked: {err}");
    }
}

#[test]
fn a_config_path_that_contains_a_colon_and_space_is_redacted_whole() {
    let work = tempfile::tempdir().unwrap();
    let outside = external();
    let missing = outside
        .path()
        .join("a: b")
        .join("private dir")
        .join("nope.toml");
    let out = slopvac(work.path())
        .args(["prime", "--config"])
        .arg(&missing)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        err.starts_with("error[config-io]: cannot read <external>/nope.toml: no such file"),
        "{err}"
    );
    for leaked in ["private", "a: b", "SENTINEL"] {
        assert!(!err.contains(leaked), "{leaked} leaked: {err}");
    }
}
