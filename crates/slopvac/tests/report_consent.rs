//! `slopvac report consent on|off|show` through the built binary: the per-user record in the
//! state directory, its refusal of a damaged file and its refusal to change in CI.
//!
//! Every test points `XDG_STATE_HOME` at its own temporary directory and removes `CI` and
//! `GITHUB_ACTIONS`, so no test reads the real state directory or meets the CI refusal by
//! accident.
#![allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code: a panic is a failed test"
)]
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rstest::rstest;
use snapbox::cmd::{Command, cargo_bin};

fn slopvac(state_home: &Path) -> Command {
    let mut cmd = Command::new(cargo_bin!("slopvac"));
    for var in [
        "CI",
        "GITHUB_ACTIONS",
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd = cmd.env_remove(var);
    }
    cmd.env("NO_COLOR", "1")
        .env("SLOPVAC_NO_UPDATE_CHECK", "1")
        .env("XDG_STATE_HOME", state_home)
}

fn consent(state_home: &Path, arg: &str) -> std::process::Output {
    slopvac(state_home)
        .args(["report", "consent", arg])
        .output()
        .unwrap()
}

fn record_path(state_home: &Path) -> PathBuf {
    state_home.join("slopvac").join("reporting.json")
}

fn stage(state_home: &Path, bytes: &[u8]) {
    fs::create_dir_all(record_path(state_home).parent().unwrap()).unwrap();
    fs::write(record_path(state_home), bytes).unwrap();
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The Unix time that a `show` line reports, after `prefix`.
fn recorded_time(shown: &str, prefix: &str) -> u64 {
    let rest = shown
        .strip_prefix(prefix)
        .unwrap_or_else(|| panic!("{shown:?} does not start with {prefix:?}"));
    rest.trim_end().trim_end_matches('.').parse().unwrap()
}

mod conformance {
    use super::*;
    #[rstest]
    #[case::on("on", "Issue reporting consent: on, recorded at Unix time ")]
    #[case::off("off", "Issue reporting consent: off, recorded at Unix time ")]
    fn show_reports_the_recorded_value_and_the_time(#[case] value: &str, #[case] prefix: &str) {
        let dir = tempfile::tempdir().unwrap();
        let before = now();
        let set = consent(dir.path(), value);
        assert_eq!(set.status.code(), Some(0), "{}", stderr(&set));
        let after = now();

        let shown = consent(dir.path(), "show");
        assert_eq!(shown.status.code(), Some(0), "{}", stderr(&shown));
        let at = recorded_time(&stdout(&shown), prefix);
        assert!(
            (before..=after).contains(&at),
            "{at} not in {before}..={after}"
        );
    }

    #[test]
    fn show_with_no_record_reports_unset_and_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let shown = consent(dir.path(), "show");
        assert_eq!(shown.status.code(), Some(0), "{}", stderr(&shown));
        assert_eq!(stdout(&shown), "Issue reporting consent: unset.\n");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn the_record_is_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(consent(dir.path(), "on").status.code(), Some(0));
        let mode = fs::metadata(record_path(dir.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_new_value_keeps_the_unattended_entries() {
        let dir = tempfile::tempdir().unwrap();
        stage(
            dir.path(),
            br#"{"schema":1,"consent":"on","recorded":1000,"unattended":[1700000000,1700000100]}"#,
        );
        assert_eq!(consent(dir.path(), "off").status.code(), Some(0));
        let record: serde_json::Value =
            serde_json::from_slice(&fs::read(record_path(dir.path())).unwrap()).unwrap();
        assert_eq!(record["schema"], 1);
        assert_eq!(record["consent"], "off");
        assert!(record["recorded"].as_u64().unwrap() > 1000);
        assert_eq!(
            record["unattended"],
            serde_json::json!([1_700_000_000, 1_700_000_100])
        );
    }

    #[rstest]
    #[case::empty(b"".as_slice())]
    #[case::not_json(b"consent: on".as_slice())]
    #[case::truncated(br#"{"schema":1,"consent":"on","recorded":"#.as_slice())]
    #[case::missing_consent(br#"{"schema":1,"recorded":5,"unattended":[]}"#.as_slice())]
    #[case::missing_recorded(br#"{"schema":1,"consent":"on","unattended":[]}"#.as_slice())]
    #[case::missing_unattended(br#"{"schema":1,"consent":"on","recorded":5}"#.as_slice())]
    #[case::other_schema(br#"{"schema":2,"consent":"on","recorded":5,"unattended":[]}"#.as_slice())]
    #[case::unknown_value(br#"{"schema":1,"consent":"maybe","recorded":5,"unattended":[]}"#.as_slice())]
    #[case::negative_time(br#"{"schema":1,"consent":"on","recorded":-5,"unattended":[]}"#.as_slice())]
    #[case::not_an_object(b"[]".as_slice())]
    fn a_damaged_record_is_refused_and_left_as_it_was(#[case] bytes: &[u8]) {
        let dir = tempfile::tempdir().unwrap();
        stage(dir.path(), bytes);
        let out = consent(dir.path(), "on");
        assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
        assert!(
            stderr(&out).contains("error[report-state-failed]"),
            "{}",
            stderr(&out)
        );
        assert_eq!(fs::read(record_path(dir.path())).unwrap(), bytes);
    }

    #[test]
    fn a_record_over_64_kib_is_refused_and_left_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = br#"{"schema":1,"consent":"on","recorded":5,"unattended":[]}"#.to_vec();
        bytes.resize(64 * 1024 + 1, b' ');
        stage(dir.path(), &bytes);
        let out = consent(dir.path(), "off");
        assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
        assert!(stderr(&out).contains("error[report-state-failed]"));
        assert_eq!(fs::read(record_path(dir.path())).unwrap(), bytes);
    }

    #[test]
    fn a_record_that_is_not_a_regular_file_is_refused_and_left_in_place() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(record_path(dir.path())).unwrap();
        let out = consent(dir.path(), "on");
        assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
        assert!(stderr(&out).contains("error[report-state-failed]"));
        assert!(record_path(dir.path()).is_dir());
    }

    #[test]
    fn show_reports_a_damaged_record_the_same_way() {
        let dir = tempfile::tempdir().unwrap();
        stage(dir.path(), b"garbage");
        let out = consent(dir.path(), "show");
        assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
        assert!(stderr(&out).contains("error[report-state-failed]"));
        assert_eq!(fs::read(record_path(dir.path())).unwrap(), b"garbage");
    }

    #[rstest]
    #[case::ci("CI", "true")]
    #[case::ci_empty("CI", "")]
    #[case::github_actions("GITHUB_ACTIONS", "true")]
    fn a_consent_change_in_ci_is_refused_and_touches_nothing(
        #[case] var: &str,
        #[case] value: &str,
        #[values("on", "off")] arg: &str,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let out = slopvac(dir.path())
            .env(var, value)
            .args(["report", "consent", arg])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
        assert!(
            stderr(&out).contains("error[report-ci-refused]"),
            "{}",
            stderr(&out)
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn show_still_works_in_ci() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(consent(dir.path(), "on").status.code(), Some(0));
        let out = slopvac(dir.path())
            .env("CI", "true")
            .args(["report", "consent", "show"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert!(stdout(&out).starts_with("Issue reporting consent: on, recorded at Unix time "));
    }

    #[test]
    fn an_existing_wider_record_is_owner_only_after_a_write() {
        let dir = tempfile::tempdir().unwrap();
        stage(
            dir.path(),
            br#"{"schema":1,"consent":"off","recorded":5,"unattended":[]}"#,
        );
        fs::set_permissions(record_path(dir.path()), fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(consent(dir.path(), "on").status.code(), Some(0));
        let mode = fs::metadata(record_path(dir.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[rstest]
    #[case::unknown_key(
        br#"{"schema":1,"consent":"on","recorded":5,"unattended":[],"SENTINEL-4f2a":1}"#.as_slice()
    )]
    #[case::bad_value(
        br#"{"schema":1,"consent":"SENTINEL-4f2a","recorded":5,"unattended":[]}"#.as_slice()
    )]
    #[case::bad_type(
        br#"{"schema":1,"consent":"on","recorded":"SENTINEL-4f2a","unattended":[]}"#.as_slice()
    )]
    fn a_damaged_record_never_leaks_its_text_into_the_debug_log(#[case] bytes: &[u8]) {
        let dir = tempfile::tempdir().unwrap();
        stage(dir.path(), bytes);
        let out = slopvac(dir.path())
            .env("SLOPVAC_LOG", "debug")
            .args(["report", "consent", "on"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
        assert!(stderr(&out).contains("error[report-state-failed]"));
        assert!(
            !stderr(&out).contains("SENTINEL-4f2a"),
            "the record text reached stderr: {}",
            stderr(&out)
        );
    }

    #[test]
    fn a_rewrite_that_would_pass_64_kib_is_refused_and_the_record_stays() {
        let dir = tempfile::tempdir().unwrap();
        let head = br#"{"schema":1,"consent":"on","recorded":5,"unattended":["#;
        let mut bytes = head.to_vec();
        // Fill the record to 64 KiB. The tail `0]}` takes 3 bytes. At most one space of padding is
        // left, so the longer `recorded` and the newline of a rewrite pass the cap.
        while bytes.len() + 11 <= 64 * 1024 - 3 {
            bytes.extend_from_slice(b"1700000000,");
        }
        while bytes.len() + 2 <= 64 * 1024 - 3 {
            bytes.extend_from_slice(b"0,");
        }
        bytes.extend_from_slice(b"0]}");
        bytes.resize(64 * 1024, b' ');
        assert_eq!(bytes.len(), 64 * 1024);
        stage(dir.path(), &bytes);
        let out = consent(dir.path(), "off");
        assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
        assert!(stderr(&out).contains("error[report-state-failed]"));
        assert_eq!(fs::read(record_path(dir.path())).unwrap(), bytes);
    }

    #[rstest]
    fn a_positional_array_is_not_a_record(#[values("on", "off", "show")] arg: &str) {
        let dir = tempfile::tempdir().unwrap();
        let bytes = br#"[1,"on",5,[]]"#;
        stage(dir.path(), bytes);
        let out = consent(dir.path(), arg);
        assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
        assert!(stderr(&out).contains("error[report-state-failed]"));
        assert_eq!(fs::read(record_path(dir.path())).unwrap(), bytes);
    }

    #[rstest]
    fn a_ci_value_that_is_not_utf8_still_refuses_a_change(
        #[values("CI", "GITHUB_ACTIONS")] var: &str,
        #[values("on", "off")] arg: &str,
    ) {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let dir = tempfile::tempdir().unwrap();
        let out = slopvac(dir.path())
            .env(var, OsString::from_vec(vec![0xff, 0xfe]))
            .args(["report", "consent", arg])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
        assert!(
            stderr(&out).contains("error[report-ci-refused]"),
            "{}",
            stderr(&out)
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn the_ci_refusal_comes_before_the_project_policy_is_read() {
        let state = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        // Over the 1 MiB cap on the policy read, so the policy read fails.
        fs::write(
            project.path().join("slopvac.toml"),
            vec![b'#'; 1024 * 1024 + 1],
        )
        .unwrap();
        let out = slopvac(state.path())
            .env("CI", "true")
            .current_dir(project.path())
            .args(["report", "consent", "on"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
        assert!(
            stderr(&out).contains("error[report-ci-refused]"),
            "{}",
            stderr(&out)
        );
        assert_eq!(fs::read_dir(state.path()).unwrap().count(), 0);
    }

    /// Puts a directory's mode back when it drops, so the temporary directory can be cleaned up.
    struct RestoreMode(PathBuf, u32);

    impl Drop for RestoreMode {
        fn drop(&mut self) {
            if let Err(e) = fs::set_permissions(&self.0, fs::Permissions::from_mode(self.1)) {
                // A failed restore leaves a directory that cannot be cleaned up, so it must fail
                // the test. A second panic during unwinding would abort the process and hide the
                // first failure, so in that case the error goes to stderr instead.
                if std::thread::panicking() {
                    #[expect(
                        clippy::print_stderr,
                        reason = "test code: the only way to report without a second panic"
                    )]
                    {
                        eprintln!("could not restore the directory mode: {:?}", e.kind());
                    }
                } else {
                    panic!("could not restore the directory mode: {:?}", e.kind());
                }
            }
        }
    }

    #[test]
    fn a_failed_write_leaves_the_existing_record_and_its_mode_alone() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = br#"{"schema":1,"consent":"off","recorded":5,"unattended":[]}"#;
        stage(dir.path(), bytes);
        let state = dir.path().join("slopvac");
        fs::write(state.join("reporting.json.lock"), b"").unwrap();
        fs::set_permissions(record_path(dir.path()), fs::Permissions::from_mode(0o644)).unwrap();
        // Searchable but not writable: the guard opens and the record reads, but no file can be staged.
        fs::set_permissions(&state, fs::Permissions::from_mode(0o500)).unwrap();
        let _restore = RestoreMode(state, 0o700);
        let out = consent(dir.path(), "on");
        assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
        assert!(stderr(&out).contains("error[report-state-failed]"));
        assert_eq!(fs::read(record_path(dir.path())).unwrap(), bytes);
        let mode = fs::metadata(record_path(dir.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o644);
    }
}
