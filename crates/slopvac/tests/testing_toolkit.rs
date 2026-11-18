//! Worked examples of the workspace testing conventions: see `docs/testing-rust.md`.
//!
//! These tests drive the built binary the way a user does, so they survive any
//! refactor of the code behind it.

use proptest::prelude::*;
use rstest::rstest;
use snapbox::cmd::{Command, cargo_bin};
use std::time::{Duration, Instant};

/// The binary under test, with the ambient colour and logging settings cleared so
/// a developer's or CI's `FORCE_COLOR` cannot change the output.
fn slopvac() -> Command {
    let mut cmd = Command::new(cargo_bin!("slopvac"));
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd = cmd.env_remove(var);
    }
    cmd.env("NO_COLOR", "1")
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
#[case::short("-h")]
#[case::version_short("-V")]
fn short_flags_succeed(#[case] flag: &str) {
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

/// Longest a generated command line may run before the case fails.
const CASE_TIMEOUT: Duration = Duration::from_secs(10);

/// Run `cmd` with empty stdin under `timeout` and judge the exit: only codes 0 and 2
/// pass. snapbox kills a child that outlives the timeout, which leaves no exit code,
/// so a hang and a signal death both come back as `Err`.
fn clean_exit(cmd: Command, timeout: Duration) -> Result<(), String> {
    let output = cmd
        .stdin("")
        .timeout(timeout)
        .output()
        .map_err(|e| format!("binary did not run: {e}"))?;
    match output.status.code() {
        Some(0 | 2) => Ok(()),
        code => Err(format!(
            "exit code was {code:?} (no code: killed by the {timeout:?} timeout or a signal)"
        )),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// proptest: state an invariant over generated input. Arbitrary words must
    /// make the CLI exit cleanly (0 or 2), never crash or hang. Each case spawns
    /// the binary, so the case count stays low.
    #[test]
    fn arbitrary_arguments_never_crash(args in proptest::collection::vec("[a-zA-Z0-9_./-]{0,12}", 0..4)) {
        let verdict = clean_exit(slopvac().args(&args), CASE_TIMEOUT);
        prop_assert!(verdict.is_ok(), "{}", verdict.unwrap_err());
    }
}

/// A command that outlives the timeout fails the verdict, and quickly.
#[cfg(unix)]
#[test]
fn hanging_command_fails_the_case_within_the_bound() {
    let start = Instant::now();
    let verdict = clean_exit(Command::new("sleep").arg("30"), Duration::from_secs(1));
    assert!(verdict.is_err(), "a hang must not count as a clean exit");
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "timeout did not stop the child: {:?}",
        start.elapsed()
    );
}
