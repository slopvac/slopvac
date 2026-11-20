//! No internal tracking ids in anything a user can read: help, errors and the generated
//! reference.

use assert_cmd::Command;
use std::path::Path;

fn has_internal_id(text: &str) -> bool {
    text.contains("slopvac-8l4") || text.contains("8l4.")
}

fn slopvac() -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    for var in ["SLOPVAC_LOG", "FORCE_COLOR", "CLICOLOR_FORCE"] {
        cmd.env_remove(var);
    }
    cmd.env("NO_COLOR", "1");
    cmd
}

/// Every command path in the tree, such as `["self", "update"]`.
fn command_paths(cmd: &clap::Command, prefix: &[String], out: &mut Vec<Vec<String>>) {
    for sub in cmd.get_subcommands() {
        let mut path = prefix.to_vec();
        path.push(sub.get_name().to_owned());
        out.push(path.clone());
        command_paths(sub, &path, out);
    }
}

#[test]
fn help_for_every_command_names_no_internal_id() {
    let tree = slopvac_cli::cli::command();
    let mut paths = vec![Vec::new()];
    command_paths(&tree, &[], &mut paths);
    assert!(paths.len() > 10, "the walk found the commands");
    for path in paths {
        let out = slopvac().args(&path).arg("--help").output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            !has_internal_id(&text),
            "{path:?} help names an id:\n{text}"
        );
    }
}

#[test]
fn every_placeholder_error_names_no_internal_id() {
    for (name, _) in slopvac_cli::cli::PLACEHOLDERS {
        let out = slopvac().arg(name).output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{name}");
        assert!(
            err.starts_with(&format!(
                "error[command-not-wired]: `slopvac {name}` is not available in this build yet."
            )),
            "{err}"
        );
        assert!(!has_internal_id(&err), "{name}: {err}");
    }
}

#[test]
fn the_generated_cli_reference_names_no_internal_id() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/reference/cli.md");
    let text = std::fs::read_to_string(path).unwrap();
    assert!(!has_internal_id(&text));
}
