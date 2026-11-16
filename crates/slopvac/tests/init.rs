//! Installed-binary tests for `slopvac init` (compat rows `cmd:init`,
//! `opt:init:--force`, `--path`, `--skip-agents`, `--harness`, `--no-skills`,
//! `--profile` (removed), `behavior:init-flags`, `behavior:init-idempotence`).
//!
//! The output is validated against the published config schema.
//! STUB(slopvac-8l4.12.25): swap to `slopvac validate` when 8l4.4.6 lands.
#![cfg(test)]

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

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

fn schemas() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3")
}

fn validator() -> jsonschema::Validator {
    let read = |name: &str| -> Value {
        serde_json::from_str(&fs::read_to_string(schemas().join(name)).expect("schema"))
            .expect("schema json")
    };
    let registry = jsonschema::Registry::new()
        .add(
            "https://srobroek.github.io/slopvac/schemas/v3/common.schema.json",
            read("common.schema.json"),
        )
        .expect("register common")
        .prepare()
        .expect("prepare registry");
    jsonschema::options()
        .with_registry(&registry)
        .build(&read("config.schema.json"))
        .expect("schema compiles")
}

#[test]
fn init_writes_a_config_that_validates_against_the_schema() {
    let dir = tempfile::tempdir().unwrap();
    slopvac(dir.path())
        .args(["init", "--skip-agents"])
        .assert()
        .code(0)
        .stdout("wrote slopvac.toml\nnext: slopvac prime\n");
    let text = fs::read_to_string(dir.path().join("slopvac.toml")).unwrap();
    let value: toml::Value = toml::from_str(&text).expect("the template is TOML");
    let json = serde_json::to_value(&value).unwrap();
    let problems: Vec<String> = validator()
        .iter_errors(&json)
        .map(|e| e.to_string())
        .collect();
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(json["schema_version"], 3);
    assert_eq!(json["profile"], "normal");
}

#[test]
fn the_written_config_is_commented_so_a_reader_sees_every_setting() {
    let dir = tempfile::tempdir().unwrap();
    slopvac(dir.path())
        .args(["init", "--skip-agents"])
        .assert()
        .code(0);
    let text = fs::read_to_string(dir.path().join("slopvac.toml")).unwrap();
    for section in [
        "# [thresholds]",
        "# [guidance]",
        "# [[packs]]",
        "# [[overrides]]",
    ] {
        assert!(text.contains(section), "missing {section}");
    }
    assert!(text.lines().all(|l| l.len() <= 80), "a line is too long");
}

#[test]
fn a_second_init_keeps_the_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("slopvac.toml");
    fs::write(&file, "schema_version = 3\n# mine\n").unwrap();
    slopvac(dir.path())
        .args(["init", "--skip-agents"])
        .assert()
        .code(0)
        .stdout("slopvac.toml exists; keeping it.\nnext: slopvac prime\n");
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "schema_version = 3\n# mine\n"
    );
}

#[test]
fn force_overwrites_the_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("slopvac.toml");
    fs::write(&file, "old\n").unwrap();
    slopvac(dir.path())
        .args(["init", "--skip-agents", "--force"])
        .assert()
        .code(0)
        .stdout(predicate::str::starts_with("wrote slopvac.toml\n"));
    assert!(
        fs::read_to_string(&file)
            .unwrap()
            .starts_with("# slopvac configuration.")
    );
}

#[test]
fn path_chooses_the_file_and_creates_its_directories() {
    let dir = tempfile::tempdir().unwrap();
    slopvac(dir.path())
        .args(["init", "--skip-agents", "--path", "conf/sub/slopvac.toml"])
        .assert()
        .code(0)
        .stdout(predicate::str::starts_with("wrote conf/sub/slopvac.toml\n"));
    assert!(dir.path().join("conf/sub/slopvac.toml").is_file());
}

#[test]
fn the_default_harness_writes_the_agents_md_steering_block() {
    let dir = tempfile::tempdir().unwrap();
    slopvac(dir.path())
        .arg("init")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("harnesses: agents\n"));
    let steering = fs::read_to_string(dir.path().join("AGENTS.md")).expect("AGENTS.md");
    assert!(steering.contains("slopvac"), "{steering}");
}

#[test]
fn a_named_harness_writes_its_own_steering_file_and_skill() {
    let dir = tempfile::tempdir().unwrap();
    slopvac(dir.path())
        .args(["init", "--harness", "claude"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("harnesses: claude\n"));
    assert!(dir.path().join("CLAUDE.md").is_file());
    assert!(dir.path().join(".claude/skills").is_dir());
}

#[test]
fn no_skills_writes_the_steering_but_no_skill() {
    let dir = tempfile::tempdir().unwrap();
    slopvac(dir.path())
        .args(["init", "--harness", "claude", "--no-skills"])
        .assert()
        .code(0);
    assert!(dir.path().join("CLAUDE.md").is_file());
    assert!(!dir.path().join(".claude/skills").exists());
}

#[test]
fn harness_may_repeat_and_a_repeat_is_set_up_once() {
    let dir = tempfile::tempdir().unwrap();
    slopvac(dir.path())
        .args([
            "init",
            "--harness",
            "claude",
            "--harness",
            "codex",
            "--harness",
            "claude",
        ])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("harnesses: claude, codex\n"));
    assert!(dir.path().join("CLAUDE.md").is_file());
    assert!(dir.path().join("AGENTS.md").is_file());
}

#[test]
fn skip_agents_writes_no_steering() {
    let dir = tempfile::tempdir().unwrap();
    slopvac(dir.path())
        .args(["init", "--skip-agents"])
        .assert()
        .code(0);
    assert!(!dir.path().join("AGENTS.md").exists());
}

#[test]
fn skip_agents_conflicts_with_harness_options() {
    let dir = tempfile::tempdir().unwrap();
    for extra in [&["--harness", "claude"][..], &["--no-skills"][..]] {
        slopvac(dir.path())
            .args(["init", "--skip-agents"])
            .args(extra)
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with(
                "error[usage]: --skip-agents writes no steering",
            ));
    }
    assert!(
        !dir.path().join("slopvac.toml").exists(),
        "nothing is written on a usage error"
    );
}

#[test]
fn profile_is_removed_with_a_pointer_to_migrate() {
    let dir = tempfile::tempdir().unwrap();
    slopvac(dir.path())
        .args(["init", "--profile", "strict"])
        .assert()
        .code(2)
        .stderr(
            predicate::str::starts_with("error[removed-option]: --profile is removed in 3.0.")
                .and(predicate::str::contains("slopvac migrate")),
        );
}

#[test]
fn an_unknown_harness_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    slopvac(dir.path())
        .args(["init", "--harness", "emacs"])
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("error[usage]: "));
}

#[test]
fn a_directory_as_path_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("conf")).unwrap();
    slopvac(dir.path())
        .args(["init", "--skip-agents", "--path", "conf"])
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with(
            "error[usage]: --path conf is a directory.",
        ));
}

#[test]
fn an_absolute_path_outside_the_working_directory_is_redacted() {
    let work = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let file = target.path().join("slopvac.toml");
    let out = slopvac(work.path())
        .args(["init", "--skip-agents", "--path"])
        .arg(&file)
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        stdout.starts_with("wrote <external>/slopvac.toml\n"),
        "{stdout}"
    );
    assert!(
        !stdout.contains(&target.path().display().to_string()),
        "{stdout}"
    );
    assert!(file.is_file());
}

#[test]
fn a_write_failure_is_an_incomplete_run_that_hides_the_path() {
    let work = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let blocker = target.path().join("blocker");
    fs::write(&blocker, "").unwrap();
    let out = slopvac(work.path())
        .args(["init", "--skip-agents", "--path"])
        .arg(blocker.join("slopvac.toml"))
        .output()
        .expect("run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr.starts_with("error[output-failed]: <external>/slopvac.toml: cannot write"),
        "{stderr}"
    );
    assert!(
        !stderr.contains(&target.path().display().to_string()),
        "{stderr}"
    );
}

#[cfg(unix)]
#[test]
fn a_failed_write_leaves_the_existing_config_untouched() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("slopvac.toml");
    fs::write(&file, "schema_version = 3\n# mine\n").unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
    // A read-only directory refuses the temporary sibling; skip when the user can write anyway.
    let probe = dir.path().join("probe");
    let writable = fs::write(&probe, "").is_ok();
    let result = slopvac(dir.path())
        .args(["init", "--skip-agents", "--force"])
        .output()
        .expect("run");
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
    if writable {
        return;
    }
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).starts_with("error[output-failed]: "));
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "schema_version = 3\n# mine\n"
    );
}

#[test]
fn init_leaves_no_temporary_files_next_to_the_config() {
    let dir = tempfile::tempdir().unwrap();
    slopvac(dir.path())
        .args(["init", "--skip-agents"])
        .assert()
        .code(0);
    let names: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["slopvac.toml"]);
}

#[test]
fn init_without_a_reviewer_keeps_an_installed_reviewer_and_skill() {
    use slopvac_agents::{content::BlockKind, harness, setup};
    let dir = tempfile::tempdir().unwrap();
    setup::execute(&setup::Options {
        harness: Some("claude".into()),
        list: false,
        remove: false,
        check: false,
        root: dir.path().into(),
        block: BlockKind::Standard,
        no_skill: false,
        agent: true,
        reviewer_model: None,
    })
    .unwrap();
    let h = harness::find("claude").unwrap();
    let skill = dir.path().join(h.skill_path().unwrap().unwrap());
    let reviewer = dir.path().join(h.agent_path().unwrap().unwrap());
    assert!(skill.is_file() && reviewer.is_file());
    slopvac(dir.path())
        .args(["init", "--harness", "claude", "--no-skills"])
        .assert()
        .code(0);
    assert!(reviewer.is_file(), "init removed the reviewer file");
    assert!(skill.is_file(), "init --no-skills removed the skill");
}

#[test]
fn nested_path_puts_steering_and_skills_beside_the_config() {
    let work = tempfile::tempdir().unwrap();
    slopvac(work.path())
        .args([
            "init",
            "--path",
            "conf/sub/slopvac.toml",
            "--harness",
            "claude",
        ])
        .assert()
        .code(0);
    let nested = work.path().join("conf/sub");
    assert!(nested.join("CLAUDE.md").is_file());
    assert!(nested.join(".claude/skills").is_dir());
    assert!(
        !work.path().join("CLAUDE.md").exists(),
        "steering landed in the cwd"
    );
    assert!(
        !work.path().join(".claude").exists(),
        "skills landed in the cwd"
    );
}

#[test]
fn external_path_with_agents_enabled_prints_no_absolute_path() {
    let work = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let out = slopvac(work.path())
        .args(["init", "--harness", "claude", "--path"])
        .arg(target.path().join("slopvac.toml"))
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(0));
    let all = [out.stdout, out.stderr].concat();
    let all = String::from_utf8_lossy(&all);
    assert!(all.contains("<external>/CLAUDE.md"), "{all}");
    assert!(!all.contains(&target.path().display().to_string()), "{all}");
    assert!(target.path().join("CLAUDE.md").is_file());
}

#[test]
fn a_setup_error_under_an_external_path_prints_no_absolute_path() {
    let work = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    // A lone begin marker makes the steering file malformed, so setup fails.
    fs::write(target.path().join("AGENTS.md"), "<!-- slopvac:begin -->\n").unwrap();
    let out = slopvac(work.path())
        .args(["init", "--path"])
        .arg(target.path().join("slopvac.toml"))
        .output()
        .expect("run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr.starts_with("error["), "{stderr}");
    assert!(
        !stderr.contains(&target.path().display().to_string()),
        "{stderr}"
    );
    assert!(stderr.contains("<external>"), "{stderr}");
}

/// Run `slopvac init --skip-agents [extra]` in `dir` under the given umask.
#[cfg(unix)]
fn init_with_umask(dir: &Path, umask: &str, extra: &[&str]) {
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("umask {umask}; exec \"$0\" \"$@\""))
        .arg(env!("CARGO_BIN_EXE_slopvac"))
        .args(["init", "--skip-agents"])
        .args(extra)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .status()
        .expect("run");
    assert!(status.success());
}

#[cfg(unix)]
fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[cfg(unix)]
#[test]
fn a_new_config_gets_the_mode_the_umask_allows() {
    for (umask, want) in [("022", 0o644), ("077", 0o600), ("002", 0o664)] {
        let dir = tempfile::tempdir().unwrap();
        init_with_umask(dir.path(), umask, &[]);
        assert_eq!(
            mode_of(&dir.path().join("slopvac.toml")),
            want,
            "umask {umask}"
        );
    }
}

#[cfg(unix)]
#[test]
fn force_keeps_the_mode_of_the_file_it_replaces() {
    use std::os::unix::fs::PermissionsExt;
    for mode in [0o600, 0o660, 0o640] {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("slopvac.toml");
        fs::write(&file, "old\n").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(mode)).unwrap();
        init_with_umask(dir.path(), "022", &["--force"]);
        assert!(fs::read_to_string(&file).unwrap().starts_with("# slopvac"));
        assert_eq!(mode_of(&file), mode, "was {mode:o}");
    }
}

#[test]
fn init_suggests_the_google_preset_for_a_vale_google_repo() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join(".vale.ini"),
        "[*.md]\nBasedOnStyles = Google\n",
    )
    .unwrap();
    slopvac(dir.path())
        .args(["init", "--skip-agents"])
        .assert()
        .code(0)
        .stdout(
            "wrote slopvac.toml\n\
             house style: Google developer documentation style guide: suggest the \"google\" preset (.vale.ini:2)\n\
             next: slopvac prime\n",
        );
}
