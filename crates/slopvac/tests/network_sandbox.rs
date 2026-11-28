//! Conformance test N1 (policies.md section 4): with the update check off, the commands that
//! have no network path finish in a network namespace that has no route out.
//!
//! Each command runs as `unshare --user --map-root-user --net -- slopvac ...`. A new network
//! namespace holds a loopback interface and nothing else, so any outward connection fails. The
//! test asserts each command's exit code and an output marker, so a command that stops early
//! on a usage error does not pass.
//!
//! The namespace is required. When `unshare` cannot build it, or the namespace holds more than
//! loopback, every test fails with the same message. Nothing skips. Ubuntu 24.04 runners
//! restrict unprivileged user namespaces, and the `rust` job in `.github/workflows/test.yml`
//! lifts that for the job.
//!
//! The test runs on Linux only: network namespaces do not exist elsewhere. A run on another
//! host does not execute it, so the row counts as covered only from a Linux run.
#![cfg(target_os = "linux")]
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a panic is a failed test"
)]

mod conformance {
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use assert_cmd::Command;
    use rstest::rstest;

    /// The prefix that puts a process in a new, empty network namespace.
    const NAMESPACE: [&str; 4] = ["--user", "--map-root-user", "--net", "--"];

    const NO_NAMESPACE: &str = "N1 needs `unshare --user --map-root-user --net` to build an \
        unprivileged network namespace that holds only loopback, and it cannot here. On Ubuntu \
        24.04, run `sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0` first.";

    fn scratch() -> tempfile::TempDir {
        tempfile::tempdir().expect("scratch directory")
    }

    fn binary() -> PathBuf {
        Command::cargo_bin("slopvac")
            .expect("binary builds")
            .get_program()
            .into()
    }

    /// `unshare` with the namespace prefix, then `program`.
    fn in_namespace(program: &Path) -> Command {
        let mut cmd = Command::new("unshare");
        cmd.args(NAMESPACE).arg(program);
        cmd.timeout(Duration::from_secs(60));
        cmd
    }

    /// Fail unless a process in the namespace sees loopback and no other interface.
    fn assert_the_namespace_has_no_route_out() {
        let out = Command::new("unshare")
            .args(NAMESPACE)
            .args(["cat", "/proc/net/dev"])
            .timeout(Duration::from_secs(30))
            .output()
            .unwrap_or_else(|_| panic!("{NO_NAMESPACE}"));
        assert!(out.status.success(), "{NO_NAMESPACE}");
        let table = String::from_utf8_lossy(&out.stdout);
        let interfaces: Vec<&str> = table
            .lines()
            .skip(2)
            .filter_map(|line| line.split(':').next())
            .map(str::trim)
            .collect();
        assert_eq!(interfaces, ["lo"], "{NO_NAMESPACE}");
    }

    /// Conformance: N1
    #[rstest]
    #[case::lint(&["lint", "a.md"], 0, "a.md: PASS")]
    #[case::validate(&["validate"], 0, "")]
    #[case::rules(&["rules"], 0, "chat-leakage")]
    #[case::explain(&["explain", "core:ai-residue.chat-leakage"], 0, "chat-leakage")]
    #[case::prime(&["prime"], 0, "slopvac")]
    // `reference` answers `command-not-wired` until it is wired. Change this case then.
    #[case::reference(&["reference"], 2, "")]
    #[case::claims(&["claims", "b.md", "--original", "a.md"], 0, "\"change\": \"changed\"")]
    fn a_command_with_no_network_path_finishes_with_no_route_out(
        #[case] args: &[&str],
        #[case] expected_exit: i32,
        #[case] marker: &str,
    ) {
        assert_the_namespace_has_no_route_out();
        let project = scratch();
        let root = project.path();
        let home = root.join("home");
        std::fs::create_dir_all(&home).expect("home");
        std::fs::write(root.join("slopvac.toml"), "schema_version = 3\n").expect("config");
        std::fs::write(root.join("a.md"), "The cache holds 10 entries.\n").expect("a.md");
        std::fs::write(root.join("b.md"), "The cache holds 20 entries.\n").expect("b.md");

        let mut cmd = in_namespace(&binary());
        cmd.args(args)
            .current_dir(root)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &home)
            .env("XDG_CACHE_HOME", home.join("cache"))
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_STATE_HOME", home.join("state"))
            .env("SLOPVAC_NO_UPDATE_CHECK", "1")
            .env("NO_COLOR", "1");
        let out = cmd
            .output()
            .unwrap_or_else(|e| panic!("{args:?} did not finish in the namespace: {e}"));

        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            out.status.code(),
            Some(expected_exit),
            "{args:?}\nstdout: {stdout}\nstderr: {stderr}"
        );
        assert!(stdout.contains(marker), "{args:?} printed:\n{stdout}");
        let expects_error = expected_exit != 0;
        assert_eq!(
            stderr.contains("error["),
            expects_error,
            "{args:?}\nstderr: {stderr}"
        );
        assert!(
            !stderr.contains("offline-network-needed"),
            "{args:?} needed the network:\n{stderr}"
        );
    }
}
