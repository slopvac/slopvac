//! Installed-binary tests for the command tree, exit codes and global settings.

mod support;

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

fn slopvac() -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    cmd.current_dir(support::neutral_dir());
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "NO_COLOR",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

fn doc(contents: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("doc.md");
    fs::write(&path, contents).expect("write");
    (dir, path)
}

#[test]
fn bare_path_lints_like_the_lint_command() {
    let (_dir, path) = doc(b"# Title\n\nPlain text.\n");
    slopvac().arg(&path).assert().code(0);
    slopvac().arg("lint").arg(&path).assert().code(0);
}

#[test]
fn bare_path_works_after_global_flags() {
    let (_dir, path) = doc(b"hello\n");
    slopvac()
        .args(["--offline", "--color", "never"])
        .arg(&path)
        .assert()
        .code(0);
}

#[test]
fn missing_file_is_an_incomplete_run() {
    slopvac()
        .arg("/nonexistent/slopvac/doc.md")
        .assert()
        .code(2);
}

#[test]
fn invalid_utf8_is_an_incomplete_run() {
    let (_dir, path) = doc(&[0x66, 0xff, 0x6f]);
    slopvac()
        .arg(&path)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("not valid UTF-8"));
}

#[test]
fn no_input_is_incomplete() {
    slopvac().arg("lint").assert().code(2);
}

#[test]
fn unknown_option_is_a_usage_error_with_exit_2() {
    slopvac().args(["lint", "--bogus"]).assert().code(2);
}

#[test]
fn version_and_help_exit_0() {
    slopvac()
        .arg("--version")
        .assert()
        .code(0)
        .stdout(predicate::str::starts_with("slopvac "));
    slopvac().arg("--help").assert().code(0);
}

#[test]
fn removed_options_point_at_the_replacement() {
    let (_dir, path) = doc(b"x\n");
    let cases = [
        (vec!["--profile", "strict"], "slopvac migrate"),
        (vec!["--rules-dir", "rules"], "slopvac migrate"),
        (vec!["--no-vale"], "Vale is gone"),
        (vec!["--comments"], "--mode code-comments"),
    ];
    for (args, expect) in cases {
        slopvac()
            .arg("lint")
            .args(&args)
            .arg(&path)
            .assert()
            .code(2)
            .stderr(predicate::str::contains(expect));
    }
}

#[test]
fn removed_commands_are_unknown() {
    for name in ["compile", "cache"] {
        let (_dir, path) = doc(b"x\n");
        // A removed command word is read as a path, so the lint fails to find it.
        slopvac().arg(name).arg(&path).assert().code(2);
    }
}

#[test]
fn help_offers_no_way_to_turn_offline_off() {
    let out = slopvac().arg("--help").output().expect("run");
    let help = String::from_utf8_lossy(&out.stdout).to_lowercase();
    assert!(help.contains("--offline"));
    for flag in ["--online", "--no-offline", "--allow-network"] {
        assert!(!help.contains(flag), "{flag}");
    }
}

#[test]
fn log_stays_off_without_slopvac_log() {
    let (_dir, path) = doc(b"x\n");
    slopvac()
        .arg(&path)
        .assert()
        .code(0)
        .stderr(predicate::str::is_empty());
}

#[test]
fn trace_logging_never_copies_document_text() {
    let sentinel = "ZX9-SENTINEL-4471";
    let (dir, path) = doc(format!("# {sentinel}\n\nBody {sentinel}\n").as_bytes());
    let out = slopvac()
        .env("SLOPVAC_LOG", "trace")
        .arg(&path)
        .output()
        .expect("run");
    let all = [out.stdout, out.stderr].concat();
    let all = String::from_utf8_lossy(&all);
    assert!(!all.contains(sentinel), "sentinel leaked: {all}");
    assert!(
        !all.contains(&dir.path().display().to_string()),
        "absolute path leaked: {all}"
    );
}

#[test]
fn logging_emits_on_stderr_and_leaves_stdout_unchanged() {
    let (_dir, path) = doc(b"x\n");
    let quiet = slopvac().arg(&path).output().expect("run");
    let logged = slopvac()
        .env("SLOPVAC_LOG", "debug")
        .arg(&path)
        .output()
        .expect("run");
    assert_eq!(logged.status.code(), Some(0));
    assert_eq!(logged.stdout, quiet.stdout);
    assert!(String::from_utf8_lossy(&logged.stderr).contains("lint start"));
}

#[test]
fn unwired_commands_exit_2_without_naming_anyone() {
    slopvac()
        .arg("onboard")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "is not available in this build yet.",
        ));
}

#[test]
fn completions_cover_every_shell() {
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        slopvac()
            .args(["completions", shell])
            .assert()
            .code(0)
            .stdout(predicate::str::contains("slopvac"));
    }
}

#[test]
fn man_page_is_roff() {
    slopvac()
        .arg("man")
        .assert()
        .code(0)
        .stdout(predicate::str::contains(".TH"));
}

#[test]
fn man_out_dir_writes_a_page_per_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    slopvac()
        .args(["man", "--out-dir"])
        .arg(dir.path())
        .assert()
        .code(0);
    for page in ["slopvac.1", "slopvac-lint.1", "slopvac-doctor.1"] {
        assert!(dir.path().join(page).is_file(), "{page}");
    }
}

#[test]
fn checked_in_cli_reference_is_current() {
    let generated = slopvac().arg("cli-reference").output().expect("run").stdout;
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/reference/cli.md");
    // A Windows checkout may convert the committed file to CRLF.
    let committed = fs::read_to_string(path)
        .expect("docs/reference/cli.md exists")
        .replace("\r\n", "\n");
    assert!(
        String::from_utf8(generated).expect("utf-8") == committed,
        "docs/reference/cli.md is stale; run `cargo run -p slopvac -- cli-reference > docs/reference/cli.md`"
    );
}

#[test]
fn failures_carry_stable_bracketed_codes() {
    let (_dir, bad) = doc(&[0xff]);
    let cases: Vec<(Vec<std::ffi::OsString>, &str)> = vec![
        (vec!["lint".into()], "error[no-input]: "),
        (
            vec![bad.clone().into_os_string()],
            "error[input-not-utf8]: ",
        ),
        (
            vec!["/nonexistent/x.md".into()],
            "error[input-unreadable]: ",
        ),
        (
            vec!["lint".into(), "--no-vale".into(), "x".into()],
            "error[removed-option]: ",
        ),
        (vec!["onboard".into()], "error[command-not-wired]: "),
        (vec!["lint".into(), "--bogus".into()], "error[usage]: "),
    ];
    for (args, prefix) in cases {
        slopvac()
            .args(&args)
            .assert()
            .code(2)
            .stderr(predicate::str::starts_with(prefix));
    }
}

#[test]
fn file_errors_never_show_absolute_paths() {
    let (dir, path) = doc(&[0xff]);
    let out = slopvac().arg(&path).output().expect("run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("<external>/doc.md"), "{stderr}");
    assert!(
        !stderr.contains(&dir.path().display().to_string()),
        "{stderr}"
    );
}

#[test]
fn debug_logging_with_forced_colour_does_not_panic() {
    let (_dir, path) = doc(b"x\n");
    slopvac()
        .env("SLOPVAC_LOG", "debug")
        .args(["--color", "always"])
        .arg(&path)
        .assert()
        .code(0)
        .stderr(predicate::str::contains("\u{1b}["));
}

#[test]
fn colour_flag_colours_help_and_no_color_does_not() {
    slopvac()
        .args(["--color", "always", "--help"])
        .assert()
        .code(0)
        .stdout(predicate::str::contains("\u{1b}["));
    slopvac()
        .args(["--help", "--no-color"])
        .env("FORCE_COLOR", "1")
        .assert()
        .code(0)
        .stdout(predicate::str::contains("\u{1b}[").not());
}

#[test]
fn man_pages_into_an_unwritable_directory_exit_2() {
    let (_dir, file) = doc(b"not a directory");
    slopvac()
        .args(["man", "--out-dir"])
        .arg(&file)
        .assert()
        .code(2)
        .stderr(predicate::str::starts_with("error[output-failed]"));
}

#[test]
fn usage_errors_keep_clap_styling_when_colour_is_forced() {
    slopvac()
        .args(["--color", "always", "lint", "--bogus"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("\u{1b}["))
        .stderr(predicate::str::contains("error[usage]:"));
    slopvac()
        .args(["--color", "never", "lint", "--bogus"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("\u{1b}[").not())
        .stderr(predicate::str::starts_with("error[usage]:"));
}

#[test]
fn version_json_prints_a_document_with_notices() {
    let cache = tempfile::tempdir().unwrap();
    let out = slopvac()
        .env("SLOPVAC_CACHE_DIR", cache.path())
        .env("SLOPVAC_OFFLINE", "1")
        .args(["--version", "--json"])
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    let doc: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(doc["name"], "slopvac");
    assert_eq!(doc["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(doc["notices"], serde_json::json!([]));
}

#[test]
fn self_update_on_an_unmanaged_install_names_the_upgrade_command() {
    // The test binary is not an installer install, so `self update` explains instead of fetching.
    slopvac()
        .env("SLOPVAC_OFFLINE", "1")
        .env("AXOUPDATER_CONFIG_PATH", "/nonexistent")
        .args(["self", "update"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[self-update-unmanaged]"));
}

/// A project directory with a `slopvac.toml` that holds `network_toml`.
fn project(network_toml: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("slopvac.toml"),
        format!("schema_version = 3\n[network]\n{network_toml}"),
    )
    .unwrap();
    dir
}

/// A command with every update opt-out variable removed.
fn update_enabled() -> Command {
    let mut cmd = slopvac();
    for var in [
        "CI",
        "NO_UPDATE_NOTIFIER",
        "SLOPVAC_NO_UPDATE_CHECK",
        "DO_NOT_TRACK",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

/// A cache directory that says 99.0.0 is out.
fn cache_with_newer_release() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("update-check.json"),
        r#"{"checked_at":4102444800,"latest":"99.0.0"}"#,
    )
    .unwrap();
    dir
}

fn version_json(cmd: &mut Command, cache: &std::path::Path) -> serde_json::Value {
    let out = cmd
        .env("SLOPVAC_CACHE_DIR", cache)
        .args(["--version", "--json"])
        .assert()
        .code(0)
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap()
}

#[test]
fn project_update_check_false_suppresses_the_notice() {
    let cache = cache_with_newer_release();
    let with_notice = version_json(&mut update_enabled(), cache.path());
    assert_eq!(with_notice["notices"][0]["x-latest"], "99.0.0");

    let dir = project("update_check = false\n");
    let silenced = version_json(update_enabled().current_dir(dir.path()), cache.path());
    assert_eq!(silenced["notices"], serde_json::json!([]));
}

#[test]
fn project_offline_blocks_self_update_before_any_request() {
    // Make the test binary look like an installer install, so only offline mode can stop it.
    let exe = slopvac().get_program().to_owned();
    let prefix = std::path::Path::new(&exe).parent().unwrap().to_owned();
    let receipts = tempfile::tempdir().unwrap();
    fs::write(
        receipts.path().join("slopvac-receipt.json"),
        serde_json::json!({
            "install_prefix": prefix,
            "source": {"app_name": "slopvac"},
        })
        .to_string(),
    )
    .unwrap();
    let dir = project("offline = true\n");
    slopvac()
        .current_dir(dir.path())
        .env("AXOUPDATER_CONFIG_PATH", receipts.path())
        .args(["self", "update"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "error[offline-network-needed]: offline: self update needs the network",
        ));
}

#[test]
fn version_json_accepts_flags_in_any_order() {
    let cache = cache_with_newer_release();
    for args in [
        &["--json", "--version"][..],
        &["--color", "never", "--version", "--json"],
        &["--color=never", "--json", "-V"],
        &["--no-color", "--version", "--json"],
    ] {
        let out = update_enabled()
            .env("SLOPVAC_CACHE_DIR", cache.path())
            .args(args)
            .assert()
            .code(0)
            .get_output()
            .stdout
            .clone();
        let doc: serde_json::Value =
            serde_json::from_slice(&out).unwrap_or_else(|e| panic!("{args:?}: {e}"));
        assert_eq!(doc["name"], "slopvac", "{args:?}");
    }
}

#[test]
fn unreadable_project_config_exits_two_before_any_command_runs() {
    // A directory, an oversize file and (on unix) a FIFO all fail the same way and never hang.
    let mut cases: Vec<(&str, &str, tempfile::TempDir)> = Vec::new();

    let dir_case = tempfile::tempdir().unwrap();
    fs::create_dir(dir_case.path().join("slopvac.toml")).unwrap();
    cases.push(("config-unreadable", "not a regular file", dir_case));

    let big = tempfile::tempdir().unwrap();
    fs::write(big.path().join("slopvac.toml"), vec![b'#'; 1024 * 1024 + 1]).unwrap();
    cases.push(("update-policy-too-large", "larger than 1048576 bytes", big));

    #[cfg(unix)]
    {
        let fifo_case = tempfile::tempdir().unwrap();
        let status = std::process::Command::new("mkfifo")
            .arg(fifo_case.path().join("slopvac.toml"))
            .status()
            .expect("mkfifo");
        assert!(status.success());
        cases.push(("config-unreadable", "not a regular file", fifo_case));
    }

    for (code, message, dir) in &cases {
        for args in [
            &["--version", "--json"][..],
            &["self", "update"],
            &["--help"],
        ] {
            slopvac()
                .current_dir(dir.path())
                .timeout(std::time::Duration::from_secs(20))
                .args(args)
                .assert()
                .code(2)
                .stderr(predicate::str::contains(format!("error[{code}]")))
                .stderr(predicate::str::contains(*message));
        }
    }
}

#[test]
fn config_read_errors_do_not_print_absolute_paths() {
    // policies.md 1.7: paths in errors are relative to the project root, or `<external>/<name>`.
    let base = tempfile::Builder::new()
        .prefix("sentinel-secret-")
        .tempdir()
        .unwrap();
    let sentinel = base
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    fs::create_dir(base.path().join("slopvac.toml")).unwrap();
    let sub = base.path().join("sub");
    fs::create_dir(&sub).unwrap();

    // The config sits above the working directory, so it is outside the root.
    let outside = slopvac()
        .current_dir(&sub)
        .arg("--version")
        .assert()
        .code(2)
        .get_output()
        .stderr
        .clone();
    let outside = String::from_utf8(outside).unwrap();
    assert!(!outside.contains(&sentinel), "{outside}");
    assert!(
        outside.contains("error[config-unreadable]: <external>/slopvac.toml: not a regular file"),
        "{outside}"
    );

    // The config sits in the working directory: shown relative to it.
    let inside = slopvac()
        .current_dir(base.path())
        .arg("--version")
        .assert()
        .code(2)
        .get_output()
        .stderr
        .clone();
    let inside = String::from_utf8(inside).unwrap();
    assert!(!inside.contains(&sentinel), "{inside}");
    assert!(
        inside.contains("error[config-unreadable]: slopvac.toml: not a regular file"),
        "{inside}"
    );
}

fn frame(body: &str) -> String {
    format!("Content-Length: {}\r\n\r\n{body}", body.len())
}

#[test]
fn lsp_answers_initialize_over_stdio_and_exits_0_after_shutdown() {
    let input = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":null,"capabilities":{}}}"#,
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#,
        r#"{"jsonrpc":"2.0","method":"exit"}"#,
    ]
    .map(frame)
    .concat();
    slopvac()
        .arg("lsp")
        .write_stdin(input)
        .timeout(std::time::Duration::from_secs(30))
        .assert()
        .code(0)
        .stdout(predicate::str::contains(r#""positionEncoding":"utf-16""#))
        .stdout(predicate::str::contains(r#""name":"slopvac""#));
}

const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":null,"capabilities":{}}}"#;

#[test]
fn lsp_exit_without_shutdown_exits_1() {
    let input = [
        INITIALIZE,
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
        r#"{"jsonrpc":"2.0","method":"exit"}"#,
    ]
    .map(frame)
    .concat();
    slopvac()
        .arg("lsp")
        .write_stdin(input)
        .timeout(std::time::Duration::from_secs(30))
        .assert()
        .code(1);
}

#[test]
fn lsp_stops_on_a_fatal_error_while_stdin_stays_open() {
    use std::io::Write as _;
    // `params` is a number, so the server cannot read the initialize request and ends in
    // error. The client keeps its end of stdin open, so the reader thread never sees EOF.
    let bad = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":5}"#;
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("slopvac"))
        .arg("lsp")
        .current_dir(support::neutral_dir())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn slopvac");
    let mut stdin = child.stdin.take().expect("piped stdin");
    stdin.write_all(frame(bad).as_bytes()).expect("write");
    stdin.flush().expect("flush");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break Some(status);
        }
        if std::time::Instant::now() > deadline {
            child.kill().expect("kill");
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    drop(stdin);
    let status = status.expect("the process exits while stdin is open");
    assert_eq!(status.code(), Some(2));
}

/// Read one framed LSP message body from `reader`.
fn read_message(reader: &mut impl std::io::BufRead) -> String {
    let mut length = 0;
    loop {
        let mut line = String::new();
        assert!(
            reader.read_line(&mut line).expect("header") > 0,
            "stdout closed"
        );
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length: ") {
            length = value.parse().expect("length");
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).expect("body");
    String::from_utf8(body).expect("utf-8")
}

/// What one run of the stopped-reader scenario showed.
enum Stopped {
    /// The worker had already written so many diagnostics before the shutdown response that
    /// too few were left to fill the pipe: the run proves nothing.
    NotReached { received: usize },
    /// The scenario held. `code` is the exit code, or `None` if the process was still
    /// running at the deadline.
    Held { code: Option<i32> },
}

/// Size of one `publishDiagnostics` message for a document with the long uri below.
const PUBLISH_BYTES: usize = 2000;
/// Unread diagnostics that must remain when the client stops reading: far more than any
/// OS pipe buffer (64 KiB on Linux, up to 64 KiB on macOS).
const MIN_UNREAD_BYTES: usize = 1 << 20;

/// Open `documents` documents and ask for shutdown in one write, read until the shutdown
/// response, and stop reading. The worker lints the documents while the main loop answers
/// the shutdown, so diagnostics are still queued behind the response.
fn stopped_reader_run(documents: usize) -> Stopped {
    use std::io::Write as _;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("slopvac"))
        .arg("lsp")
        .current_dir(support::neutral_dir())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn slopvac");
    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = std::io::BufReader::new(child.stdout.take().expect("piped stdout"));
    let child = Arc::new(std::sync::Mutex::new(child));
    // A hung server must not hang the test run: kill it after a while.
    let finished = Arc::new(AtomicBool::new(false));
    let watchdog = {
        let (child, finished) = (Arc::clone(&child), Arc::clone(&finished));
        std::thread::spawn(move || {
            for _ in 0..1800 {
                if finished.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let _ = child.lock().unwrap().kill();
        })
    };

    let mut input = String::new();
    input.push_str(&frame(INITIALIZE));
    input.push_str(&frame(
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
    ));
    let padding = "a".repeat(PUBLISH_BYTES - 100);
    for n in 0..documents {
        let open = format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///{padding}{n}.md","languageId":"markdown","version":1,"text":"x"}}}}}}"#
        );
        input.push_str(&frame(&open));
    }
    input.push_str(&frame(r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#));
    // One write, so the shutdown is read right behind the last document.
    stdin.write_all(input.as_bytes()).expect("write requests");
    stdin.flush().expect("flush");

    // Read up to the shutdown response, then stop reading but keep the pipe open.
    let mut received = 0;
    loop {
        let body = read_message(&mut stdout);
        if body.contains("publishDiagnostics") {
            received += 1;
        } else if body.contains(r#""id":2"#) && !body.contains("method") {
            break;
        }
    }
    let unread = documents.saturating_sub(received);
    let outcome = if unread * PUBLISH_BYTES < MIN_UNREAD_BYTES {
        None
    } else {
        stdin
            .write_all(frame(r#"{"jsonrpc":"2.0","method":"exit"}"#).as_bytes())
            .expect("write exit");
        stdin.flush().expect("flush");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            if let Some(status) = child.lock().unwrap().try_wait().expect("wait") {
                break Some(status.code());
            }
            if std::time::Instant::now() > deadline {
                break Some(None);
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    };
    finished.store(true, Ordering::SeqCst);
    let _ = child.lock().unwrap().kill();
    // Reap the child so discarded attempts leave no zombie behind.
    let _ = child.lock().unwrap().wait();
    watchdog.join().unwrap();
    drop((stdin, stdout));
    match outcome {
        None => Stopped::NotReached { received },
        Some(code) => Stopped::Held { code },
    }
}

#[test]
fn lsp_exits_after_shutdown_although_the_client_stopped_reading_stdout() {
    // The scenario needs diagnostics still queued behind the shutdown response. How many
    // are depends on the machine's speed, so grow the load until the run shows they were.
    let mut seen = Vec::new();
    for documents in [1000, 3000, 8000] {
        match stopped_reader_run(documents) {
            Stopped::Held { code } => {
                assert_eq!(code, Some(0), "the process exits while stdout is unread");
                return;
            }
            Stopped::NotReached { received } => seen.push((documents, received)),
        }
    }
    panic!(
        "the stopped-reader scenario was never reached; (documents, diagnostics read before \
         the shutdown response): {seen:?}"
    );
}

#[test]
fn lsp_publishes_findings_from_the_shipped_rules() {
    let open = r##"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///work/doc.md","languageId":"markdown","version":1,"text":"# T\n\nWe leverage it.\n"}}}"##;
    let input = [
        INITIALIZE,
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
        open,
        r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#,
        r#"{"jsonrpc":"2.0","method":"exit"}"#,
    ]
    .map(frame)
    .concat();
    slopvac()
        .arg("lsp")
        .write_stdin(input)
        .timeout(std::time::Duration::from_secs(60))
        .assert()
        .code(0)
        .stdout(predicate::str::contains(
            "core:prose-inflation.slop-lexicon",
        ));
}
