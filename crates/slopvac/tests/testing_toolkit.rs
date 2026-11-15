//! Worked examples of the workspace testing conventions: see `docs/testing-rust.md`.
//!
//! These tests drive the built binary the way a user does, so they survive any
//! refactor of the code behind it.

use proptest::prelude::*;
use rstest::rstest;
use snapbox::cmd::{Command, cargo_bin};

fn slopvac() -> Command {
    Command::new(cargo_bin!("slopvac"))
}

/// snapbox: assert on output with inline expectations. `[..]` matches any text.
#[test]
fn version_prints_name_and_semver() {
    slopvac()
        .arg("--version")
        .assert()
        .success()
        .stdout_eq("slopvac [..].[..].[..]\n");
}

/// rstest: one behaviour, several inputs. Cases differ only in data.
#[rstest]
#[case::long("--help")]
#[case::short("-h")]
fn help_flags_succeed(#[case] flag: &str) {
    slopvac().arg(flag).assert().success();
}

/// insta: snapshot output that is too large to write inline. Review changes
/// with `cargo insta review`; CI never writes snapshots.
#[test]
fn help_text_snapshot() {
    let output = slopvac().arg("--help").output().expect("binary runs");
    let help = String::from_utf8(output.stdout).expect("utf-8 help");
    // Normalise what differs by platform: Windows prints `slopvac.exe` and CRLF.
    insta::assert_snapshot!(help.replace("\r\n", "\n").replace("slopvac.exe", "slopvac"));
}

proptest! {
    /// proptest: state an invariant over generated input. Arbitrary words must
    /// make the CLI exit cleanly (0 or 2), never crash or hang.
    #[test]
    fn arbitrary_arguments_never_crash(args in proptest::collection::vec("[a-zA-Z0-9_./-]{0,12}", 0..4)) {
        let output = slopvac().args(&args).output().expect("binary runs");
        let code = output.status.code();
        prop_assert!(matches!(code, Some(0 | 2)), "exit code was {code:?}");
    }
}
