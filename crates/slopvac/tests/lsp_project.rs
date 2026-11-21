//! `slopvac lsp` with a real project config: per-document `[[overrides]]` and `[[genres]]`
//! settings, and a reload when a config file is saved or changes on disk.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

use serde_json::{Value, json};

const SLOPPY: &str = "# Title\n\nWe leverage a robust, seamless solution to delve into the data.\n";
const HISTORY: &str = "# Notes\n\nWe refactored the parser.\n";
const LEXICON: &str = "core:prose-inflation.slop-lexicon";
const HISTORY_RULE: &str = "core:docs-discipline.history-internal-reference";
const WAIT: Duration = Duration::from_secs(20);

const OVERRIDE_OFF: &str = "schema_version = 3\n\n[[overrides]]\nfiles = [\"docs/**\"]\n\n[overrides.rules]\n\"prose-inflation.slop-lexicon\" = \"off\"\n";

/// A running `slopvac lsp` in a project directory.
struct Session {
    child: Child,
    stdin: ChildStdin,
    frames: Receiver<Value>,
    next_id: u64,
}

impl Session {
    fn start(dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_slopvac"))
            .arg("lsp")
            .current_dir(dir)
            .env_remove("SLOPVAC_LOG")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("slopvac lsp starts");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let (tx, frames) = channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut length = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let line = line.trim_end();
                    if line.is_empty() {
                        break;
                    }
                    if let Some(n) = line.strip_prefix("Content-Length: ") {
                        length = n.parse::<usize>().ok();
                    }
                }
                let mut body = vec![0; length.expect("a length")];
                if reader.read_exact(&mut body).is_err() {
                    return;
                }
                let value = serde_json::from_slice(&body).expect("a JSON body");
                if tx.send(value).is_err() {
                    return;
                }
            }
        });
        let mut session = Self {
            child,
            stdin,
            frames,
            next_id: 0,
        };
        session.request(
            "initialize",
            &json!({"processId": null, "rootUri": null, "capabilities": {}}),
        );
        session.notify("initialized", &json!({}));
        session
    }

    fn send(&mut self, body: &Value) {
        let text = body.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{text}", text.len()).expect("write frame");
        self.stdin.flush().expect("flush");
    }

    fn notify(&mut self, method: &str, params: &Value) {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn request(&mut self, method: &str, params: &Value) {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        self.wait_for(|m| m["id"] == id);
    }

    /// The next frame that satisfies `pred`; earlier frames are dropped.
    fn wait_for(&self, pred: impl Fn(&Value) -> bool) -> Value {
        loop {
            match self.frames.recv_timeout(WAIT) {
                Ok(frame) if pred(&frame) => return frame,
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => panic!("no matching message in {WAIT:?}"),
                Err(RecvTimeoutError::Disconnected) => panic!("the server closed its output"),
            }
        }
    }

    fn open(&mut self, dir: &Path, rel: &str, text: &str) {
        let uri = uri_of(dir, rel);
        self.notify(
            "textDocument/didOpen",
            &json!({"textDocument": {"uri": uri, "languageId": "markdown", "version": 1, "text": text}}),
        );
    }

    /// Diagnostic codes of the next `publishDiagnostics` for the document at `rel`.
    fn codes(&self, dir: &Path, rel: &str) -> Vec<String> {
        let uri = uri_of(dir, rel);
        let published = self.wait_for(|m| {
            m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri
        });
        published["params"]["diagnostics"]
            .as_array()
            .expect("diagnostics")
            .iter()
            .map(|d| d["code"].as_str().expect("code").to_owned())
            .collect()
    }

    fn finish(mut self) {
        self.request("shutdown", &Value::Null);
        self.notify("exit", &Value::Null);
        assert!(self.child.wait().expect("exit").success());
    }
}

fn uri_of(dir: &Path, rel: &str) -> String {
    format!("file://{}", dir.join(rel).display())
}

/// A canonical project directory, so the document paths match what the server resolves.
fn project(config: &str, files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("slopvac.toml"), config).expect("config");
    for (rel, text) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, text).expect("document");
    }
    dir
}

#[test]
fn an_override_block_turns_a_rule_off_only_for_the_documents_it_matches() {
    let dir = project(OVERRIDE_OFF, &[]);
    let root = dir.path().canonicalize().expect("canonical");
    let mut session = Session::start(&root);
    session.open(&root, "docs/a.md", SLOPPY);
    session.open(&root, "top.md", SLOPPY);
    let inside = session.codes(&root, "docs/a.md");
    let outside = session.codes(&root, "top.md");
    assert!(!inside.iter().any(|c| c == LEXICON), "{inside:?}");
    assert!(outside.iter().any(|c| c == LEXICON), "{outside:?}");
    session.finish();
}

#[test]
fn a_genres_block_gives_the_documents_it_matches_the_genre_level() {
    let config =
        "schema_version = 3\n\n[[genres]]\nfiles = [\"log/**\"]\ngenre = \"change-comms\"\n";
    let dir = project(config, &[]);
    let root = dir.path().canonicalize().expect("canonical");
    let mut session = Session::start(&root);
    session.open(&root, "log/c.md", HISTORY);
    session.open(&root, "reference/e.md", HISTORY);
    let assigned = session.codes(&root, "log/c.md");
    let unassigned = session.codes(&root, "reference/e.md");
    assert!(!assigned.iter().any(|c| c == HISTORY_RULE), "{assigned:?}");
    assert!(
        unassigned.iter().any(|c| c == HISTORY_RULE),
        "{unassigned:?}"
    );
    session.finish();
}

#[test]
fn saving_the_edited_config_lints_an_open_document_under_the_new_settings() {
    let dir = project("schema_version = 3\n", &[]);
    let root = dir.path().canonicalize().expect("canonical");
    let mut session = Session::start(&root);
    session.open(&root, "docs/a.md", SLOPPY);
    assert!(
        session
            .codes(&root, "docs/a.md")
            .iter()
            .any(|c| c == LEXICON)
    );
    fs::write(root.join("slopvac.toml"), OVERRIDE_OFF).expect("edit config");
    session.notify(
        "textDocument/didSave",
        &json!({"textDocument": {"uri": uri_of(&root, "slopvac.toml")}}),
    );
    let after = session.codes(&root, "docs/a.md");
    assert!(!after.iter().any(|c| c == LEXICON), "{after:?}");
    session.finish();
}

#[test]
fn a_watched_change_of_an_extended_file_lints_an_open_document_under_the_new_settings() {
    let dir = project("schema_version = 3\nextends = [\"base.toml\"]\n", &[]);
    let root = dir.path().canonicalize().expect("canonical");
    fs::write(root.join("base.toml"), "schema_version = 3\n").expect("base");
    let mut session = Session::start(&root);
    session.open(&root, "a.md", SLOPPY);
    assert!(session.codes(&root, "a.md").iter().any(|c| c == LEXICON));
    fs::write(
        root.join("base.toml"),
        "schema_version = 3\n\n[rules]\n\"prose-inflation.slop-lexicon\" = \"off\"\n",
    )
    .expect("edit base");
    session.notify(
        "workspace/didChangeWatchedFiles",
        &json!({"changes": [{"uri": uri_of(&root, "base.toml"), "type": 2}]}),
    );
    let after = session.codes(&root, "a.md");
    assert!(!after.iter().any(|c| c == LEXICON), "{after:?}");
    session.finish();
}

#[test]
fn a_reload_shows_an_unknown_rule_in_the_new_config() {
    let dir = project("schema_version = 3\n", &[]);
    let root = dir.path().canonicalize().expect("canonical");
    let mut session = Session::start(&root);
    fs::write(
        root.join("slopvac.toml"),
        "schema_version = 3\n\n[rules]\n\"core:orwell.no-such-rule\" = \"off\"\n",
    )
    .expect("edit config");
    session.notify(
        "textDocument/didSave",
        &json!({"textDocument": {"uri": uri_of(&root, "slopvac.toml")}}),
    );
    let shown = session.wait_for(|m| m["method"] == "window/showMessage");
    let message = shown["params"]["message"].as_str().expect("message");
    assert!(
        message.contains("unknown rule core:orwell.no-such-rule"),
        "{message}"
    );
    session.finish();
}

#[test]
fn a_config_that_breaks_is_shown_on_the_file_and_a_fixed_one_restores_the_findings() {
    let dir = project(OVERRIDE_OFF, &[]);
    let root = dir.path().canonicalize().expect("canonical");
    let mut session = Session::start(&root);
    session.open(&root, "top.md", SLOPPY);
    assert!(session.codes(&root, "top.md").iter().any(|c| c == LEXICON));
    fs::write(
        root.join("slopvac.toml"),
        "schema_version = 3\nprofile = [\n",
    )
    .expect("break");
    let save = json!({"textDocument": {"uri": uri_of(&root, "slopvac.toml")}});
    session.notify("textDocument/didSave", &save);
    let config_uri = uri_of(&root, "slopvac.toml");
    let on_config = session.wait_for(|m| {
        m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == config_uri
    });
    let shown = &on_config["params"]["diagnostics"];
    assert_eq!(shown.as_array().map(Vec::len), Some(1), "{shown}");
    assert_eq!(shown[0]["severity"], 1);
    assert!(
        shown[0]["code"]
            .as_str()
            .is_some_and(|c| c.starts_with("config-")),
        "{shown}"
    );
    let incomplete = session.codes(&root, "top.md");
    assert_eq!(incomplete, ["lsp-lint-incomplete"]);
    fs::write(root.join("slopvac.toml"), "schema_version = 3\n").expect("fix");
    session.notify("textDocument/didSave", &save);
    let cleared = session.wait_for(|m| {
        m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == config_uri
    });
    assert_eq!(cleared["params"]["diagnostics"], json!([]));
    assert!(session.codes(&root, "top.md").iter().any(|c| c == LEXICON));
    session.finish();
}

const LEXICON_OFF: &str =
    "schema_version = 3\n\n[rules]\n\"prose-inflation.slop-lexicon\" = \"off\"\n";

/// Tell the server that the file at `path` changed on disk.
fn changed(session: &mut Session, uri: &str) {
    session.notify(
        "workspace/didChangeWatchedFiles",
        &json!({"changes": [{"uri": uri, "type": 2}]}),
    );
}

fn diagnostics_on(session: &Session, uri: &str) -> Value {
    session
        .wait_for(|m| m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri)
        ["params"]["diagnostics"]
        .clone()
}

#[test]
fn a_buffer_outside_the_project_with_no_config_on_its_path_gets_the_core_defaults() {
    let dir = project(LEXICON_OFF, &[]);
    let root = dir.path().canonicalize().expect("canonical");
    let elsewhere = tempfile::tempdir().expect("tempdir");
    let other = elsewhere.path().canonicalize().expect("canonical");
    let mut session = Session::start(&root);
    session.open(&other, "a.md", SLOPPY);
    let codes = session.codes(&other, "a.md");
    assert!(codes.iter().any(|c| c == LEXICON), "{codes:?}");
    session.finish();
}

#[test]
fn a_config_that_fails_in_an_extended_file_recovers_when_only_that_file_is_fixed() {
    let dir = project("schema_version = 3\nextends = [\"base.toml\"]\n", &[]);
    let root = dir.path().canonicalize().expect("canonical");
    fs::write(root.join("base.toml"), LEXICON_OFF).expect("base");
    let mut session = Session::start(&root);
    session.open(&root, "a.md", SLOPPY);
    assert!(!session.codes(&root, "a.md").iter().any(|c| c == LEXICON));
    let base = uri_of(&root, "base.toml");
    fs::write(root.join("base.toml"), "schema_version = 3\nprofile = [\n").expect("break");
    changed(&mut session, &base);
    let shown = diagnostics_on(&session, &base);
    assert_eq!(shown.as_array().map(Vec::len), Some(1), "{shown}");
    assert_eq!(session.codes(&root, "a.md"), ["lsp-lint-incomplete"]);
    fs::write(root.join("base.toml"), "schema_version = 3\n").expect("fix");
    changed(&mut session, &base);
    assert_eq!(diagnostics_on(&session, &base), json!([]));
    assert!(session.codes(&root, "a.md").iter().any(|c| c == LEXICON));
    session.finish();
}

#[test]
fn an_extended_file_outside_the_project_is_watched_and_shows_its_own_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let top = dir.path().canonicalize().expect("canonical");
    fs::create_dir_all(top.join("shared")).expect("mkdir");
    fs::create_dir_all(top.join("proj")).expect("mkdir");
    fs::write(top.join("shared/base.toml"), LEXICON_OFF).expect("base");
    fs::write(
        top.join("proj/slopvac.toml"),
        "schema_version = 3\nextends = [\"../shared/base.toml\"]\n",
    )
    .expect("config");
    let root = top.join("proj");
    let mut session = Session::start(&root);
    session.open(&root, "a.md", SLOPPY);
    assert!(!session.codes(&root, "a.md").iter().any(|c| c == LEXICON));
    let base = uri_of(&top, "shared/base.toml");
    fs::write(
        top.join("shared/base.toml"),
        "schema_version = 3\nprofile = [\n",
    )
    .expect("break");
    // The client may name the file with a `..` in it.
    changed(&mut session, &uri_of(&root, "../shared/base.toml"));
    let shown = diagnostics_on(&session, &base);
    assert_eq!(shown.as_array().map(Vec::len), Some(1), "{shown}");
    fs::write(top.join("shared/base.toml"), "schema_version = 3\n").expect("fix");
    changed(&mut session, &base);
    assert_eq!(diagnostics_on(&session, &base), json!([]));
    session.finish();
}

#[test]
fn a_change_to_a_file_that_a_nested_config_extends_lints_the_open_document_again() {
    let dir = project("schema_version = 3\n", &[]);
    let root = dir.path().canonicalize().expect("canonical");
    fs::create_dir_all(root.join("shared")).expect("mkdir");
    fs::create_dir_all(root.join("sub")).expect("mkdir");
    fs::write(root.join("shared/nested.toml"), LEXICON_OFF).expect("base");
    fs::write(
        root.join("sub/slopvac.toml"),
        "schema_version = 3\nextends = [\"../shared/nested.toml\"]\n",
    )
    .expect("nested config");
    let mut session = Session::start(&root);
    session.open(&root, "sub/a.md", SLOPPY);
    assert!(
        !session
            .codes(&root, "sub/a.md")
            .iter()
            .any(|c| c == LEXICON)
    );
    fs::write(root.join("shared/nested.toml"), "schema_version = 3\n").expect("edit");
    changed(&mut session, &uri_of(&root, "shared/nested.toml"));
    assert!(
        session
            .codes(&root, "sub/a.md")
            .iter()
            .any(|c| c == LEXICON)
    );
    session.finish();
}

#[test]
fn a_nested_config_is_checked_as_lint_checks_it() {
    let dir = project("schema_version = 3\n", &[]);
    let root = dir.path().canonicalize().expect("canonical");
    fs::create_dir_all(root.join("bad")).expect("mkdir");
    fs::create_dir_all(root.join("warn")).expect("mkdir");
    fs::write(
        root.join("bad/slopvac.toml"),
        "schema_version = 3\n\n[categories]\nno-such-category = \"off\"\n",
    )
    .expect("config");
    fs::write(
        root.join("warn/slopvac.toml"),
        "schema_version = 3\n\n[rules]\n\"core:orwell.no-such-rule\" = \"off\"\n",
    )
    .expect("config");
    let mut session = Session::start(&root);
    session.open(&root, "bad/a.md", SLOPPY);
    let published = diagnostics_on(&session, &uri_of(&root, "bad/a.md"));
    assert_eq!(published[0]["code"], "lsp-lint-incomplete");
    let message = published[0]["message"].as_str().expect("message");
    assert!(message.contains("no-such-category"), "{message}");
    session.open(&root, "warn/a.md", SLOPPY);
    let shown = session.wait_for(|m| m["method"] == "window/showMessage");
    let message = shown["params"]["message"].as_str().expect("message");
    assert!(
        message.contains("unknown rule core:orwell.no-such-rule"),
        "{message}"
    );
    session.finish();
}

#[test]
fn a_symlinked_document_gets_the_settings_of_the_file_it_links_to() {
    let dir = project(OVERRIDE_OFF, &[("docs/real.md", SLOPPY)]);
    let root = dir.path().canonicalize().expect("canonical");
    std::os::unix::fs::symlink(root.join("docs/real.md"), root.join("alias.md")).expect("link");
    let mut session = Session::start(&root);
    session.open(&root, "alias.md", SLOPPY);
    // `slopvac lint alias.md` resolves the link first, so the `docs/**` block applies.
    let codes = session.codes(&root, "alias.md");
    assert!(!codes.iter().any(|c| c == LEXICON), "{codes:?}");
    session.finish();
}

#[test]
fn a_change_to_the_target_of_a_symlinked_extended_file_lints_the_open_document_again() {
    let dir = project(
        "schema_version = 3\nextends = [\"link.toml\"]\n",
        &[("real/base.toml", LEXICON_OFF)],
    );
    let root = dir.path().canonicalize().expect("canonical");
    std::os::unix::fs::symlink(root.join("real/base.toml"), root.join("link.toml")).expect("link");
    let mut session = Session::start(&root);
    session.open(&root, "a.md", SLOPPY);
    assert!(!session.codes(&root, "a.md").iter().any(|c| c == LEXICON));
    fs::write(root.join("real/base.toml"), "schema_version = 3\n").expect("edit");
    // An editor that resolves links reports the file the link points to.
    changed(&mut session, &uri_of(&root, "real/base.toml"));
    assert!(session.codes(&root, "a.md").iter().any(|c| c == LEXICON));
    session.finish();
}

#[test]
fn a_category_error_in_an_external_extended_file_is_shown_on_that_file_at_the_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    let top = dir.path().canonicalize().expect("canonical");
    fs::create_dir_all(top.join("shared")).expect("mkdir");
    fs::create_dir_all(top.join("proj")).expect("mkdir");
    fs::write(top.join("shared/base.toml"), "schema_version = 3\n").expect("base");
    fs::write(
        top.join("proj/slopvac.toml"),
        "schema_version = 3\nextends = [\"../shared/base.toml\"]\n",
    )
    .expect("config");
    let root = top.join("proj");
    let mut session = Session::start(&root);
    fs::write(
        top.join("shared/base.toml"),
        "schema_version = 3\n\n[categories]\nno-such-category = \"off\"\n",
    )
    .expect("break");
    let base = uri_of(&top, "shared/base.toml");
    changed(&mut session, &base);
    let shown = diagnostics_on(&session, &base);
    assert_eq!(shown[0]["code"], "config-unknown-category", "{shown}");
    assert_eq!(
        shown[0]["range"]["start"],
        json!({"line": 3, "character": 0})
    );
    let message = shown[0]["message"].as_str().expect("message");
    assert!(
        !message.contains(top.to_str().expect("utf-8")),
        "an absolute path in {message}"
    );
    session.finish();
}

#[test]
fn a_nested_config_that_breaks_fails_the_reload_and_a_fix_restores_every_document() {
    let dir = project("schema_version = 3\n", &[]);
    let root = dir.path().canonicalize().expect("canonical");
    fs::create_dir_all(root.join("sub")).expect("mkdir");
    fs::write(root.join("sub/slopvac.toml"), "schema_version = 3\n").expect("nested");
    let mut session = Session::start(&root);
    session.open(&root, "top.md", SLOPPY);
    session.open(&root, "sub/a.md", SLOPPY);
    assert!(session.codes(&root, "top.md").iter().any(|c| c == LEXICON));
    assert!(
        session
            .codes(&root, "sub/a.md")
            .iter()
            .any(|c| c == LEXICON)
    );
    let nested = uri_of(&root, "sub/slopvac.toml");
    fs::write(
        root.join("sub/slopvac.toml"),
        "schema_version = 3\n\n[categories]\nno-such-category = \"off\"\n",
    )
    .expect("break");
    changed(&mut session, &nested);
    let shown = diagnostics_on(&session, &nested);
    assert_eq!(shown[0]["code"], "config-unknown-category", "{shown}");
    // The document outside the nested folder is not left looking checked.
    assert_eq!(session.codes(&root, "top.md"), ["lsp-lint-incomplete"]);
    fs::write(root.join("sub/slopvac.toml"), "schema_version = 3\n").expect("fix");
    changed(&mut session, &nested);
    assert_eq!(diagnostics_on(&session, &nested), json!([]));
    assert!(session.codes(&root, "top.md").iter().any(|c| c == LEXICON));
    session.finish();
}
