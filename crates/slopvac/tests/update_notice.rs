//! The update notice after `lint`, `doctor` and `prime`: stderr only, once the command's own
//! output is done, and only where the update policy allows it.
//!
//! The network is never reached. A cache entry stamped far in the future counts as fresh, so
//! the notice comes from the cache and no refresh starts.

use std::cell::RefCell;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::rc::Rc;

use rstest::rstest;
use snapbox::cmd::{Command, cargo_bin};

const NOTICE: &str = "notice[update-available]: slopvac 99.0.0 is available (you have ";

/// A cache directory that says 99.0.0 is out.
fn seeded_cache() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("update-check.json"),
        r#"{"checked_at":4102444800,"latest":"99.0.0"}"#,
    )
    .expect("seed the cache");
    dir
}

/// The binary in `work`, with home, cache and config directories inside `home`, and every
/// update opt-out removed.
fn slopvac(work: &Path, home: &Path, cache: &Path) -> Command {
    let mut cmd = Command::new(cargo_bin!("slopvac"));
    for var in [
        "CI",
        "NO_UPDATE_NOTIFIER",
        "SLOPVAC_NO_UPDATE_CHECK",
        "DO_NOT_TRACK",
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
        "XDG_CACHE_HOME",
        "XDG_CONFIG_HOME",
    ] {
        cmd = cmd.env_remove(var);
    }
    cmd.env("NO_COLOR", "1")
        .env("HOME", home)
        .env("SLOPVAC_CACHE_DIR", cache)
        .env("AXOUPDATER_CONFIG_PATH", home.join("receipts"))
        .current_dir(work)
}

struct Fixture {
    work: tempfile::TempDir,
    home: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let work = tempfile::tempdir().expect("tempdir");
    fs::write(work.path().join("slopvac.toml"), "schema_version = 3\n").expect("config");
    fs::write(work.path().join("doc.md"), "# Title\n\nPlain words here.\n").expect("doc");
    Fixture {
        work,
        home: tempfile::tempdir().expect("tempdir"),
    }
}

fn run(fx: &Fixture, cache: &Path, args: &[&str]) -> std::process::Output {
    slopvac(fx.work.path(), fx.home.path(), cache)
        .args(args)
        .assert()
        .get_output()
        .clone()
}

#[rstest]
#[case::lint(&["lint", "doc.md"])]
#[case::doctor(&["doctor"])]
#[case::prime(&["prime"])]
fn a_cached_newer_release_is_noticed_on_stderr_after_the_output(#[case] args: &[&str]) {
    let fx = fixture();
    let cache = seeded_cache();
    let out = run(&fx, cache.path(), args);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stderr.contains(NOTICE), "stderr: {stderr}");
    assert!(!stdout.contains("update-available"), "stdout: {stdout}");
    let last = stderr.lines().last().unwrap_or_default();
    assert!(last.starts_with(NOTICE), "the notice comes last: {stderr}");
}

// `doctor` is left out: its report names the offline state this comparison turns on.
#[rstest]
#[case::lint(&["lint", "doc.md"])]
#[case::prime(&["prime"])]
fn the_notice_leaves_stdout_and_the_exit_code_alone(#[case] args: &[&str]) {
    let fx = fixture();
    let quiet = tempfile::tempdir().expect("tempdir");
    let with = run(&fx, seeded_cache().path(), args);
    let without = run(&fx, quiet.path(), &[args, &["--offline"]].concat());
    assert_eq!(with.status.code(), without.status.code());
    assert_eq!(with.stdout, without.stdout);
}

#[rstest]
#[case::ci("CI", "true")]
#[case::no_update_notifier("NO_UPDATE_NOTIFIER", "1")]
#[case::no_update_check("SLOPVAC_NO_UPDATE_CHECK", "1")]
#[case::do_not_track("DO_NOT_TRACK", "1")]
fn an_opt_out_variable_suppresses_the_notice(#[case] name: &str, #[case] value: &str) {
    let fx = fixture();
    let cache = seeded_cache();
    let out = slopvac(fx.work.path(), fx.home.path(), cache.path())
        .env(name, value)
        .args(["lint", "doc.md"])
        .assert()
        .get_output()
        .clone();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("update-available"), "stderr: {stderr}");
}

#[test]
fn update_check_false_in_the_project_config_suppresses_the_notice() {
    let fx = fixture();
    fs::write(
        fx.work.path().join("slopvac.toml"),
        "schema_version = 3\n[network]\nupdate_check = false\n",
    )
    .expect("config");
    let cache = seeded_cache();
    let out = run(&fx, cache.path(), &["lint", "doc.md"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("update-available"), "stderr: {stderr}");
}

#[rstest]
#[case::lint(&["lint", "doc.md"])]
#[case::doctor(&["doctor"])]
#[case::prime(&["prime"])]
fn offline_gives_no_notice_even_from_a_cache(#[case] args: &[&str]) {
    let fx = fixture();
    let cache = seeded_cache();
    let out = run(&fx, cache.path(), &[args, &["--offline"]].concat());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("update-available"), "stderr: {stderr}");
}

#[rstest]
#[case::lint(&["lint", "doc.md"])]
#[case::doctor(&["doctor"])]
#[case::prime(&["prime"])]
fn offline_starts_no_refresh_and_writes_no_cache(#[case] args: &[&str]) {
    let fx = fixture();
    let cache = tempfile::tempdir().expect("tempdir");
    run(&fx, cache.path(), &[args, &["--offline"]].concat());
    assert!(!cache.path().join("update-check.json").exists());
}

#[rstest]
#[case::version(&["--version"])]
#[case::rules(&["rules", "--help"])]
#[case::self_update_help(&["self", "update", "--help"])]
fn other_commands_print_no_notice(#[case] args: &[&str]) {
    let fx = fixture();
    let cache = seeded_cache();
    let out = run(&fx, cache.path(), args);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("update-available"), "stderr: {stderr}");
}

/// One ordered log of every write and flush on either stream.
type Log = Rc<RefCell<Vec<String>>>;

/// A stream that records each call, with its own name, in the shared log.
struct Recorder {
    name: &'static str,
    log: Log,
}

impl Write for Recorder {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        self.log
            .borrow_mut()
            .push(format!("{} write: {text}", self.name));
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.log.borrow_mut().push(format!("{} flush", self.name));
        Ok(())
    }
}

#[test]
fn stdout_is_flushed_before_the_notice_is_written() {
    let cache = seeded_cache();
    let home = tempfile::tempdir().expect("tempdir");
    // `main_with_io` reads the working directory. This is the only test here that runs in the
    // test process rather than in the binary, so changing it cannot race another test.
    let fx = fixture();
    std::env::set_current_dir(fx.work.path()).expect("enter the project");
    let cache_dir = cache.path().to_string_lossy().into_owned();
    let home_dir = home.path().to_string_lossy().into_owned();
    let env = slopvac_cli::env::MapEnv::new(
        &[
            ("SLOPVAC_CACHE_DIR", cache_dir.as_str()),
            ("HOME", home_dir.as_str()),
        ],
        false,
    );
    let log = Log::default();
    let mut out = Recorder {
        name: "out",
        log: Rc::clone(&log),
    };
    let mut err = Recorder {
        name: "err",
        log: Rc::clone(&log),
    };
    let _ = slopvac_cli::main_with_io(["slopvac", "prime"], &env, &mut out, &mut err);

    let log = log.borrow();
    let last_out_flush = log
        .iter()
        .rposition(|l| l == "out flush")
        .unwrap_or_else(|| panic!("stdout was not flushed: {log:?}"));
    let notice = log
        .iter()
        .position(|l| l.starts_with("err write: ") && l.contains(NOTICE))
        .unwrap_or_else(|| panic!("no notice on stderr: {log:?}"));
    assert!(
        last_out_flush < notice,
        "stdout is flushed before the notice: {log:?}"
    );
}
