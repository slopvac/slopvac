//! `slopvac report`: preview, submit, prefilled URL and redaction.
//!
//! The library tests inject the token source, the poster and the browser opener, so nothing
//! touches the network. `github.rs` tests the real poster against a local listener. The binary
//! tests at the end run `slopvac` with no token and no `gh` on the path.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use slopvac_cli::cli;
use slopvac_cli::env::MapEnv;
use slopvac_cli::report::gh::{Gh, GhError};
use slopvac_cli::report::github::{Opener, prefilled_url};
use slopvac_cli::report::{Deps, run};

const COMPLETE_BUG: [&str; 12] = [
    "--field",
    "agent=Claude Code",
    "--field",
    "model=test-model",
    "--field",
    "harness=test-harness 1.0",
    "--field",
    "latest=Yes",
    "--field",
    "duplicates=true",
    "--field",
    "environment={\"ok\":true}",
];
const BUG_REST: [&str; 8] = [
    "--field",
    "command=slopvac lint --stdin",
    "--field",
    "what-happened=it crashed",
    "--field",
    "expected=no crash",
    "--field",
    "reporter=true",
];

/// A stand-in for the GitHub CLI that records what it was asked to do.
struct FakeGh {
    check: Result<(), GhError>,
    created: RefCell<Vec<(String, String)>>,
}

impl Gh for FakeGh {
    fn check(&self) -> Result<(), GhError> {
        self.check.clone()
    }

    fn create(&self, title: &str, body_file: &Path) -> Result<String, GhError> {
        let body = std::fs::read_to_string(body_file).unwrap();
        self.created.borrow_mut().push((title.to_owned(), body));
        Ok("https://github.com/slopvac/slopvac/issues/77".into())
    }
}

struct FakeOpener {
    opened: RefCell<Vec<String>>,
    fail: bool,
}

impl Opener for FakeOpener {
    fn open(&self, url: &str) -> std::io::Result<()> {
        self.opened.borrow_mut().push(url.to_owned());
        if self.fail {
            Err(std::io::Error::other("no browser"))
        } else {
            Ok(())
        }
    }
}

struct World {
    env: MapEnv,
    gh: FakeGh,
    opener: FakeOpener,
    cwd: PathBuf,
    offline: bool,
    stderr: RefCell<String>,
}

impl World {
    /// What `gh` receives (title, then body) when the report is filed with `--gh --yes`.
    fn filed(&self, args: &[String]) -> String {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("body.md");
        let mut all = args.to_vec();
        all.extend(["--body-file", path.to_str().unwrap(), "--gh", "--yes"].map(String::from));
        let (result, _) = self.run(&refs(&all));
        result.unwrap();
        let created = self.gh.created.borrow();
        let (title, body) = created.last().expect("gh was not called");
        format!("{title}\n{body}")
    }

    fn new() -> Self {
        Self {
            env: MapEnv::default(),
            gh: FakeGh {
                check: Ok(()),
                created: RefCell::default(),
            },
            opener: FakeOpener {
                opened: RefCell::default(),
                fail: false,
            },
            cwd: PathBuf::from("/nonexistent-cwd-for-report-tests"),
            offline: false,
            stderr: RefCell::default(),
        }
    }

    /// Run `slopvac report <args>` and return the exit result and everything printed.
    fn run(&self, args: &[&str]) -> (Result<u8, slopvac_cli::errors::CliError>, String) {
        let mut out = Vec::new();
        let result = self.run_into(args, &mut out);
        (result, String::from_utf8(out).unwrap())
    }

    /// Like `run`, writing to `out`. The warnings go to `self.stderr`.
    fn run_into(
        &self,
        args: &[&str],
        out: &mut dyn std::io::Write,
    ) -> Result<u8, slopvac_cli::errors::CliError> {
        let matches = cli::command()
            .try_get_matches_from(
                std::iter::once("slopvac")
                    .chain(["report"])
                    .chain(args.iter().copied()),
            )
            .unwrap_or_else(|e| panic!("usage: {e}"));
        let (_, m) = matches.subcommand().unwrap();
        let mut err = Vec::new();
        let result = run(
            m,
            &Deps {
                env: &self.env,
                offline: self.offline,
                cwd: self.cwd.clone(),
                opener: &self.opener,
                gh: &self.gh,
            },
            out,
            &mut err,
        );
        *self.stderr.borrow_mut() = String::from_utf8(err).unwrap();
        result
    }
}

/// Joins a token prefix and body at runtime, so the source never holds a token-shaped
/// literal that a secret scanner would flag.
fn fake(prefix: &str, body: &str) -> String {
    format!("{prefix}{body}")
}

fn bug(extra: &[&str]) -> Vec<String> {
    let mut v = vec![
        "bug".to_owned(),
        "--title".to_owned(),
        "crash on empty input".to_owned(),
    ];
    v.extend(COMPLETE_BUG.iter().map(|s| (*s).to_owned()));
    v.extend(BUG_REST.iter().map(|s| (*s).to_owned()));
    v.extend(extra.iter().map(|s| (*s).to_owned()));
    v
}

fn refs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

#[test]
fn a_plain_run_previews_and_prints_the_form_url_without_calling_gh() {
    let w = World::new();
    let (result, out) = w.run(&refs(&bug(&[])));
    assert_eq!(result.unwrap(), 0);
    assert!(w.gh.created.borrow().is_empty());
    assert!(
        out.starts_with(
            "Target: slopvac/slopvac\nTitle: [bug]: crash on empty input\n\n### Reporter\n"
        ),
        "{out}"
    );
    assert!(out.contains("### What happened\n\nit crashed"), "{out}");
    assert!(!out.contains("Labels:"), "{out}");
    assert!(
        !out.contains("agent-reported"),
        "the form does not set it:\n{out}"
    );
    assert!(
        out.contains("The issue form adds the labels: bug, needs-triage\n"),
        "{out}"
    );
    assert!(out.contains("Nothing was posted."), "{out}");
    assert!(out.contains("Prefilled issue form: https://github.com/slopvac/slopvac/issues/new?template=bug.yml&title=%5Bbug%5D%3A%20crash%20on%20empty%20input&agent="), "{out}");
    assert!(
        !out.contains("labels="),
        "the form URL must not carry labels:\n{out}"
    );
    assert!(out.contains("&what-happened=it%20crashed"), "{out}");
    assert_eq!(*w.stderr.borrow(), "");
}

#[test]
fn offline_still_previews() {
    let mut w = World::new();
    w.offline = true;
    let (result, out) = w.run(&refs(&bug(&[])));
    assert_eq!(result.unwrap(), 0);
    assert!(out.contains("Nothing was posted."), "{out}");
}

#[test]
fn body_file_holds_exactly_the_previewed_body() {
    let w = World::new();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("body.md");
    let path_text = path.to_str().unwrap();
    let (result, out) = w.run(&refs(&bug(&["--body-file", path_text])));
    assert_eq!(result.unwrap(), 0);
    let body = std::fs::read_to_string(&path).unwrap();
    let preview_body = out
        .split_once("\n\n")
        .unwrap()
        .1
        .split_once("\n\n---\n")
        .unwrap()
        .0;
    assert_eq!(body, preview_body, "{out}");
    assert!(body.starts_with("### Reporter\n"), "{body}");
}

#[test]
fn body_file_replaces_an_existing_file_in_one_step() {
    let w = World::new();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("body.md");
    std::fs::write(&path, "old content that is much longer than anything we would write back, so truncation would show".repeat(200)).unwrap();
    let (result, _) = w.run(&refs(&bug(&["--body-file", path.to_str().unwrap()])));
    assert_eq!(result.unwrap(), 0);
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("old content")
    );
    let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(leftovers.len(), 1, "a temporary file was left behind");
}

#[test]
fn gh_prints_the_exact_command_for_the_body_file_and_does_not_run_it() {
    let w = World::new();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("body.md");
    let path_text = path.to_str().unwrap();
    let (result, out) = w.run(&refs(&bug(&["--body-file", path_text, "--gh"])));
    assert_eq!(result.unwrap(), 0);
    let want = format!(
        "GitHub CLI command:\ngh issue create -R github.com/slopvac/slopvac --title '[bug]: crash on empty input' --body-file {path_text}\n"
    );
    assert!(out.contains(&want), "{out}");
    assert!(w.gh.created.borrow().is_empty());
}

#[test]
fn gh_offline_still_prints_the_command() {
    let mut w = World::new();
    w.offline = true;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.md");
    let (result, out) = w.run(&refs(&bug(&[
        "--body-file",
        path.to_str().unwrap(),
        "--gh",
    ])));
    assert_eq!(result.unwrap(), 0);
    assert!(
        out.contains("gh issue create -R github.com/slopvac/slopvac --title"),
        "{out}"
    );
}

#[test]
fn gh_yes_creates_the_issue_from_the_previewed_title_and_body() {
    let w = World::new();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("body.md");
    let (result, out) = w.run(&refs(&bug(&[
        "--body-file",
        path.to_str().unwrap(),
        "--gh",
        "--yes",
    ])));
    assert_eq!(result.unwrap(), 0);
    let created = w.gh.created.borrow();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].0, "[bug]: crash on empty input");
    assert_eq!(created[0].1, std::fs::read_to_string(&path).unwrap());
    assert!(
        out.ends_with("Created: https://github.com/slopvac/slopvac/issues/77\n"),
        "{out}"
    );
}

#[test]
fn gh_yes_stops_when_gh_is_missing_or_not_logged_in_and_creates_nothing() {
    for (check, code) in [
        (GhError::Missing, "report-gh-missing"),
        (GhError::NotLoggedIn, "report-gh-not-logged-in"),
    ] {
        let mut w = World::new();
        w.gh.check = Err(check);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.md");
        let (result, _) = w.run(&refs(&bug(&[
            "--body-file",
            path.to_str().unwrap(),
            "--gh",
            "--yes",
        ])));
        assert_eq!(result.unwrap_err().code, code);
        assert!(w.gh.created.borrow().is_empty());
    }
}

#[test]
fn gh_yes_offline_is_refused_before_anything_runs() {
    let mut w = World::new();
    w.offline = true;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.md");
    let (result, out) = w.run(&refs(&bug(&[
        "--body-file",
        path.to_str().unwrap(),
        "--gh",
        "--yes",
    ])));
    let err = result.unwrap_err();
    assert_eq!(err.code, "offline-network-needed");
    assert_eq!(out, "");
    assert!(!path.exists());
    assert!(w.gh.created.borrow().is_empty());
}

#[test]
fn credential_shaped_text_warns_on_stderr_and_is_never_refused() {
    let w = World::new();
    let args = bug(&[
        "--field",
        &format!(
            "output=key {} and {}",
            fake("AKIA", "IOSFODNN7EXAMPLE"),
            fake("ghp_", "abcdefghijklmnopqrstuvwxyz0123456789")
        ),
    ]);
    let (result, out) = w.run(&refs(&args));
    assert_eq!(result.unwrap(), 0);
    assert!(!out.contains("ghp_abcdefghij"), "{out}");
    assert!(
        w.stderr.borrow().contains("warning: possible"),
        "{}",
        w.stderr.borrow()
    );
    assert!(
        w.stderr.borrow().contains("in field `output`"),
        "{}",
        w.stderr.borrow()
    );
    assert!(!w.stderr.borrow().contains("ghp_"));
}

#[test]
fn paths_user_and_host_names_never_reach_the_preview_the_filed_issue_or_the_url() {
    let mut w = World::new();
    w.cwd = PathBuf::from("/work/marker-dir-91ab");
    w.env = MapEnv::new(
        &[
            ("HOME", "/Users/marker-home-77"),
            ("USER", "marker-user-55"),
            ("HOSTNAME", "marker-host-33"),
        ],
        false,
    );
    let args = bug(&[
        "--field",
        "output=ran in /work/marker-dir-91ab as marker-user-55 on marker-host-33",
        "--field",
        "config=path = \"/Users/marker-home-77/notes/a.toml\"",
    ]);
    let (result, out) = w.run(&refs(&args));
    result.unwrap();
    let payload = w.filed(&bug(&[
        "--field",
        "output=ran in /work/marker-dir-91ab as marker-user-55 on marker-host-33",
    ]));
    for marker in ["marker-dir", "marker-home", "marker-user", "marker-host"] {
        assert!(
            !out.contains(marker),
            "{marker} reached the preview:\n{out}"
        );
        assert!(
            !payload.contains(marker),
            "{marker} reached the filed issue"
        );
    }
    assert!(out.contains("ran in <path> as <user> on <host>"), "{out}");
}

#[test]
fn document_text_appears_in_the_preview_only_through_include_text() {
    let dir = tempfile::tempdir().unwrap();
    let sample = dir.path().join("sample-text.txt");
    std::fs::write(&sample, "It's not a bug, it's a feature.\n").unwrap();
    let w = World::new();
    let mut args: Vec<String> = [
        "false-positive",
        "--title",
        "flagged",
        "--field",
        "agent=a",
        "--field",
        "model=m",
        "--field",
        "harness=h",
        "--field",
        "latest=Yes",
        "--field",
        "duplicates=true",
        "--field",
        "environment=e",
        "--field",
        "command=c",
        "--field",
        "rule-id=core:x",
        "--field",
        "context=reference normal",
        "--field",
        "finding={}",
        "--field",
        "why=w",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    args.push("--include-text".into());
    args.push(sample.display().to_string());
    let (result, out) = w.run(&refs(&args));
    assert_eq!(result.unwrap(), 0);
    assert!(
        out.contains("### Flagged text\n\n```markdown\nIt's not a bug, it's a feature.\n\n```"),
        "{out}"
    );
    assert!(!out.contains("sample-text"), "the file name leaked:\n{out}");
}

#[test]
fn include_text_refuses_a_missing_file_a_directory_and_oversize_input() {
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("big");
    std::fs::write(&big, vec![b'a'; 256 * 1024 + 1]).unwrap();
    let w = World::new();
    for path in [dir.path().join("nope"), dir.path().to_owned(), big] {
        let mut args = bug(&[]);
        args[0] = "false-positive".into();
        args.push("--include-text".into());
        args.push(path.display().to_string());
        let (result, _) = w.run(&refs(&args));
        let err = result.unwrap_err();
        assert_eq!(err.code, "report-sample-unreadable", "{path:?}");
        assert!(
            !err.message.contains(&path.display().to_string()),
            "{}",
            err.message
        );
    }
}

#[cfg(unix)]
#[test]
fn include_text_does_not_block_on_a_fifo() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("pipe");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let (tx, rx) = std::sync::mpsc::channel();
    let path = fifo.display().to_string();
    std::thread::spawn(move || {
        let w = World::new();
        let mut args = bug(&[]);
        args[0] = "false-positive".into();
        args.push("--include-text".into());
        args.push(path);
        let _ = tx.send(w.run(&refs(&args)).0.map_err(|e| e.code));
    });
    let result = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("blocked on the FIFO");
    assert_eq!(result.unwrap_err(), "report-sample-unreadable");
}

#[test]
fn open_launches_the_printed_url_and_a_failed_browser_does_not_fail_the_report() {
    let mut w = World::new();
    w.opener.fail = true;
    let (result, out) = w.run(&refs(&bug(&["--open"])));
    assert_eq!(result.unwrap(), 0);
    let opened = w.opener.opened.borrow();
    assert_eq!(opened.len(), 1);
    assert!(
        out.contains(&format!("Prefilled issue form: {}\n", opened[0])),
        "{out}"
    );
    assert!(
        opened[0].starts_with("https://github.com/slopvac/slopvac/issues/new?template=bug.yml")
    );
}

#[test]
fn a_long_field_is_left_out_of_the_url_and_named() {
    let w = World::new();
    let huge = "word ".repeat(2000);
    let args = bug(&["--field", &format!("output={huge}")]);
    let (result, out) = w.run(&refs(&args));
    assert_eq!(result.unwrap(), 0);
    let url_line = out
        .lines()
        .find(|l| l.starts_with("Prefilled issue form: "))
        .unwrap();
    assert!(url_line.len() <= 7100, "{}", url_line.len());
    assert!(!url_line.contains("&output="), "{url_line}");
    assert!(out.contains("paste them from the preview: output"), "{out}");
    assert!(
        out.contains(&huge.trim()[..50]),
        "the preview keeps the full body"
    );
}

#[test]
fn prefilled_urls_use_the_form_field_ids() {
    use slopvac_cli::report::{categories, issue};
    let cat = categories()
        .into_iter()
        .find(|c| c.key == "mis-tagged")
        .unwrap();
    let answers: Vec<String> = [
        "agent=a",
        "model=m",
        "harness=h",
        "latest=Yes",
        "duplicates=true",
        "environment=e",
        "command=c",
        "rule-id=core:x",
        "context=g p",
        "finding={}",
        "problem=Wrong rule",
        "problem=Wrong severity",
        "expected=e",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    let scrubber = slopvac_cli::report::scrub::Scrubber::from_env(
        &MapEnv::default(),
        std::path::Path::new("/nonexistent-cwd"),
    );
    let report = issue::build(&cat, "t", &answers, Some("sample"), "3.0.0", &scrubber).unwrap();
    let url = prefilled_url(&cat, &report);
    for id in [
        "rule-id", "context", "input", "finding", "problem", "expected", "agent", "latest",
    ] {
        assert!(
            url.url.contains(&format!("&{id}=")),
            "{id} missing: {}",
            url.url
        );
    }
    assert!(
        url.url
            .contains("&problem=Wrong%20rule%2C%20Wrong%20severity"),
        "{}",
        url.url
    );
    // Checkboxes are the reporter's decision.
    assert!(
        !url.url.contains("&duplicates=") && !url.url.contains("&reporter="),
        "{}",
        url.url
    );
    assert_eq!(url.omitted, Vec::<&str>::new());
}

#[test]
fn tokens_in_the_environment_are_never_read() {
    // Option A: slopvac collects no token. A value in `GH_TOKEN` or a variable named like a
    // secret is ordinary text to the scrubber, so it neither changes the output nor warns.
    let mut w = World::new();
    w.env = MapEnv::new(
        &[
            ("GH_TOKEN", "opaque-env-token-9"),
            ("GITHUB_TOKEN", "legacy-env-token-8"),
            ("DEPLOY_PASSWORD", "marker-pass-phrase-77"),
        ],
        false,
    );
    let args = bug(&[
        "--field",
        "output=pasted opaque-env-token-9 and legacy-env-token-8 and marker-pass-phrase-77",
    ]);
    let (result, out) = w.run(&refs(&args));
    assert_eq!(result.unwrap(), 0);
    assert!(
        out.contains("pasted opaque-env-token-9 and legacy-env-token-8 and marker-pass-phrase-77"),
        "{out}"
    );
    assert_eq!(*w.stderr.borrow(), "");
}

#[test]
fn bearer_credentials_are_cleaned_or_warned_about_whatever_the_spacing() {
    let w = World::new();
    for line in [
        "Authorization: Bearer   abc.def-123 done",
        "Authorization: Bearer\tabc.def-123 done",
        "authorization: BEARER abc.def-123 done",
    ] {
        let args = bug(&["--field", &format!("output={line}")]);
        assert_cleaned_or_warned(&w, &args, &["abc.def-123"]);
    }
}

/// Run `args` in the preview and in a filed issue. Each marker is either absent from what
/// leaves, or the run warned on stderr; a warning never quotes a marker, and nothing is refused.
fn assert_cleaned_or_warned(w: &World, args: &[String], markers: &[&str]) {
    let (result, out) = w.run(&refs(args));
    assert_eq!(result.unwrap(), 0, "{args:?}");
    let warned = w.stderr.borrow().contains("warning: possible");
    let filed = w.filed(args);
    for marker in markers {
        assert!(
            !w.stderr.borrow().contains(marker),
            "a warning quoted {marker}"
        );
        assert!(
            warned || (!out.contains(marker) && !filed.contains(marker)),
            "{marker} left silently:\n{out}"
        );
    }
}

/// Text a report must never carry, with the marker that proves it did not.
const PRIVATE: [(&str, &str); 7] = [
    (
        "opened C:\\Users\\marker-win\\proj\\notes here",
        "marker-win",
    ),
    ("opened D:/work/marker-drive/x here", "marker-drive"),
    (
        "share \\\\marker-fileserver\\data\\x here",
        "marker-fileserver",
    ),
    ("in marker-company/private/subdir here", "marker-company"),
    (
        "cloned https://github.com/marker-org/secret-repo/issues/4 here",
        "marker-org",
    ),
    (
        "remote git@github.com:marker-org2/secret.git here",
        "marker-org2",
    ),
    (
        "see https://gitlab.com/marker-lab/internal here",
        "marker-lab",
    ),
];

#[test]
fn private_paths_and_repositories_never_reach_the_preview_the_url_or_the_filed_issue() {
    let w = World::new();
    for (line, marker) in PRIVATE {
        let args = bug(&["--field", &format!("output={line}")]);
        let (result, out) = w.run(&refs(&args));
        result.unwrap();
        assert!(
            !out.contains(marker),
            "{marker} reached the preview or URL:\n{out}"
        );
        let filed = w.filed(&args);
        assert!(!filed.contains(marker), "{marker} reached the filed issue");
    }
    // The prefilled URL is built from the same cleaned answers.
    let args = bug(&[
        "--field",
        "output=remote git@github.com:marker-org3/secret.git here",
    ]);
    let (_, out) = w.run(&refs(&args));
    assert!(!out.contains("marker-org3"), "{out}");
    assert!(
        out.contains("&output=remote%20git%40github.com%3A%3Corg%3E%2F%3Crepo%3E%20here"),
        "{out}"
    );
}

#[test]
fn the_slopvac_repository_and_ordinary_slashes_stay() {
    let w = World::new();
    let args = bug(&[
        "--field",
        "output=see https://github.com/slopvac/slopvac/issues/12 and read/write, input/output, and/or on/off, TCP/IP",
    ]);
    let (result, out) = w.run(&refs(&args));
    assert_eq!(result.unwrap(), 0);
    assert!(out.contains("see https://github.com/slopvac/slopvac/issues/12 and read/write, input/output, and/or on/off, TCP/IP"), "{out}");
}

#[test]
fn offline_open_prints_the_url_but_launches_no_browser() {
    let mut w = World::new();
    w.offline = true;
    let (result, out) = w.run(&refs(&bug(&["--open"])));
    assert_eq!(result.unwrap(), 0);
    assert!(
        w.opener.opened.borrow().is_empty(),
        "a browser was started offline"
    );
    assert!(
        out.contains("Prefilled issue form: https://github.com/"),
        "{out}"
    );
}

/// A writer that fails like a closed pipe.
struct BrokenOut {
    fail_on_write: bool,
}

impl std::io::Write for BrokenOut {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.fail_on_write {
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
        } else {
            Ok(buf.len())
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
    }
}

#[test]
fn a_preview_that_cannot_be_written_stops_gh() {
    for fail_on_write in [true, false] {
        let w = World::new();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.md");
        let mut out = BrokenOut { fail_on_write };
        let err = w
            .run_into(
                &refs(&bug(&[
                    "--body-file",
                    path.to_str().unwrap(),
                    "--gh",
                    "--yes",
                ])),
                &mut out,
            )
            .unwrap_err();
        assert_eq!(err.code, "output-failed", "write failure: {fail_on_write}");
        assert!(
            w.gh.created.borrow().is_empty(),
            "filed without a visible preview"
        );
    }
}

#[test]
fn include_text_that_is_not_utf8_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let sample = dir.path().join("latin1");
    std::fs::write(&sample, b"caf\xe9 \xff\xfe").unwrap();
    let w = World::new();
    let mut args = bug(&[]);
    args[0] = "false-positive".into();
    args.push("--include-text".into());
    args.push(sample.display().to_string());
    let (result, out) = w.run(&refs(&args));
    let err = result.unwrap_err();
    assert_eq!(err.code, "report-sample-unreadable");
    assert!(err.message.contains("not valid UTF-8"), "{}", err.message);
    assert_eq!(out, "");
}

#[test]
fn a_very_long_title_is_refused_with_a_clear_error() {
    let w = World::new();
    let long = "x".repeat(8000);
    let mut args = bug(&[]);
    args[2] = long;
    let (result, out) = w.run(&refs(&args));
    let err = result.unwrap_err();
    assert_eq!(err.code, "report-invalid");
    assert!(
        err.message.contains("longer than 200 characters"),
        "{}",
        err.message
    );
    assert_eq!(out, "");
    // The longest accepted title keeps the URL inside its budget.
    let mut args = bug(&[]);
    args[2] = "y".repeat(200);
    let (result, out) = w.run(&refs(&args));
    assert_eq!(result.unwrap(), 0);
    let url = out
        .lines()
        .find(|l| l.starts_with("Prefilled issue form: "))
        .unwrap();
    assert!(url.len() < 7100, "{}", url.len());
}

/// Run `text` through the preview with its URL and the filed issue. A marker is cleaned out or
/// the run warned; nothing is refused and no warning quotes a marker.
fn assert_removed(w: &World, text: &str, markers: &[&str]) {
    let args = bug(&["--field", &format!("output={text}")]);
    assert_cleaned_or_warned(w, &args, markers);
}

#[test]
fn authorization_values_and_wrapped_tokens_never_leave_through_any_channel() {
    let w = World::new();
    for (text, markers) in [
        (
            "Authorization: Basic dXNlcjpwYXNzd29yZA==",
            &["dXNlcjpw"][..],
        ),
        ("Authorization: Token abc123SECRETvalue", &["abc123SECRET"]),
        (
            "Authorization:Digest username=\"marker-bob\"",
            &["marker-bob"],
        ),
        ("sent Basic dXNlcjpwYXNzd29yZA== twice", &["dXNlcjpw"]),
        (
            "ghp_ABCDEFGH\nIJKLMNOPQRSTUVWXYZ0123456789",
            &["IJKLMNOPQRSTUVWX"],
        ),
        (
            "ghp_abcdef12 \n 3456789abcdefghijklmnopqrst",
            &["3456789abcdef"],
        ),
    ] {
        assert_removed(&w, text, markers);
    }
}

#[test]
fn an_unknown_field_id_is_not_echoed() {
    let w = World::new();
    let id = fake("ghp_", "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789");
    let id = id.as_str();
    assert_eq!(id.len(), 40);
    let mut args = bug(&[]);
    args.push("--field".into());
    args.push(format!("{id}=x"));
    let (result, out) = w.run(&refs(&args));
    let err = result.unwrap_err();
    assert_eq!(err.code, "report-invalid");
    assert!(
        !err.message.contains("ghp_") && !err.message.contains("ABCDEFGH"),
        "{}",
        err.message
    );
    assert!(
        err.message.contains("unknown field id") && err.message.contains("what-happened"),
        "{}",
        err.message
    );
    assert_eq!(out, "");
}

#[test]
fn percent_encoded_repositories_and_backslash_paths_do_not_escape() {
    let w = World::new();
    for (text, markers) in [
        (
            "https://github.com/marker%2Dorg/secret%2Drepo/issues/4",
            &["marker", "secret"][..],
        ),
        (
            "https://github.com/marker-acme%2Fhidden-repo",
            &["marker-acme", "hidden-repo"],
        ),
        (r"see marker-corp\notes for it", &["marker-corp"]),
    ] {
        assert_removed(&w, text, markers);
    }
}

#[test]
fn urls_that_merely_mention_a_forge_host_are_kept() {
    let w = World::new();
    let line = "see https://example.com/github.com/guide and https://docs.example.com/gitlab.com/x";
    let args = bug(&["--field", &format!("output={line}")]);
    let (result, out) = w.run(&refs(&args));
    assert_eq!(result.unwrap(), 0);
    assert!(out.contains(line), "{out}");
}

#[test]
fn repository_names_in_an_included_sample_are_redacted_in_every_channel() {
    let dir = tempfile::tempdir().unwrap();
    let sample = dir.path().join("s.txt");
    std::fs::write(&sample, "clone https://github.com/marker-org/secret-repo and git@github.com:marker-org2/x.git now\n").unwrap();
    let w = World::new();
    let mut args: Vec<String> = [
        "false-positive",
        "--title",
        "flagged",
        "--field",
        "agent=a",
        "--field",
        "model=m",
        "--field",
        "harness=h",
        "--field",
        "latest=Yes",
        "--field",
        "duplicates=true",
        "--field",
        "environment=e",
        "--field",
        "command=c",
        "--field",
        "rule-id=core:x",
        "--field",
        "context=reference normal",
        "--field",
        "finding={}",
        "--field",
        "why=w",
        "--include-text",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    args.push(sample.display().to_string());
    let (result, preview) = w.run(&refs(&args));
    result.unwrap();
    let posted = w.filed(&args);
    for marker in ["marker-org", "secret-repo"] {
        assert!(
            !preview.contains(marker),
            "{marker} reached the preview or URL:\n{preview}"
        );
        assert!(!posted.contains(marker), "{marker} reached the filed issue");
    }
    assert!(
        preview
            .contains("clone https://github.com/<org>/<repo> and git@github.com:<org>/<repo> now"),
        "{preview}"
    );
}

/// Inputs that got past the scrubber's heuristics in review: the text, the kind the warning names
/// when the scrubber leaves it in, a marker that must never be quoted, and whether the scrubber
/// is expected to leave it for the warning (`true`) or may clean it itself (`false`).
const BYPASSES: [(&str, &str, &str, bool); 14] = [
    (
        "ghp_\nABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
        "credential",
        "ABCDEFGHIJKL",
        true,
    ),
    ("ghp_ab cd ef gh1234", "credential", "gh1234", true),
    (
        "Authorization:\n Basic dXNlcjpwYXNzZm9sZGVk",
        "credential",
        "dXNlcjpwYXNzZm9sZGVk",
        false,
    ),
    (
        "Basic abCDefGHijKLmnOP",
        "credential",
        "abCDefGHijKLmnOP",
        false,
    ),
    ("Basic dXNlcjpwYXNz", "credential", "dXNlcjpwYXNz", false),
    ("Token q7Fz9LmN2xKd", "credential", "q7Fz9LmN2xKd", false),
    (
        "secret abc123SECRETvalueXYZ789longer",
        "credential",
        "SECRETvalueXYZ",
        true,
    ),
    (
        "https://github.com./marker-priv/repo",
        "private repository",
        "marker-priv",
        true,
    ),
    (
        "https://GITHUB.com./marker-priv/repo/issues",
        "private repository",
        "marker-priv",
        true,
    ),
    (
        "//github.com/marker-priv/repo",
        "private repository",
        "marker-priv",
        false,
    ),
    (
        "https://github.com/slopvac/slopvac/../../marker-priv/repo",
        "private repository",
        "marker-priv",
        true,
    ),
    (
        "https://github.com/marker-priv%2Frepo",
        "private repository",
        "marker-priv",
        false,
    ),
    (
        "opened /srv/marker-data/reports/q3 today",
        "private path",
        "marker-data",
        false,
    ),
    (
        "opened ~/marker-notes/todo",
        "private path",
        "marker-notes",
        false,
    ),
];

#[test]
fn every_bypass_is_cleaned_or_warned_about_and_never_refused_or_quoted() {
    let w = World::new();
    for (text, kind, secret, warn_only) in BYPASSES {
        let args = bug(&["--field", &format!("output={text}")]);
        let (result, out) = w.run(&refs(&args));
        assert_eq!(result.unwrap(), 0, "{text:?}");
        let warnings = w.stderr.borrow().clone();
        assert!(
            !warnings.contains(secret),
            "a warning quoted the text: {warnings}"
        );
        let filed = w.filed(&args);
        if warn_only {
            assert!(
                warnings.contains(&format!("possible {kind} in field `output`")),
                "{text:?}: {warnings}"
            );
        } else if out.contains(secret) || filed.contains(secret) {
            assert!(
                warnings.contains(&format!("possible {kind}")),
                "{text:?}: {warnings}"
            );
        }
    }
}

#[test]
fn the_title_is_warned_about() {
    let w = World::new();
    let mut args = bug(&[]);
    args[2] = "see https://github.com./marker-priv/repo".into();
    let (result, _) = w.run(&refs(&args));
    assert_eq!(result.unwrap(), 0);
    assert!(
        w.stderr.borrow().contains("in field `title`"),
        "{}",
        w.stderr.borrow()
    );
    assert!(!w.stderr.borrow().contains("marker-priv"));
}

#[test]
fn a_private_repository_address_in_an_included_sample_is_warned_about() {
    let dir = tempfile::tempdir().unwrap();
    let sample = dir.path().join("s.txt");
    std::fs::write(&sample, "see //github.com/marker-priv/repo for it\n").unwrap();
    let w = World::new();
    let mut args: Vec<String> = [
        "false-positive",
        "--title",
        "flagged",
        "--field",
        "agent=a",
        "--field",
        "model=m",
        "--field",
        "harness=h",
        "--field",
        "latest=Yes",
        "--field",
        "duplicates=true",
        "--field",
        "environment=e",
        "--field",
        "command=c",
        "--field",
        "rule-id=core:x",
        "--field",
        "context=reference normal",
        "--field",
        "finding={}",
        "--field",
        "why=w",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    args.extend(["--include-text".to_owned(), sample.display().to_string()]);
    let (result, _) = w.run(&refs(&args));
    assert_eq!(result.unwrap(), 0);
    let warnings = w.stderr.borrow().clone();
    assert!(
        warnings.contains("possible private repository in field `input`"),
        "{warnings}"
    );
    assert!(!warnings.contains("marker-priv"), "{warnings}");
}

#[test]
fn ordinary_prose_slugs_and_placeholders_draw_no_warning() {
    let w = World::new();
    for text in [
        "basic internationalization support",
        "Basic authentication works",
        "Token count is 5 and the token budget",
        "read/write, input/output, and/or on/off, TCP/IP",
        "model claude-3-5-sonnet-20241022 on x86_64-unknown-linux-gnu",
        "see https://github.com/slopvac/slopvac/issues/12 and github.com for more",
        "https://example.com/github.com/guide",
        "slash command /model and version 3.0.0",
    ] {
        let args = bug(&["--field", &format!("output={text}")]);
        let (result, out) = w.run(&refs(&args));
        assert_eq!(result.unwrap(), 0, "{text:?}");
        assert!(out.contains(text), "{text:?} changed:\n{out}");
        assert_eq!(*w.stderr.borrow(), "", "{text:?} drew a warning");
    }
}

/// Two wrapped-token chunk patterns from review: 8/6/6/6/6/4 mixed case, and 8 plus 4-char chunks.
const CHUNKED: [&str; 2] = [
    "ghp_ABcdEFgh ijKLmn OPqrST UVwxYZ 012345 6789",
    "ghp_ABcdEFgh 0123 4567 89ab cdef ghij klmn opqr stuv wxyz",
];

#[test]
fn wrapped_chunked_tokens_draw_a_warning_and_are_not_quoted() {
    let w = World::new();
    for chunked in CHUNKED {
        for field_text in [chunked.to_owned(), chunked.replace(' ', "\n")] {
            let args = bug(&["--field", &format!("output={field_text}")]);
            let (result, _) = w.run(&refs(&args));
            assert_eq!(result.unwrap(), 0, "{field_text:?}");
            let warnings = w.stderr.borrow().clone();
            assert!(
                warnings.contains("warning: possible credential in field `output`"),
                "{warnings}"
            );
            assert!(!warnings.contains("ABcdEF"), "{warnings}");
        }
    }
}

#[test]
fn scheme_words_followed_by_an_unfamiliar_token_warn_and_prose_does_not() {
    let w = World::new();
    for text in [
        "basic abcdefghijklmnop",
        "bearer\nqwrtpsdfghjklzxcv",
        "Token qwrtpsdfghjk",
        "Digest\n\nzxcvbnmqwrtp",
    ] {
        let args = bug(&["--field", &format!("output={text}")]);
        let (result, _) = w.run(&refs(&args));
        assert_eq!(result.unwrap(), 0, "{text:?}");
        assert!(w.stderr.borrow().contains("warning: possible"), "{text:?}");
    }
    for text in [
        "Basic authentication.",
        "basic internationalization support",
        "Bearer tokens are short-lived",
        "Token expiry, then refresh!",
    ] {
        let args = bug(&["--field", &format!("output={text}")]);
        let (result, _) = w.run(&refs(&args));
        assert_eq!(result.unwrap(), 0, "{text:?}");
        assert_eq!(*w.stderr.borrow(), "", "{text:?}");
    }
}

#[test]
fn backslash_traversal_cannot_reach_a_private_repository() {
    let w = World::new();
    for text in [
        r"https://github.com/slopvac/slopvac\..\..\marker-priv\repo",
        r"https://github.com/slopvac\slopvac/../../marker-priv/repo",
    ] {
        assert_removed(&w, text, &["marker-priv"]);
    }
}

#[cfg(unix)]
#[test]
fn a_non_utf8_environment_variable_does_not_crash_or_print() {
    use std::os::unix::ffi::OsStrExt;
    let args = bug(&[]);
    let assert = slopvac()
        .env(
            std::ffi::OsStr::from_bytes(b"MARKER_BAD_VAR"),
            std::ffi::OsStr::from_bytes(b"\xff\xfe-secret-value"),
        )
        .arg("report")
        .args(&args)
        .assert()
        .code(0);
    let output = assert.get_output();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !all.contains("secret-value") && !all.contains("panicked"),
        "{all}"
    );
}

fn slopvac() -> Command {
    let mut cmd = Command::cargo_bin("slopvac").expect("binary builds");
    let empty = std::env::temp_dir().join("slopvac-report-no-gh-on-path");
    std::fs::create_dir_all(&empty).unwrap();
    for var in [
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd.env_remove(var);
    }
    // No `gh` on the path.
    cmd.env("PATH", &empty).env("NO_COLOR", "1");
    cmd
}

#[test]
fn the_binary_previews_a_complete_report() {
    let args = bug(&[]);
    slopvac()
        .arg("report")
        .args(&args)
        .assert()
        .code(0)
        .stdout(predicate::str::contains(
            "Title: [bug]: crash on empty input",
        ))
        .stdout(predicate::str::contains("Nothing was posted."));
}

#[test]
fn the_binary_never_quotes_an_unreadable_sample_in_any_channel() {
    const SENTINEL: &str = "SENTINEL-sample-contents-7c1e";
    let oversize = usize::try_from(slopvac_cli::report::issue::MAX_SAMPLE_BYTES).unwrap() + 1;
    let mut not_utf8 = SENTINEL.as_bytes().to_vec();
    not_utf8.extend_from_slice(b" \xff\xfe tail");
    let mut too_big = SENTINEL.as_bytes().to_vec();
    too_big.resize(oversize, b'x');
    for (name, bytes) in [("not-utf8", not_utf8), ("too-big", too_big)] {
        let dir = tempfile::tempdir().unwrap();
        let sample = dir.path().join("sample.txt");
        std::fs::write(&sample, bytes).unwrap();
        let mut args = bug(&[]);
        args.extend(["--include-text".to_owned(), sample.display().to_string()]);
        let assert = slopvac()
            .arg("report")
            .args(&args)
            .assert()
            .code(2)
            .stderr(predicate::str::contains("error[report-sample-unreadable]"));
        let output = assert.get_output();
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!all.contains(SENTINEL), "{name}: {all}");
    }
}

#[test]
fn the_binary_lists_missing_fields() {
    slopvac()
        .args(["report", "bug", "--title", "x"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[report-incomplete]"))
        .stderr(predicate::str::contains("what-happened"));
}

#[test]
fn the_binary_gh_yes_without_gh_on_the_path_exits_two_and_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.md");
    let args = bug(&["--body-file", path.to_str().unwrap(), "--gh", "--yes"]);
    slopvac()
        .arg("report")
        .args(&args)
        .assert()
        .code(2)
        .stdout(predicate::str::contains(
            "gh issue create -R github.com/slopvac/slopvac --title",
        ))
        .stderr(predicate::str::contains("error[report-gh-missing]"));
}

#[test]
fn the_binary_gh_yes_offline_exits_two_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b.md");
    let args = bug(&["--body-file", path.to_str().unwrap(), "--gh", "--yes"]);
    slopvac()
        .arg("--offline")
        .arg("report")
        .args(&args)
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains(
            "error[offline-network-needed]: offline: report --gh --yes needs the network",
        ));
    assert!(!path.exists());
}

#[test]
fn the_binary_gh_without_a_body_file_is_a_usage_error() {
    slopvac()
        .arg("report")
        .args(bug(&["--gh"]))
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[usage]"));
}

/// A fake `gh` on `PATH`: `auth status` succeeds (or fails), `issue create` records its
/// arguments, one per line, and prints an issue URL.
#[cfg(unix)]
fn fake_gh(dir: &std::path::Path, auth_ok: bool) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let log = dir.join("argv.log");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = auth ]; then exit {}; fi\nfor a in \"$@\"; do printf '%s\\n' \"$a\" >> '{}'; done\necho https://github.com/slopvac/slopvac/issues/5\n",
        i32::from(!auth_ok),
        log.display()
    );
    let bin = dir.join("gh");
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    log
}

#[cfg(unix)]
#[test]
fn the_binary_gh_yes_runs_gh_with_the_exact_arguments() {
    let bin_dir = tempfile::tempdir().unwrap();
    let log = fake_gh(bin_dir.path(), true);
    let work = tempfile::tempdir().unwrap();
    let body = work.path().join("b.md");
    let args = bug(&["--body-file", body.to_str().unwrap(), "--gh", "--yes"]);
    slopvac()
        .env("PATH", bin_dir.path())
        .arg("report")
        .args(&args)
        .assert()
        .code(0)
        .stdout(predicate::str::ends_with(
            "Created: https://github.com/slopvac/slopvac/issues/5\n",
        ));
    let recorded = std::fs::read_to_string(log).unwrap();
    assert_eq!(
        recorded,
        format!(
            "issue\ncreate\n-R\ngithub.com/slopvac/slopvac\n--title\n[bug]: crash on empty input\n--body-file\n{}\n",
            body.display()
        )
    );
    assert!(
        std::fs::read_to_string(&body)
            .unwrap()
            .starts_with("### Reporter\n")
    );
}

#[cfg(unix)]
#[test]
fn the_binary_gh_yes_stops_when_gh_is_not_logged_in() {
    let bin_dir = tempfile::tempdir().unwrap();
    let log = fake_gh(bin_dir.path(), false);
    let work = tempfile::tempdir().unwrap();
    let body = work.path().join("b.md");
    let args = bug(&["--body-file", body.to_str().unwrap(), "--gh", "--yes"]);
    slopvac()
        .env("PATH", bin_dir.path())
        .arg("report")
        .args(&args)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[report-gh-not-logged-in]"));
    assert!(!log.exists(), "gh issue create ran");
}

#[cfg(unix)]
#[test]
fn the_repository_cannot_be_changed_by_flags_or_config() {
    // There is no flag for it, and a config key of the same name is not read.
    slopvac()
        .args(["report", "bug", "--title", "x", "--repo", "other/repo"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[usage]"));
    let bin_dir = tempfile::tempdir().unwrap();
    let log = fake_gh(bin_dir.path(), true);
    let work = tempfile::tempdir().unwrap();
    std::fs::write(
        work.path().join("slopvac.toml"),
        "schema_version = 3\nrepository = \"other/repo\"\n",
    )
    .unwrap();
    let body = work.path().join("b.md");
    let args = bug(&["--body-file", body.to_str().unwrap(), "--gh", "--yes"]);
    slopvac()
        .current_dir(work.path())
        .env("PATH", bin_dir.path())
        .arg("report")
        .args(&args)
        .assert()
        .code(0);
    let recorded = std::fs::read_to_string(log).unwrap();
    assert!(
        recorded.contains("-R\ngithub.com/slopvac/slopvac\n"),
        "{recorded}"
    );
    assert!(!recorded.contains("other/repo"));
}

#[test]
fn an_unknown_category_is_a_usage_error() {
    slopvac()
        .args(["report", "nonsense", "--title", "x"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("error[usage]"));
}

#[test]
fn clap_errors_for_report_do_not_echo_what_was_typed() {
    let secret = fake("ghp_", "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789");
    let secret = secret.as_str();
    for args in [
        vec![
            "report".to_owned(),
            secret.to_owned(),
            "--title".into(),
            "x".into(),
        ],
        vec![
            "report".to_owned(),
            "bug".into(),
            "--title".into(),
            "x".into(),
            format!("--{secret}"),
        ],
        vec!["report".to_owned(), "bug".into(), format!("extra-{secret}")],
    ] {
        let assert = slopvac().args(&args).assert().code(2);
        let output = assert.get_output();
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !all.contains("ghp_") && !all.contains("ABCDEFGH"),
            "{args:?} echoed:\n{all}"
        );
        assert!(all.contains("error[usage]"), "{all}");
    }
}

#[test]
fn control_characters_are_refused_in_the_title_and_the_fields_without_echo() {
    let w = World::new();
    let esc = "\u{1b}]52;c;ZXZpbA==\u{7}\u{1b}[2J";
    let mut args = bug(&[]);
    args[2] = format!("crash{esc}");
    let (result, out) = w.run(&refs(&args));
    let err = result.unwrap_err();
    assert_eq!(err.code, "report-invalid");
    assert_eq!(out, "");
    assert!(!err.message.contains('\u{1b}'), "{}", err.message);
    for field in ["output", "what-happened"] {
        let args = bug(&["--field", &format!("{field}=a{esc}b")]);
        let (result, out) = w.run(&refs(&args));
        assert_eq!(result.unwrap_err().code, "report-invalid", "{field}");
        assert_eq!(out, "");
    }
    // A C1 control (CSI as one char) is refused too.
    let args = bug(&["--field", "output=a\u{9b}2Jb"]);
    assert_eq!(w.run(&refs(&args)).0.unwrap_err().code, "report-invalid");
}

#[test]
fn tabs_newlines_and_crlf_in_a_body_are_kept() {
    let w = World::new();
    let args = bug(&["--field", "output=a\tb\r\nc\nd"]);
    let (result, out) = w.run(&refs(&args));
    assert_eq!(result.unwrap(), 0);
    assert!(out.contains("a\tb\nc\nd"), "{out}");
    assert!(!out.contains('\r'));
}

#[test]
fn a_sample_with_an_escape_sequence_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let sample = dir.path().join("s.txt");
    std::fs::write(&sample, "ok \u{1b}]52;c;ZXZpbA==\u{7}\n").unwrap();
    let w = World::new();
    let mut args = bug(&[]);
    args[0] = "false-positive".into();
    args.extend(["--include-text".to_owned(), sample.display().to_string()]);
    let (result, out) = w.run(&refs(&args));
    assert_eq!(result.unwrap_err().code, "report-invalid");
    assert_eq!(out, "");
}

#[test]
fn a_body_file_path_with_a_control_character_cannot_be_printed_in_the_command() {
    let w = World::new();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("b\u{1b}[2J.md");
    let (result, out) = w.run(&refs(&bug(&[
        "--body-file",
        path.to_str().unwrap(),
        "--gh",
    ])));
    assert_eq!(result.unwrap_err().code, "report-body-file");
    assert!(!out.contains('\u{1b}'), "{out:?}");
}

#[cfg(unix)]
#[test]
fn the_binary_gh_yes_failure_exits_two_with_no_success_message() {
    use std::os::unix::fs::PermissionsExt;
    let bin_dir = tempfile::tempdir().unwrap();
    let gh = bin_dir.path().join("gh");
    std::fs::write(
        &gh,
        "#!/bin/sh\nif [ \"$1\" = auth ]; then exit 0; fi\necho https://github.com/slopvac/slopvac/issues/9\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let work = tempfile::tempdir().unwrap();
    let body = work.path().join("b.md");
    let args = bug(&["--body-file", body.to_str().unwrap(), "--gh", "--yes"]);
    slopvac()
        .env("PATH", bin_dir.path())
        .arg("report")
        .args(&args)
        .assert()
        .code(2)
        .stdout(predicate::str::contains("Created:").not())
        .stderr(predicate::str::contains("error[report-gh-failed]"));
}

#[cfg(unix)]
#[test]
fn gh_sees_the_pinned_host_and_repository_whatever_the_caller_exports() {
    use std::os::unix::fs::PermissionsExt;
    let bin_dir = tempfile::tempdir().unwrap();
    let log = bin_dir.path().join("env.log");
    let gh = bin_dir.path().join("gh");
    std::fs::write(
        &gh,
        format!(
            "#!/bin/sh\nprintf 'GH_HOST=%s GH_REPO=%s ARGS=%s\\n' \"${{GH_HOST-unset}}\" \"${{GH_REPO-unset}}\" \"$*\" >> '{}'\necho https://github.com/slopvac/slopvac/issues/9\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let work = tempfile::tempdir().unwrap();
    let body = work.path().join("b.md");
    let args = bug(&["--body-file", body.to_str().unwrap(), "--gh", "--yes"]);
    slopvac()
        .env("PATH", bin_dir.path())
        .env("GH_HOST", "evil.example")
        .env("GH_REPO", "evil.example/other/repo")
        .arg("report")
        .args(&args)
        .assert()
        .code(0);
    let recorded = std::fs::read_to_string(&log).unwrap();
    let lines: Vec<&str> = recorded.lines().collect();
    assert_eq!(lines.len(), 2, "{recorded}");
    assert_eq!(
        lines[0],
        "GH_HOST=unset GH_REPO=unset ARGS=auth status --hostname github.com"
    );
    assert!(
        lines[1].starts_with(
            "GH_HOST=unset GH_REPO=unset ARGS=issue create -R github.com/slopvac/slopvac --title"
        ),
        "{}",
        lines[1]
    );
    assert!(!recorded.contains("evil.example"));
}

#[test]
fn a_body_file_named_dash_is_refused_because_gh_would_read_stdin() {
    let w = World::new();
    let (result, out) = w.run(&refs(&bug(&["--body-file", "-", "--gh"])));
    let err = result.unwrap_err();
    assert_eq!(err.code, "report-body-file");
    assert!(err.message.contains("./-"), "{}", err.message);
    assert_eq!(out, "");
    assert!(w.gh.created.borrow().is_empty());
}

#[test]
fn a_dash_name_with_a_directory_part_is_an_ordinary_file() {
    let w = World::new();
    let dir = tempfile::tempdir().unwrap();
    let ok = dir.path().join("-");
    let (result, _) = w.run(&refs(&bug(&["--body-file", ok.to_str().unwrap(), "--gh"])));
    assert_eq!(result.unwrap(), 0);
    assert!(ok.is_file());
}

/// `Deps::cwd` only feeds the redaction list, so the library harness cannot move the working
/// directory; this runs the binary with the child's working directory set instead.
#[test]
fn dot_slash_dash_writes_a_file_named_dash_in_the_working_directory() {
    let work = tempfile::tempdir().unwrap();
    let args = bug(&["--body-file", "./-", "--gh"]);
    let assert = slopvac()
        .current_dir(work.path())
        .arg("report")
        .args(&args)
        .assert()
        .code(0);
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let body = std::fs::read_to_string(work.path().join("-")).unwrap();
    let preview_body = out
        .split_once("\n\n")
        .unwrap()
        .1
        .split_once("\n\n---\n")
        .unwrap()
        .0;
    assert_eq!(body, preview_body, "{out}");
    assert!(
        out.contains("--title '[bug]: crash on empty input' --body-file ./-\n"),
        "{out}"
    );
}
