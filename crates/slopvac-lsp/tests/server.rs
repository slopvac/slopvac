//! Drives the server over an in-memory connection, as an editor would.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use lsp_server::{Connection, Message, Notification, Request, RequestId};
use serde_json::{Value, json};
use slopvac_engine::{DIAGRAM_PACK, Settings, diagram_rule_set};
use slopvac_lsp::{
    Check, EngineLinter, Exit, LintError, Linted, Linter, Problem, Server, ServerError,
};
use slopvac_model::{
    GenreId, GenreResult, GenreSource, KebabName, NonEmptyString, Notice, NoticeLevel, PackId,
    RuleId, Severity, Span,
};
use slopvac_rules::{CompileOptions, RuleSet};

const URI: &str = "file:///work/doc.md";
const WAIT: Duration = Duration::from_secs(20);

/// `utilize` becomes `use`; the rule lists one exception.
fn engine_linter() -> EngineLinter {
    engine_linter_with(Vec::new())
}

/// [`engine_linter`] plus `extra` rules in the same category.
fn engine_linter_with(extra: Vec<Value>) -> EngineLinter {
    EngineLinter::new(vec![core_rule_set(extra)], engine_settings()).unwrap()
}

fn engine_settings() -> Settings {
    Settings::new(
        NonEmptyString::new("normal").unwrap(),
        GenreResult {
            id: GenreId::new("reference").unwrap(),
            source: GenreSource::Default,
            reasons: Vec::new(),
        },
    )
}

/// The `swap` rule plus `extra` rules, in one category.
fn core_rule_set(extra: Vec<Value>) -> RuleSet {
    let rule = json!({
        "id": "swap", "name": "n", "message": "Found {match}.", "severity": "warning",
        "match": {"kind": "substitution", "substitutions": {"utilize": "use"}},
        "exceptions": [{"name": "dead-metaphor"}],
        "citation": {"source": "s"}
    });
    let rules = [vec![rule], extra].concat();
    let category = json!({
        "schema_version": 3, "id": "cat", "title": "t", "description": "d", "rules": rules
    });
    RuleSet::from_sources(
        &PackId::new("core").unwrap(),
        vec![("cat.json".into(), category.to_string())],
        &CompileOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"))
}

struct Client {
    conn: Connection,
    server: Option<JoinHandle<Result<Exit, ServerError>>>,
    queued: VecDeque<Notification>,
    next_id: i32,
    initialize: Value,
}

impl Client {
    fn start(server: Server, offered: Option<&[&str]>) -> Self {
        let (server_side, conn) = Connection::memory();
        let handle = std::thread::spawn(move || server.serve(&server_side));
        let mut client = Self {
            conn,
            server: Some(handle),
            queued: VecDeque::new(),
            next_id: 0,
            initialize: Value::Null,
        };
        let general = offered.map_or(json!({}), |o| json!({"positionEncodings": o}));
        client.initialize = client.request(
            "initialize",
            &json!({"processId": null, "rootUri": null, "capabilities": {"general": general}}),
        );
        client.notify("initialized", &json!({}));
        client
    }

    fn engine(offered: Option<&[&str]>) -> Self {
        Self::start(Server::new(Arc::new(engine_linter())), offered)
    }

    fn send(&self, message: Message) {
        self.conn.sender.send(message).expect("server is running");
    }

    fn notify(&self, method: &str, params: &Value) {
        self.send(Message::Notification(Notification::new(
            method.to_owned(),
            params,
        )));
    }

    /// Send a request and return its `result`, queueing notifications that arrive first.
    fn request(&mut self, method: &str, params: &Value) -> Value {
        self.next_id += 1;
        let id = RequestId::from(self.next_id);
        self.send(Message::Request(Request::new(
            id.clone(),
            method.to_owned(),
            params,
        )));
        loop {
            match self.conn.receiver.recv_timeout(WAIT).expect("a response") {
                Message::Response(r) if r.id == id => {
                    return r.response_result.unwrap_or_else(|e| panic!("{e:?}"));
                }
                Message::Notification(n) => self.queued.push_back(n),
                other => panic!("unexpected message {other:?}"),
            }
        }
    }

    /// The params of the next `publishDiagnostics`.
    fn diagnostics(&mut self) -> Value {
        if let Some(n) = self.queued.pop_front() {
            assert_eq!(n.method, "textDocument/publishDiagnostics");
            return n.params;
        }
        match self.conn.receiver.recv_timeout(WAIT).expect("diagnostics") {
            Message::Notification(n) if n.method == "textDocument/publishDiagnostics" => n.params,
            other => panic!("unexpected message {other:?}"),
        }
    }

    fn open(&mut self, text: &str) -> Value {
        self.open_at(URI, text);
        self.diagnostics()
    }

    /// Send `didOpen` at version 1 without waiting for diagnostics.
    fn open_at(&self, uri: &str, text: &str) {
        self.notify(
            "textDocument/didOpen",
            &json!({"textDocument": {"uri": uri, "languageId": "markdown", "version": 1, "text": text}}),
        );
    }

    fn close(&self, uri: &str) {
        self.notify(
            "textDocument/didClose",
            &json!({"textDocument": {"uri": uri}}),
        );
    }

    /// A request the server answers from the main loop, so its reply proves every earlier
    /// notification was handled.
    fn barrier(&mut self) {
        self.request(
            "textDocument/hover",
            &json!({"textDocument": {"uri": "file:///none.md"}, "position": {"line": 0, "character": 0}}),
        );
    }

    fn finish(mut self) -> Result<Exit, ServerError> {
        self.request("shutdown", &Value::Null);
        self.notify("exit", &Value::Null);
        self.server.take().unwrap().join().unwrap()
    }

    fn exit_without_shutdown(mut self) -> Result<Exit, ServerError> {
        self.notify("exit", &Value::Null);
        self.server.take().unwrap().join().unwrap()
    }
}

/// `(line, character)` pairs of the first diagnostic's range.
fn range_of(diagnostic: &Value) -> [u64; 4] {
    let r = &diagnostic["range"];
    [
        r["start"]["line"].as_u64().unwrap(),
        r["start"]["character"].as_u64().unwrap(),
        r["end"]["line"].as_u64().unwrap(),
        r["end"]["character"].as_u64().unwrap(),
    ]
}

#[test]
fn initialize_answers_the_negotiated_position_encoding() {
    let cases: [(Option<&[&str]>, &str); 4] = [
        (None, "utf-16"),
        (Some(&["utf-16"]), "utf-16"),
        (Some(&["utf-32", "utf-16"]), "utf-32"),
        (Some(&["utf-16", "utf-8", "utf-32"]), "utf-8"),
    ];
    for (offered, expected) in cases {
        let client = Client::engine(offered);
        assert_eq!(
            client.initialize["capabilities"]["positionEncoding"], expected,
            "offered {offered:?}"
        );
        client.finish().unwrap();
    }
}

#[test]
fn diagnostic_columns_use_the_negotiated_unit_after_an_astral_character() {
    // U+1F600 is 4 UTF-8 bytes, 2 UTF-16 code units and 1 code point. `utilize` follows
    // the emoji and one space and is 7 characters long in every unit.
    let cases: [(&[&str], u64); 3] = [
        (&["utf-8"], 5),
        (&["utf-16"], 3),
        (&["utf-32", "utf-16"], 2),
    ];
    for (offered, start) in cases {
        let mut client = Client::engine(Some(offered));
        let published = client.open("😀 utilize it\n");
        let diagnostics = published["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), 1, "offered {offered:?}");
        assert_eq!(
            range_of(&diagnostics[0]),
            [0, start, 0, start + 7],
            "offered {offered:?}"
        );
        client.finish().unwrap();
    }
}

#[test]
fn a_byte_order_mark_in_the_buffer_counts_as_a_character_of_line_one() {
    // positions.md: the BOM is one column in utf-16 and three in utf-8.
    for (offered, start) in [(&["utf-16"], 1), (&["utf-8"], 3)] {
        let mut client = Client::engine(Some(offered));
        let published = client.open("\u{feff}utilize it\n");
        let d = &published["diagnostics"][0];
        assert_eq!(range_of(d), [0, start, 0, start + 7], "offered {offered:?}");
        client.finish().unwrap();
    }
}

#[test]
fn opening_a_document_publishes_its_findings() {
    let mut client = Client::engine(None);
    let published = client.open("We utilize it.\n");
    assert_eq!(published["uri"], URI);
    assert_eq!(published["version"], 1);
    let d = &published["diagnostics"][0];
    assert_eq!(d["code"], "core:cat.swap");
    assert_eq!(d["source"], "slopvac");
    assert_eq!(d["severity"], 2);
    client.finish().unwrap();
}

#[test]
fn changing_a_document_replaces_its_diagnostics() {
    let mut client = Client::engine(None);
    assert_eq!(
        client.open("We utilize it.\n")["diagnostics"][0]["code"],
        "core:cat.swap"
    );
    client.notify(
        "textDocument/didChange",
        &json!({"textDocument": {"uri": URI, "version": 2},
                "contentChanges": [{"text": "We use it.\n"}]}),
    );
    let published = client.diagnostics();
    assert_eq!(published["version"], 2);
    assert_eq!(published["diagnostics"], json!([]));
    client.finish().unwrap();
}

/// Fails every lint with a message that names the current "rules" revision.
struct Revised(Arc<AtomicUsize>);

impl Linter for Revised {
    fn lint(&self, _path: &str, _text: &str) -> Result<Linted, LintError> {
        Err(LintError(format!(
            "rules r{}",
            self.0.load(Ordering::SeqCst)
        )))
    }

    fn allow_reasons(&self, _path: &str, _text: &str, _site: usize, _rule: &RuleId) -> Vec<String> {
        Vec::new()
    }
}

fn message(published: &Value) -> String {
    published["diagnostics"][0]["message"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn saving_a_document_lints_it_again_with_the_current_rules() {
    let revision = Arc::new(AtomicUsize::new(0));
    let mut client = Client::start(Server::new(Arc::new(Revised(Arc::clone(&revision)))), None);
    assert!(message(&client.open("Same text.\n")).ends_with("rules r0"));
    revision.store(1, Ordering::SeqCst);
    client.notify(
        "textDocument/didSave",
        &json!({"textDocument": {"uri": URI}}),
    );
    assert!(message(&client.diagnostics()).ends_with("rules r1"));
    client.finish().unwrap();
}

#[test]
fn closing_a_document_clears_its_diagnostics() {
    let mut client = Client::engine(None);
    client.open("We utilize it.\n");
    client.notify(
        "textDocument/didClose",
        &json!({"textDocument": {"uri": URI}}),
    );
    assert_eq!(client.diagnostics()["diagnostics"], json!([]));
    client.finish().unwrap();
}

fn actions(client: &mut Client, range: &Value) -> Vec<Value> {
    client
        .request(
            "textDocument/codeAction",
            &json!({"textDocument": {"uri": URI}, "range": range, "context": {"diagnostics": []}}),
        )
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn the_fix_action_replaces_the_finding_in_negotiated_units() {
    let mut client = Client::engine(Some(&["utf-16"]));
    client.open("😀 utilize it\n");
    let at = json!({"start": {"line": 0, "character": 4}, "end": {"line": 0, "character": 4}});
    let found = actions(&mut client, &at);
    let fix = found
        .iter()
        .find(|a| {
            a["title"]
                .as_str()
                .unwrap()
                .starts_with("Apply slopvac fix")
        })
        .expect("a fix action");
    assert_eq!(fix["kind"], "quickfix");
    assert_eq!(
        fix["edit"]["changes"][URI],
        json!([{"range": {"start": {"line": 0, "character": 3},
                          "end": {"line": 0, "character": 10}}, "newText": "use"}])
    );
    client.finish().unwrap();
}

#[test]
fn the_allow_action_inserts_a_directive_with_the_rule_reason_above_the_line() {
    let mut client = Client::engine(None);
    client.open("First line.\n\nWe utilize it.\n");
    let at = json!({"start": {"line": 2, "character": 3}, "end": {"line": 2, "character": 3}});
    let found = actions(&mut client, &at);
    let allow = found
        .iter()
        .find(|a| {
            a["title"]
                .as_str()
                .unwrap()
                .starts_with("Allow core:cat.swap")
        })
        .expect("an allow action");
    assert_eq!(
        allow["edit"]["changes"][URI],
        json!([{"range": {"start": {"line": 2, "character": 0},
                          "end": {"line": 2, "character": 0}},
                "newText": "<!-- slopvac-allow: rule=core:cat.swap reason=dead-metaphor -->\n"}])
    );
    client.finish().unwrap();
}

#[test]
fn no_action_is_offered_away_from_a_finding() {
    let mut client = Client::engine(None);
    client.open("First line.\n\nWe utilize it.\n");
    let at = json!({"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 2}});
    assert_eq!(actions(&mut client, &at).len(), 0);
    client.finish().unwrap();
}

#[test]
fn hover_explains_the_finding_under_the_cursor() {
    let mut client = Client::engine(None);
    client.open("We utilize it.\n");
    let hover = |client: &mut Client, character: u32| {
        client.request(
            "textDocument/hover",
            &json!({"textDocument": {"uri": URI}, "position": {"line": 0, "character": character}}),
        )
    };
    let over = hover(&mut client, 5);
    let value = over["contents"]["value"].as_str().unwrap();
    assert!(value.contains("core:cat.swap"), "{value}");
    assert!(value.contains("Found utilize."), "{value}");
    assert!(value.contains("`use`"), "{value}");
    assert_eq!(hover(&mut client, 0), Value::Null);
    client.finish().unwrap();
}

struct Shouting;

impl Check for Shouting {
    fn check(&self, _path: &str, text: &str) -> Vec<Problem> {
        text.find("NOPE")
            .map(|at| Problem {
                span: Span::new(at, at + 4).unwrap(),
                severity: Severity::Error,
                code: "front-matter-invalid".to_owned(),
                message: "NOPE is not allowed.".to_owned(),
            })
            .into_iter()
            .collect()
    }
}

#[test]
fn a_registered_check_adds_diagnostics_next_to_the_findings() {
    let server = Server::new(Arc::new(engine_linter())).with_check(Arc::new(Shouting));
    let mut client = Client::start(server, None);
    let published = client.open("We utilize NOPE.\n");
    let codes: Vec<&str> = published["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["core:cat.swap", "front-matter-invalid"]);
    client.finish().unwrap();
}

struct Failing;

impl Linter for Failing {
    fn lint(&self, _path: &str, _text: &str) -> Result<Linted, LintError> {
        Err(LintError("the engine broke".to_owned()))
    }

    fn allow_reasons(&self, _path: &str, _text: &str, _site: usize, _rule: &RuleId) -> Vec<String> {
        Vec::new()
    }
}

#[test]
fn a_document_that_cannot_be_linted_is_not_reported_clean() {
    let mut client = Client::start(Server::new(Arc::new(Failing)), None);
    let published = client.open("Anything.\n");
    let d = &published["diagnostics"][0];
    assert_eq!(d["code"], "lsp-lint-incomplete");
    assert!(d["message"].as_str().unwrap().contains("the engine broke"));
    client.finish().unwrap();
}

#[test]
fn shutdown_and_exit_end_the_server_cleanly() {
    let client = Client::engine(None);
    assert_eq!(client.finish().unwrap(), Exit::AfterShutdown);
}

#[test]
fn exit_without_shutdown_ends_the_server_and_says_so() {
    let client = Client::engine(None);
    assert_eq!(
        client.exit_without_shutdown().unwrap(),
        Exit::WithoutShutdown
    );
}

/// Lints text that contains "gate" only after the test lets it: it reports the start on
/// `started` and waits on `release`. Every lint fails with a message naming its text, so a
/// result is traceable to the text it linted.
struct Gated {
    started: crossbeam_channel::Sender<()>,
    release: crossbeam_channel::Receiver<()>,
}

impl Linter for Gated {
    fn lint(&self, _path: &str, text: &str) -> Result<Linted, LintError> {
        if text.contains("gate") {
            self.started.send(()).unwrap();
            self.release.recv_timeout(WAIT).unwrap();
        }
        Err(LintError(format!("linted {}", text.trim_end())))
    }

    fn allow_reasons(&self, _path: &str, _text: &str, _site: usize, _rule: &RuleId) -> Vec<String> {
        Vec::new()
    }
}

struct GateControls {
    started: crossbeam_channel::Receiver<()>,
    release: crossbeam_channel::Sender<()>,
}

fn gated_client() -> (Client, GateControls) {
    let (started_tx, started) = crossbeam_channel::unbounded();
    let (release, release_rx) = crossbeam_channel::unbounded();
    let linter = Gated {
        started: started_tx,
        release: release_rx,
    };
    let client = Client::start(Server::new(Arc::new(linter)), None);
    (client, GateControls { started, release })
}

#[test]
fn a_lint_that_finishes_after_the_document_closed_publishes_nothing() {
    let (mut client, gate) = gated_client();
    client.open_at(URI, "gate-a");
    gate.started.recv_timeout(WAIT).unwrap();
    client.close(URI);
    client.barrier();
    assert_eq!(client.diagnostics()["diagnostics"], json!([]));
    gate.release.send(()).unwrap();
    // The worker is idle once it lints and publishes this second document. The stale
    // result for the first would have come before it.
    let other = "file:///work/other.md";
    client.open_at(other, "plain");
    let published = client.diagnostics();
    assert_eq!(published["uri"], other);
    assert!(message(&published).ends_with("linted plain"));
    client.finish().unwrap();
}

#[test]
fn a_lint_of_a_closed_document_does_not_leak_into_its_reopened_version() {
    let (mut client, gate) = gated_client();
    client.open_at(URI, "gate-a");
    gate.started.recv_timeout(WAIT).unwrap();
    client.close(URI);
    // Reopened at the same client version 1 with different text.
    client.open_at(URI, "plain-b");
    client.barrier();
    gate.release.send(()).unwrap();
    assert_eq!(client.diagnostics()["diagnostics"], json!([]));
    let published = client.diagnostics();
    assert_eq!(published["version"], 1);
    assert!(
        message(&published).ends_with("linted plain-b"),
        "{published}"
    );
    client.finish().unwrap();
}

struct Unchecked;

impl Linter for Unchecked {
    fn lint(&self, _path: &str, _text: &str) -> Result<Linted, LintError> {
        Ok(Linted {
            findings: Vec::new(),
            unchecked: vec![Notice {
                code: KebabName::new("rst-unchecked").unwrap(),
                level: NoticeLevel::Warning,
                message: NonEmptyString::new("no RST converter found").unwrap(),
                path: None,
                data: serde_json::Map::new(),
            }],
            notices: Vec::new(),
        })
    }

    fn allow_reasons(&self, _path: &str, _text: &str, _site: usize, _rule: &RuleId) -> Vec<String> {
        Vec::new()
    }
}

#[test]
fn what_the_engine_did_not_check_shows_as_a_diagnostic() {
    let mut client = Client::start(Server::new(Arc::new(Unchecked)), None);
    let published = client.open("Clean prose.\n");
    let d = &published["diagnostics"][0];
    assert_eq!(d["code"], "rst-unchecked");
    assert_eq!(d["severity"], 2);
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .contains("no RST converter found")
    );
    client.finish().unwrap();
}

/// The engine with the built-in diagram checkers next to the `swap` rule.
fn diagram_engine_linter() -> EngineLinter {
    let pack = diagram_rule_set(&PackId::new(DIAGRAM_PACK).unwrap()).unwrap();
    EngineLinter::new(vec![core_rule_set(Vec::new()), pack], engine_settings()).unwrap()
}

const UNSUPPORTED_DIAGRAM: &str = "Intro.\n\n```mermaid\ngitGraph\n  commit\n```\n";

#[test]
fn an_unsupported_diagram_shows_as_an_information_diagnostic_without_actions() {
    let mut client = Client::start(Server::new(Arc::new(diagram_engine_linter())), None);
    let published = client.open(UNSUPPORTED_DIAGRAM);
    let d = only_diagnostic(&published);
    assert_eq!(d["code"], "extract-unsupported");
    assert_eq!(d["severity"], 3);
    assert_eq!(
        d["message"],
        "unsupported Mermaid diagram type; the diagram was not checked"
    );
    assert_eq!(d["range"]["start"], json!({"line": 0, "character": 0}));
    assert_eq!(d["range"]["end"], json!({"line": 0, "character": 0}));
    assert_eq!(actions(&mut client, &d["range"]), Vec::<Value>::new());
    client.finish().unwrap();
}

#[test]
fn a_document_without_unsupported_content_has_no_information_diagnostic() {
    let mut client = Client::start(Server::new(Arc::new(diagram_engine_linter())), None);
    let published = client.open("Intro.\n\n```mermaid\nflowchart TD\n  A --> B\n```\n");
    assert_eq!(published["diagnostics"], json!([]));
    client.finish().unwrap();
}

/// Insert `new_text` at `(line, 0)` of `text`.
fn insert_at_line(text: &str, line: usize, new_text: &str) -> String {
    let at: usize = text.split_inclusive('\n').take(line).map(str::len).sum();
    format!("{}{new_text}{}", &text[..at], &text[at..])
}

#[test]
fn the_allow_action_makes_the_real_directive_scanner_suppress_the_finding() {
    // The first case puts the finding on a line of its own; the second inside a paragraph.
    let cases = [
        ("First line.\n\nWe utilize it.\n", 2),
        ("Intro line.\nWe utilize it.\n", 1),
    ];
    for (text, line) in cases {
        let mut client = Client::engine(None);
        assert_eq!(
            client.open(text)["diagnostics"].as_array().unwrap().len(),
            1
        );
        let at =
            json!({"start": {"line": line, "character": 3}, "end": {"line": line, "character": 3}});
        let found = actions(&mut client, &at);
        let allow = found
            .iter()
            .find(|a| a["title"].as_str().unwrap().starts_with("Allow"))
            .expect("an allow action");
        let edit = &allow["edit"]["changes"][URI][0];
        let edited = insert_at_line(text, line, edit["newText"].as_str().unwrap());
        client.notify(
            "textDocument/didChange",
            &json!({"textDocument": {"uri": URI, "version": 2},
                    "contentChanges": [{"text": edited}]}),
        );
        assert_eq!(client.diagnostics()["diagnostics"], json!([]), "{edited:?}");
        client.finish().unwrap();
    }
}

#[test]
fn no_allow_action_is_offered_for_mdx_which_rejects_html_comments() {
    let mdx = "file:///work/doc.mdx";
    let mut client = Client::engine(None);
    client.open_at(mdx, "We utilize it.\n");
    assert_eq!(
        client.diagnostics()["diagnostics"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let found = client.request(
        "textDocument/codeAction",
        &json!({"textDocument": {"uri": mdx},
                "range": {"start": {"line": 0, "character": 4}, "end": {"line": 0, "character": 4}},
                "context": {"diagnostics": []}}),
    );
    let titles: Vec<&str> = found
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Apply slopvac fix: core:cat.swap"]);
    client.finish().unwrap();
}

#[test]
fn only_markdown_files_get_allow_reasons() {
    let linter = engine_linter();
    let rule = RuleId::new("core:cat.swap").unwrap();
    let text = "We utilize it.\n";
    assert_eq!(
        linter.allow_reasons("/w/a.md", text, 0, &rule),
        ["dead-metaphor"]
    );
    assert_eq!(
        linter.allow_reasons("/w/a.mdx", text, 0, &rule),
        Vec::<String>::new()
    );
    assert_eq!(
        linter.allow_reasons("/w/a.rst", text, 0, &rule),
        Vec::<String>::new()
    );
}

#[test]
fn no_allow_reasons_where_the_scanner_ignores_directives() {
    let linter = engine_linter();
    let rule = RuleId::new("core:cat.swap").unwrap();
    let reasons = |text: &str, line: &str| {
        let site = text.find(line).unwrap();
        linter.allow_reasons("/w/a.md", text, site, &rule)
    };
    let front = "---\ntitle: x\n---\nWe utilize it.\n";
    assert!(reasons(front, "title").is_empty(), "front matter");
    assert_eq!(reasons(front, "We utilize"), ["dead-metaphor"]);
    let fence = "```\ncode here\n```\n\nWe utilize it.\n";
    assert!(reasons(fence, "code here").is_empty(), "fenced code");
    assert_eq!(reasons(fence, "We utilize"), ["dead-metaphor"]);
}

fn metric_rule() -> Value {
    json!({
        "id": "long", "name": "n", "message": "Too long.", "severity": "warning",
        "match": {"kind": "metric", "metric": "syllables_per_word", "threshold": 2,
                  "comparison": "gt"},
        "scope": "sentence",
        "citation": {"source": "s"}
    })
}

#[test]
fn a_skipped_rule_kind_of_the_real_engine_shows_as_a_diagnostic() {
    // The engine does not run metric rules yet and says so in `Document.unchecked`.
    let linter = engine_linter_with(vec![metric_rule()]);
    let mut client = Client::start(Server::new(Arc::new(linter)), None);
    let published = client.open("Clean prose.\n");
    let d = &published["diagnostics"][0];
    assert_eq!(d["code"], "rule-kind-not-implemented");
    assert_eq!(d["severity"], 3);
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .contains("1 active metric rule(s) did not run")
    );
    client.finish().unwrap();
}

/// Publishes in the order the document lock decided: a message found fresh stays ahead of
/// the clear that a close queues while it waits.
#[test]
fn a_close_during_publication_is_never_followed_by_stale_diagnostics() {
    let (reached_tx, reached) = crossbeam_channel::unbounded::<()>();
    let (release, release_rx) = crossbeam_channel::unbounded::<()>();
    let first = std::sync::atomic::AtomicBool::new(true);
    let server = Server::new(Arc::new(Revised(Arc::new(AtomicUsize::new(7)))))
        .with_pre_publish_hook(Arc::new(move |_uri| {
            if first.swap(false, Ordering::SeqCst) {
                reached_tx.send(()).unwrap();
                release_rx.recv_timeout(WAIT).unwrap();
            }
        }));
    let mut client = Client::start(server, None);
    client.open_at(URI, "text");
    // The worker has found the diagnostics fresh and is about to queue them.
    reached.recv_timeout(WAIT).unwrap();
    client.close(URI);
    release.send(()).unwrap();
    let first_message = client.diagnostics();
    assert_eq!(first_message["diagnostics"].as_array().unwrap().len(), 1);
    assert_eq!(client.diagnostics()["diagnostics"], json!([]));
    // Nothing stale follows: the next message is for the next document.
    let other = "file:///work/other.md";
    client.open_at(other, "more");
    assert_eq!(client.diagnostics()["uri"], other);
    client.finish().unwrap();
}

/// A running server over zero-capacity channels, like the stdio transport: a send completes
/// only when the other side receives.
struct Stalled {
    to_server: crossbeam_channel::Sender<Message>,
    from_server: crossbeam_channel::Receiver<Message>,
    reached: crossbeam_channel::Receiver<()>,
    /// Releases the publication held in the pre-publish hook (see `start_gated`).
    release: crossbeam_channel::Sender<()>,
    done: crossbeam_channel::Receiver<Result<Exit, ServerError>>,
}

impl Stalled {
    /// With `gate_second`, the second publication waits in the pre-publish hook, with the
    /// document lock held, until `release` is signalled.
    fn start(gate_second: bool) -> Self {
        let (to_client, from_server) = crossbeam_channel::bounded::<Message>(0);
        let (to_server, from_client) = crossbeam_channel::bounded::<Message>(0);
        let (reached_tx, reached) = crossbeam_channel::unbounded::<()>();
        let (done_tx, done) = crossbeam_channel::unbounded();
        let (release, release_rx) = crossbeam_channel::unbounded::<()>();
        let calls = AtomicUsize::new(0);
        let server = Server::new(Arc::new(Revised(Arc::new(AtomicUsize::new(0)))))
            .with_pre_publish_hook(Arc::new(move |_uri| {
                let _ = reached_tx.send(());
                if gate_second && calls.fetch_add(1, Ordering::SeqCst) == 1 {
                    let _ = release_rx.recv_timeout(WAIT);
                }
            }));
        let connection = Connection {
            sender: to_client,
            receiver: from_client,
        };
        std::thread::spawn(move || {
            let _ = done_tx.send(server.serve(&connection));
        });
        Self {
            to_server,
            from_server,
            reached,
            release,
            done,
        }
    }

    fn send(&self, message: Message) {
        self.to_server
            .send_timeout(message, Duration::from_secs(5))
            .expect("the server keeps reading");
    }

    fn note(&self, method: &str, params: &Value) {
        self.send(Message::Notification(Notification::new(
            method.to_owned(),
            params,
        )));
    }

    fn change(&self, version: i32) {
        self.note(
            "textDocument/didChange",
            &json!({"textDocument": {"uri": URI, "version": version},
                    "contentChanges": [{"text": format!("v{version}")}]}),
        );
    }

    fn handshake(&self) {
        self.send(Message::Request(Request::new(
            RequestId::from(1),
            "initialize".to_owned(),
            json!({"processId": null, "rootUri": null, "capabilities": {}}),
        )));
        self.from_server
            .recv_timeout(WAIT)
            .expect("initialize response");
        self.note("initialized", &json!({}));
    }

    fn open_document(&self) {
        self.note(
            "textDocument/didOpen",
            &json!({"textDocument": {"uri": URI, "languageId": "markdown", "version": 1, "text": "a"}}),
        );
    }

    /// Handshake, open a document and change it three times, waiting for the server to
    /// queue each message. Then stop reading: the writer blocks on the transport with more
    /// messages queued behind the one it holds.
    fn start_and_stop_reading() -> Self {
        let session = Self::start(false);
        session.handshake();
        session.open_document();
        session.reached.recv_timeout(WAIT).unwrap();
        for version in 2..=4 {
            session.change(version);
            session.reached.recv_timeout(WAIT).unwrap();
        }
        session
    }

    /// Handshake and open a document, then change it. The worker finds the change's
    /// diagnostics fresh and waits in the hook, so nothing is queued yet. Nothing is read
    /// from the server after the handshake.
    fn start_gated() -> Self {
        let session = Self::start(true);
        session.handshake();
        session.open_document();
        session.reached.recv_timeout(WAIT).unwrap();
        session.change(2);
        session.reached.recv_timeout(WAIT).unwrap();
        session
    }

    /// Let the held publication go. The client reads nothing, so the writer blocks on the
    /// transport with that message in hand.
    fn release_the_held_publication(&self) {
        self.release.send(()).unwrap();
    }

    /// Send `shutdown` and read until its response.
    fn shutdown_and_read_the_response(&self) {
        self.send(Message::Request(Request::new(
            RequestId::from(2),
            "shutdown".to_owned(),
            Value::Null,
        )));
        loop {
            if let Message::Response(r) = self
                .from_server
                .recv_timeout(WAIT)
                .expect("shutdown response")
            {
                assert_eq!(r.id, RequestId::from(2));
                break;
            }
        }
    }
}

#[test]
fn a_client_that_stops_reading_does_not_stall_document_updates() {
    let session = Stalled::start_and_stop_reading();
    // The transport is blocked. Document updates must still be accepted.
    session.change(5);
    session.change(6);
    // The client reads again and gets the latest diagnostics.
    loop {
        match session
            .from_server
            .recv_timeout(WAIT)
            .expect("latest diagnostics")
        {
            Message::Notification(n) if n.params["version"] == 6 => break,
            _ => {}
        }
    }
    session.shutdown_and_read_the_response();
    session.note("exit", &Value::Null);
    // Keep reading while the server finishes, so queued messages cannot block it.
    let deadline = std::time::Instant::now() + WAIT;
    let result = loop {
        if let Ok(result) = session.done.try_recv() {
            break result;
        }
        assert!(std::time::Instant::now() < deadline, "serve did not finish");
        let _ = session.from_server.recv_timeout(Duration::from_millis(20));
    };
    assert_eq!(result.unwrap(), Exit::AfterShutdown);
}

#[test]
fn a_clean_exit_does_not_wait_for_a_client_that_stopped_reading() {
    let session = Stalled::start_gated();
    session.shutdown_and_read_the_response();
    // A publication is queued only now, after the last message the client reads, so the
    // writer is blocked on the transport with it.
    session.release_the_held_publication();
    session.note("exit", &Value::Null);
    let result = session
        .done
        .recv_timeout(Duration::from_secs(20))
        .expect("serve returns although the transport is blocked");
    assert_eq!(result.unwrap(), Exit::AfterShutdown);
}

#[test]
fn a_fatal_error_returns_at_once_although_the_transport_is_blocked() {
    let session = Stalled::start_gated();
    session.shutdown_and_read_the_response();
    session.release_the_held_publication();
    // Anything but `exit` after `shutdown` is a protocol error, which ends serve.
    session.send(Message::Request(Request::new(
        RequestId::from(3),
        "textDocument/hover".to_owned(),
        Value::Null,
    )));
    // Shorter than the bounded wait of a clean exit, so waiting for the writer fails it.
    let result = session
        .done
        .recv_timeout(Duration::from_secs(4))
        .expect("serve returns although the transport is blocked");
    let Err(error @ ServerError::Protocol) = result else {
        panic!("expected a protocol error, got {result:?}");
    };
    // The message is fixed: the protocol error's text can quote a client message.
    assert_eq!(error.to_string(), "language server protocol error");
}

/// Prose the swap rule would flag, so a document that is linted anyway shows a finding.
const FLAGGED: &str = "We utilize it.\n";

/// The `codeAction` result over the whole first line of `uri`.
fn actions_at(client: &mut Client, uri: &str) -> Value {
    client.request(
        "textDocument/codeAction",
        &json!({
            "textDocument": {"uri": uri},
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 14}},
            "context": {"diagnostics": []}
        }),
    )
}

/// The single diagnostic of `published`, asserting there is exactly one.
fn only_diagnostic(published: &Value) -> &Value {
    let all = published["diagnostics"].as_array().unwrap();
    assert_eq!(all.len(), 1, "{all:?}");
    &all[0]
}

fn assert_binary_host_diagnostic(published: &Value) {
    let d = only_diagnostic(published);
    assert_eq!(d["code"], "lsp-binary-unchecked");
    assert_eq!(d["severity"], 2);
    assert_eq!(d["source"], "slopvac");
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .contains("binary host not linted in the editor; run the CLI")
    );
}

#[rstest::rstest]
#[case::docx("file:///work/a.docx")]
#[case::docm("file:///work/a.docm")]
#[case::dotx("file:///work/a.dotx")]
#[case::dotm("file:///work/a.dotm")]
#[case::pptx("file:///work/a.pptx")]
#[case::pptm("file:///work/a.pptm")]
#[case::potx("file:///work/a.potx")]
#[case::potm("file:///work/a.potm")]
#[case::ppsx("file:///work/a.ppsx")]
#[case::ppsm("file:///work/a.ppsm")]
#[case::xlsx("file:///work/a.xlsx")]
#[case::xlsm("file:///work/a.xlsm")]
#[case::xltx("file:///work/a.xltx")]
#[case::xltm("file:///work/a.xltm")]
#[case::odt("file:///work/a.odt")]
#[case::ott("file:///work/a.ott")]
#[case::odp("file:///work/a.odp")]
#[case::otp("file:///work/a.otp")]
#[case::ods("file:///work/a.ods")]
#[case::ots("file:///work/a.ots")]
#[case::drawio_png("file:///work/a.drawio.png")]
#[case::excalidraw_png("file:///work/a.excalidraw.png")]
#[case::upper_case("file:///work/A.DOCX")]
fn opening_a_binary_container_publishes_one_diagnostic_and_offers_no_action(#[case] uri: &str) {
    let mut client = Client::engine(None);
    client.open_at(uri, FLAGGED);
    assert_binary_host_diagnostic(&client.diagnostics());
    assert_eq!(actions_at(&mut client, uri), json!([]));
    client.finish().unwrap();
}

#[rstest::rstest]
#[case::docx("file:///work/a.docx")]
#[case::drawio_png("file:///work/a.drawio.png")]
fn changing_or_saving_a_binary_container_keeps_the_one_diagnostic(#[case] uri: &str) {
    let mut client = Client::engine(None);
    client.open_at(uri, "text\n");
    assert_binary_host_diagnostic(&client.diagnostics());
    client.notify(
        "textDocument/didChange",
        &json!({
            "textDocument": {"uri": uri, "version": 2},
            "contentChanges": [{"text": FLAGGED}]
        }),
    );
    assert_binary_host_diagnostic(&client.diagnostics());
    client.notify(
        "textDocument/didSave",
        &json!({"textDocument": {"uri": uri}}),
    );
    assert_binary_host_diagnostic(&client.diagnostics());
    assert_eq!(actions_at(&mut client, uri), json!([]));
    client.finish().unwrap();
}

#[test]
fn a_markdown_file_is_still_linted_next_to_binary_containers() {
    let mut client = Client::engine(None);
    let published = client.open(FLAGGED);
    assert_eq!(only_diagnostic(&published)["code"], "core:cat.swap");
    client.finish().unwrap();
}

#[test]
fn a_registered_check_adds_nothing_to_a_binary_container() {
    let server = Server::new(Arc::new(engine_linter())).with_check(Arc::new(Shouting));
    let mut client = Client::start(server, None);
    let uri = "file:///work/a.docx";
    client.open_at(uri, "We utilize NOPE.\n");
    assert_binary_host_diagnostic(&client.diagnostics());
    client.notify(
        "textDocument/didChange",
        &json!({
            "textDocument": {"uri": uri, "version": 2},
            "contentChanges": [{"text": "NOPE again.\n"}]
        }),
    );
    assert_binary_host_diagnostic(&client.diagnostics());
    client.notify(
        "textDocument/didSave",
        &json!({"textDocument": {"uri": uri}}),
    );
    assert_binary_host_diagnostic(&client.diagnostics());
    client.finish().unwrap();
}
