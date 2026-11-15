//! Drives the server over an in-memory connection, as an editor would.

use std::collections::VecDeque;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use lsp_server::{Connection, Message, Notification, Request, RequestId};
use serde_json::{Value, json};
use slopvac_engine::Settings;
use slopvac_lsp::{Check, EngineLinter, LintError, Linter, Problem, Server, ServerError};
use slopvac_model::{
    Finding, GenreId, GenreResult, GenreSource, NonEmptyString, PackId, RuleId, Severity, Span,
};
use slopvac_rules::{CompileOptions, RuleSet};

const URI: &str = "file:///work/doc.md";
const WAIT: Duration = Duration::from_secs(20);

/// `utilize` becomes `use`; the rule lists one exception.
fn engine_linter() -> EngineLinter {
    let rule = json!({
        "id": "swap", "name": "n", "message": "Found {match}.", "severity": "warning",
        "match": {"kind": "substitution", "substitutions": {"utilize": "use"}},
        "exceptions": [{"name": "dead-metaphor"}],
        "citation": {"source": "s"}
    });
    let category = json!({
        "schema_version": 3, "id": "cat", "title": "t", "description": "d", "rules": [rule]
    });
    let set = RuleSet::from_sources(
        &PackId::new("core").unwrap(),
        vec![("cat.json".into(), category.to_string())],
        &CompileOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let settings = Settings::new(
        NonEmptyString::new("normal").unwrap(),
        GenreResult {
            id: GenreId::new("reference").unwrap(),
            source: GenreSource::Default,
            reasons: Vec::new(),
        },
    );
    EngineLinter::new(vec![set], settings).unwrap()
}

struct Client {
    conn: Connection,
    server: Option<JoinHandle<Result<(), ServerError>>>,
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
        self.notify(
            "textDocument/didOpen",
            &json!({"textDocument": {"uri": URI, "languageId": "markdown", "version": 1, "text": text}}),
        );
        self.diagnostics()
    }

    fn finish(mut self) -> Result<(), ServerError> {
        self.request("shutdown", &Value::Null);
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

#[test]
fn saving_a_document_publishes_again() {
    let mut client = Client::engine(None);
    client.open("We utilize it.\n");
    client.notify(
        "textDocument/didSave",
        &json!({"textDocument": {"uri": URI}}),
    );
    let published = client.diagnostics();
    assert_eq!(published["diagnostics"].as_array().unwrap().len(), 1);
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
    fn lint(&self, _path: &str, _text: &str) -> Result<Vec<Finding>, LintError> {
        Err(LintError("the engine broke".to_owned()))
    }

    fn allow_reasons(&self, _rule: &RuleId) -> Vec<String> {
        Vec::new()
    }
}

#[test]
fn a_document_that_cannot_be_linted_is_not_reported_clean() {
    let mut client = Client::start(Server::new(Arc::new(Failing)), None);
    let published = client.open("Anything.\n");
    let d = &published["diagnostics"][0];
    assert_eq!(d["code"], "lint-incomplete");
    assert!(d["message"].as_str().unwrap().contains("the engine broke"));
    client.finish().unwrap();
}

#[test]
fn shutdown_and_exit_end_the_server_cleanly() {
    let client = Client::engine(None);
    assert!(client.finish().is_ok());
}
