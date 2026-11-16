//! `slopvac onboard` and the removed `slopvac reference` through the binary
//! (compat rows `cmd:onboard`, `cmd:reference`, `opt:reference:--write`,
//! `opt:reference:--check`, `behavior:onboard-output`, `behavior:reference-write`,
//! `behavior:reference-check`).

use assert_cmd::Command;
use predicates::prelude::*;

fn slopvac() -> Command {
    Command::cargo_bin("slopvac").expect("binary builds")
}

#[test]
fn onboard_prints_the_setup_text_and_points_to_prime() {
    slopvac()
        .arg("onboard")
        .assert()
        .code(0)
        .stdout(predicate::str::starts_with("# Slopvac agent setup"))
        .stdout(predicate::str::contains("slopvac init"))
        .stdout(predicate::str::contains("slopvac prime"))
        .stderr(predicate::str::is_empty());
}

#[test]
fn reference_exits_2_with_a_pointer_to_the_docs_build() {
    for args in [
        &["reference"][..],
        &["reference", "--write"],
        &["reference", "--check"],
    ] {
        slopvac()
            .args(args)
            .assert()
            .code(2)
            .stdout(predicate::str::is_empty())
            .stderr(predicate::str::starts_with("error[command-removed]"))
            .stderr(predicate::str::contains("just docs"))
            .stderr(predicate::str::contains("just docs-check"));
    }
}

#[test]
fn reference_is_not_listed_in_help() {
    slopvac()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("onboard"))
        .stdout(predicate::str::contains("reference").not());
}
