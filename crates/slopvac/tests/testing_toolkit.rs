//! Worked examples of the workspace testing conventions: see `docs/testing-rust.md`.
//!
//! These tests drive the built binary the way a user does, so they survive any
//! refactor of the code behind it.

use proptest::prelude::*;
use rstest::rstest;
use snapbox::cmd::{Command, cargo_bin};
use std::path::Path;
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
    cmd.env("NO_COLOR", "1").env("SLOPVAC_NO_UPDATE_CHECK", "1")
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

/// The binary run inside `dir`, with every directory it may read or write for the
/// user (home, cache, config, data, state) pointed there. A case then sees an empty
/// tree and cannot touch the runner's real user files.
fn slopvac_in(dir: &Path) -> Command {
    let home = dir.as_os_str();
    slopvac()
        .current_dir(dir)
        .env("HOME", home)
        .env("SLOPVAC_CACHE_DIR", home)
        .env("XDG_CACHE_HOME", home)
        .env("XDG_CONFIG_HOME", home)
        .env("XDG_DATA_HOME", home)
        .env("XDG_STATE_HOME", home)
}

/// A word that stays inside the working directory: it starts with no `/`, so it is
/// never absolute, and it holds no `..`, so it never climbs out. Built by
/// construction, not by rejecting, so no generated case is discarded.
fn contained_word() -> impl Strategy<Value = String> {
    "([a-zA-Z0-9_.-][a-zA-Z0-9_./-]{0,11})?".prop_map(|mut word| {
        while word.contains("..") {
            word = word.replace("..", ".");
        }
        word
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// proptest: state an invariant over generated input. Arbitrary words must
    /// make the CLI exit cleanly (0 or 2), never crash or hang. Each case spawns
    /// the binary in an empty directory, so the case count stays low and an
    /// argument like `/` cannot walk the host's file system.
    #[test]
    fn arbitrary_arguments_never_crash(args in proptest::collection::vec(contained_word(), 0..4)) {
        let dir = tempfile::tempdir().expect("temp dir");
        let verdict = clean_exit(slopvac_in(dir.path()).args(&args), CASE_TIMEOUT);
        prop_assert!(verdict.is_ok(), "{}", verdict.unwrap_err());
    }
}

/// The current directory is a path the CLI may be asked to check. In an empty
/// directory it ends within the timeout with a clean exit.
#[test]
fn the_current_directory_as_the_only_argument_exits_cleanly() {
    let dir = tempfile::tempdir().expect("temp dir");
    let verdict = clean_exit(slopvac_in(dir.path()).arg("."), CASE_TIMEOUT);
    assert!(verdict.is_ok(), "{}", verdict.unwrap_err());
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
