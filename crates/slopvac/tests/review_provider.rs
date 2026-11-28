//! `slopvac review` with a model provider plugin, through the installed binary.
//!
//! The provider is a real plugin process: a Python script that speaks the plugin protocol,
//! installed in the project as a path plugin, synced and trusted with `slopvac plugins`. It
//! counts its own starts and records each `provider/complete` request, so a test sees what the
//! host sent and whether anything started. Nothing in the binary is replaced.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code: a panic is a failed test"
)]

use rstest::rstest;
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{Value, json};
use snapbox::cmd::{Command, cargo_bin};
use std::fs;
use std::path::{Path, PathBuf};

const FAKE: &str = include_str!("fixtures/review_provider.py");

/// An original and a revision that differ in most sentences, so the change is a rewrite and
/// the payload holds the two documents.
const REWRITE: (&str, &str) = (
    "# Cache\n\nThe cache holds 10 entries. It evicts the oldest entry first.\n",
    "# Cache\n\nThe cache holds 20 entries. It evicts the least recently used entry first. Reads refresh an entry.\n",
);

/// A rewrite whose text JSON must escape: quotes, a backslash, a tab and text outside ASCII.
const ESCAPED: (&str, &str) = (
    "# Cache\n\nThe cache holds 10 entries. It evicts the oldest entry first.\n",
    "# Caf\u{e9}\n\nThe cache holds \"20\" entries\tin C:\\cache \u{2014} na\u{ef}ve. It evicts the \
     least recently used entry first. Reads refresh an entry.\n",
);

/// One literal changes in a longer text, so an `update` review sends the changed passage.
const UPDATE: (&str, &str) = (
    "# Cache\n\nThe cache holds 10 entries. It evicts the oldest entry first. Reads are fast. \
     Writes are slow. Keys are short. Values are long. Locks are rare. Logs are kept.\n",
    "# Cache\n\nThe cache holds 20 entries. It evicts the oldest entry first. Reads are fast. \
     Writes are slow. Keys are short. Values are long. Locks are rare. Logs are kept. \
     Backups run nightly.\n",
);

/// The manifest of the provider plugin, which runs `bin/provider` with the log directory.
fn manifest(log_dir: &Path, hooks: &str) -> String {
    format!(
        "schema_version = 3\nid = \"provider\"\nversion = \"1.0.0\"\nslopvac_api = 1\n\
         kind = \"code\"\n[provides]\n[plugin]\ncommand = \"bin/provider\"\n\
         args = ['{}']\nnetwork = false\nhooks = {hooks}\n\
         features = {{ required = [], optional = [] }}\n",
        log_dir.display()
    )
}

const PROVIDER_HOOK: &str = "[{ kind = \"provider\", id = \"review\" }]";

/// A Python 3 that runs with an empty environment, as a plugin child does.
fn python() -> PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|dir| dir.join("python3"))
        .find(|cand| {
            std::process::Command::new(cand)
                .env_clear()
                .args(["-c", "import json"])
                .status()
                .is_ok_and(|s| s.success())
        })
        .expect("a python3 on PATH that runs with an empty environment")
}

/// A project with two documents, a provider plugin locked by its tree hash, a config, and
/// per-user directories of its own.
struct World {
    dir: tempfile::TempDir,
}

impl World {
    fn new(docs: (&str, &str), config: &str) -> Self {
        Self::with_hooks(docs, config, PROVIDER_HOOK)
    }

    fn with_hooks(docs: (&str, &str), config: &str, hooks: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let world = Self {
            dir: tempfile::tempdir().expect("tempdir"),
        };
        let project = world.project();
        let plugin = project.join("plugins/provider");
        fs::create_dir_all(plugin.join("bin")).unwrap();
        fs::create_dir_all(project.join(".slopvac")).unwrap();
        fs::create_dir_all(world.logs()).unwrap();
        fs::write(project.join("old.md"), docs.0).unwrap();
        fs::write(project.join("new.md"), docs.1).unwrap();
        fs::write(project.join("slopvac.toml"), config).unwrap();
        fs::write(
            plugin.join("slopvac-plugin.toml"),
            manifest(&world.logs(), hooks),
        )
        .unwrap();
        let script = plugin.join("bin/provider");
        fs::write(&script, format!("#!{}\n{FAKE}", python().display())).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        // A plugin tree lists every executable file in its executable list.
        fs::write(plugin.join(".slopvac-exec"), "bin/provider\n").unwrap();
        let digest = slopvac_plugins::extract::tree_hash(&plugin).unwrap();
        fs::write(
            project.join(".slopvac/plugins.lock"),
            format!(
                "schema_version = 3\n\n[[plugins]]\nid = \"provider\"\nkind = \"code\"\n\
                 version = \"1.0.0\"\nslopvac_api = 1\nsource = \"plugins/provider\"\n\
                 digest = \"{digest}\"\n"
            ),
        )
        .unwrap();
        world
    }

    fn project(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    fn logs(&self) -> PathBuf {
        self.dir.path().join("logs")
    }

    fn slopvac(&self) -> Command {
        let home = self.dir.path().join("home");
        let mut cmd = Command::new(cargo_bin!("slopvac"))
            .current_dir(self.project())
            .env("SLOPVAC_NO_UPDATE_CHECK", "1")
            .env("HOME", &home)
            .env("XDG_CACHE_HOME", home.join("cache"))
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_STATE_HOME", home.join("state"))
            .env("NO_COLOR", "1");
        for var in [
            "SLOPVAC_OFFLINE",
            "SLOPVAC_LOG",
            "SLOPVAC_CACHE_DIR",
            "SLOPVAC_TRUST",
            "CI",
            "FORCE_COLOR",
            "CLICOLOR_FORCE",
        ] {
            cmd = cmd.env_remove(var);
        }
        cmd
    }

    /// Sync the plugin in place and record the user's trust in it.
    fn trust(&self) {
        self.slopvac()
            .args(["--offline", "plugins", "sync"])
            .assert()
            .success();
        self.slopvac()
            .args(["--offline", "plugins", "trust", "provider"])
            .assert()
            .success();
    }

    /// Replace the project config.
    fn write_config(&self, text: &str) {
        fs::write(self.project().join("slopvac.toml"), text).unwrap();
    }

    /// How the plugin answers its next request.
    fn script(&self, script: &Value) {
        fs::write(self.logs().join("script.json"), script.to_string()).unwrap();
    }

    /// How many times the plugin process started.
    fn spawns(&self) -> usize {
        fs::read_to_string(self.logs().join("spawns")).map_or(0, |s| s.lines().count())
    }

    /// The params of every `provider/complete` request the plugin received.
    fn requests(&self) -> Vec<Value> {
        fs::read_to_string(self.logs().join("requests"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// The body of each `provider/complete` request, the bytes the plugin received.
    fn raw_requests(&self) -> Vec<String> {
        fs::read_to_string(self.logs().join("requests.raw"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn review(&self, extra: &[&str]) -> std::process::Output {
        let mut args = vec!["review", "new.md", "--original", "old.md"];
        args.extend_from_slice(extra);
        self.slopvac().args(args).output().expect("run")
    }
}

fn config(review: &str) -> String {
    format!("schema_version = 3\n\n[review]\n{review}")
}

/// A config that names the provider and the reviewer model.
fn configured() -> String {
    config("provider = \"provider\"\nmodel = \"reviewer-model\"\n")
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn plan(out: &std::process::Output) -> Value {
    let parsed = serde_json::from_slice::<Value>(&out.stdout);
    assert!(
        parsed.is_ok(),
        "a JSON plan on stdout: {parsed:?}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        stderr(out)
    );
    parsed.unwrap()
}

fn assert_refused(out: &std::process::Output, code: &str) {
    let err = stderr(out);
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(
        err.lines()
            .any(|l| l.starts_with(&format!("error[{code}]: "))),
        "want error[{code}]:\n{err}"
    );
}

#[test]
fn a_complete_reply_becomes_the_report_in_payload_order() {
    let world = World::new(REWRITE, &configured());
    world.trust();
    // The answers come back in reverse order. The first text ends in newlines and the second
    // is empty, which is valid.
    world.script(&json!({"order": "reverse", "text": {"1": "first\n\n", "2": ""}}));

    let out = world.review(&[]);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        plan(&out)["report"],
        format!(
            "{}\n\n### Item 1 (original_document, line 1)\n\nfirst\n\n### Item 2 (revised_document, line 1)\n",
            untrusted_line()
        )
    );
    assert_eq!(world.spawns(), 1);
}

/// The registry text that marks a report as untrusted provider output, as the catalog holds it.
fn untrusted_line() -> String {
    let entry = slopvac_agents::message_entries()
        .iter()
        .find(|e| e.code == "review-report-untrusted");
    assert!(
        entry.is_some(),
        "the agents catalog holds review-report-untrusted"
    );
    entry.map(|e| e.template.to_owned()).unwrap_or_default()
}

/// A result that plugin-protocol.md section 5.3 refuses is `review-provider-reply-invalid`.
#[rstest]
#[case::missing_item(json!({"omit": ["2"]}))]
#[case::extra_item(json!({"extra": ["zzz"]}))]
#[case::out_of_range_item(json!({"extra": ["3"]}))]
#[case::duplicate_item(json!({"duplicate": ["1"]}))]
#[case::entry_without_text(json!({"no_text": ["2"]}))]
#[case::entry_without_model(json!({"no_model": ["1"]}))]
#[case::entry_with_an_empty_model(json!({"empty_model": ["1"]}))]
#[case::results_not_a_list(json!({"results": "none"}))]
fn a_refused_provider_result_exits_2_as_an_invalid_reply(#[case] script: Value) {
    let world = World::new(REWRITE, &configured());
    world.trust();
    world.script(&script);

    let out = world.review(&[]);

    assert_refused(&out, "review-provider-reply-invalid");
    assert!(out.stdout.is_empty(), "no plan on a failed run");
}

/// Failures outside the result check keep their host codes. The plugin's own message never
/// reaches stderr, because it can quote document text (policies.md section 1.7).
#[rstest]
#[case::malformed_error_object(json!({"bad_error": ["1"]}), "plugin-output-invalid")]
#[case::request_level_error(json!({"request_error": -32603}), "internal-error")]
fn a_failure_outside_the_result_check_keeps_its_host_code(
    #[case] script: Value,
    #[case] code: &str,
) {
    let world = World::new(REWRITE, &configured());
    world.trust();
    world.script(&script);

    let out = world.review(&[]);

    assert_refused(&out, code);
    assert!(out.stdout.is_empty(), "no plan on a failed run");
    assert!(
        !stderr(&out).contains("PLUGIN-SECRET-MESSAGE"),
        "{}",
        stderr(&out)
    );
}

/// An entry with `error` stops the review: no plan, the code of the first failed item in
/// payload order, every failed item named, and the provider's own text never shown. The
/// notice known before the call still reaches stderr.
#[test]
fn an_item_error_stops_the_review_with_the_code_of_the_first_failed_item() {
    let world = World::new(
        REWRITE,
        &config("provider = \"provider\"\nmodel = \"same-model\"\nauthor_model = \"same-model\"\n"),
    );
    world.trust();
    world.script(&json!({"order": "reverse", "errors": {"2": -32603, "1": -32006}}));

    let out = world.review(&[]);

    assert_refused(&out, "document-invalid");
    let err = stderr(&out);
    assert!(err.contains("1 (document-invalid)"), "{err}");
    assert!(err.contains("2 (internal-error)"), "{err}");
    assert!(!err.contains("PLUGIN-SECRET-MESSAGE"), "{err}");
    assert!(!err.contains("review-provider-reply-invalid"), "{err}");
    assert!(err.contains("notice[review-model-same]: "), "{err}");
    assert!(out.stdout.is_empty(), "no plan on a failed run");
}

const MECHANICAL: (&str, &str) = (
    "The alpha module handles routine requests. The bravo module handles routine requests. \
     The charlie module handles routine requests. The delta module handles routine requests. \
     The echo module handles routine requests.\n",
    "The alpha module handles routine requests. The bravo module handles routine requests. \
     The charlie module serves routine requests. The delta module handles routine requests. \
     The echo module handles routine requests.\n",
);

#[derive(Debug, Clone, Copy)]
enum Setup {
    Trusted,
    Untrusted,
}

/// How a test lifts the one gate that kept the provider idle, for the control run.
#[derive(Debug, Clone)]
enum Lift {
    /// Run again with these flags in place of the gated ones.
    Flags(&'static [&'static str]),
    /// Trust the plugin, then run again with the same flags.
    Trust,
    /// Write this config, then run again with the same flags.
    Config(String),
}

/// The control: with only the gate lifted, the same project starts the plugin once. It shows
/// that the gate, and nothing else in the setup, kept the plugin idle.
fn assert_lifting_the_gate_starts_it_once(world: &World, flags: &[&str], lift: &Lift) {
    let flags: Vec<&str> = match lift {
        Lift::Flags(lifted) => lifted.to_vec(),
        Lift::Trust => {
            world.trust();
            flags.to_vec()
        }
        Lift::Config(text) => {
            world.write_config(text);
            flags.to_vec()
        }
    };
    let out = world.review(&flags);
    assert_eq!(out.status.code(), Some(0), "control: {}", stderr(&out));
    assert_eq!(world.spawns(), 1, "control: the plugin starts once");
}

/// Each gate alone keeps the provider from starting (policies.md N11, N12). A missing or blank
/// reviewer model stops a run that would call it before the plugin starts, and a dry run needs
/// no model.
#[rstest]
#[case::dry_run(REWRITE, configured(), Setup::Trusted, &["--dry-run"][..], None, Lift::Flags(&[]))]
#[case::offline(
    REWRITE,
    configured(),
    Setup::Trusted,
    &["--offline"][..],
    Some("offline-network-needed"),
    Lift::Flags(&[])
)]
#[case::untrusted(REWRITE, configured(), Setup::Untrusted, &[][..], Some("plugin-untrusted"), Lift::Trust)]
#[case::absent_from_the_config(
    REWRITE,
    config("model = \"reviewer-model\"\n"),
    Setup::Trusted,
    &[][..],
    Some("review-provider-missing"),
    Lift::Config(configured())
)]
#[case::mechanical(
    MECHANICAL,
    configured(),
    Setup::Trusted,
    &[][..],
    None,
    Lift::Flags(&["--type", "rewrite"])
)]
#[case::no_reviewer_model(
    REWRITE,
    config("provider = \"provider\"\n"),
    Setup::Trusted,
    &[][..],
    Some("review-model-missing"),
    Lift::Flags(&["--reviewer-model", "reviewer-model"])
)]
#[case::blank_reviewer_model(
    REWRITE,
    config("provider = \"provider\"\nmodel = \"  \"\n"),
    Setup::Trusted,
    &[][..],
    Some("review-model-missing"),
    Lift::Flags(&["--reviewer-model", "reviewer-model"])
)]
#[case::blank_reviewer_model_flag(
    REWRITE,
    config("provider = \"provider\"\n"),
    Setup::Trusted,
    &["--reviewer-model", " "][..],
    Some("review-model-missing"),
    Lift::Flags(&["--reviewer-model", "reviewer-model"])
)]
#[case::dry_run_without_a_reviewer_model(
    REWRITE,
    config("provider = \"provider\"\n"),
    Setup::Trusted,
    &["--dry-run"][..],
    None,
    Lift::Flags(&["--reviewer-model", "reviewer-model"])
)]
fn each_gate_alone_keeps_the_provider_from_starting(
    #[case] docs: (&str, &str),
    #[case] config: String,
    #[case] setup: Setup,
    #[case] flags: &[&str],
    #[case] refusal: Option<&str>,
    #[case] lift: Lift,
) {
    let world = World::new(docs, &config);
    if matches!(setup, Setup::Trusted) {
        world.trust();
    }

    let out = world.review(flags);

    if let Some(code) = refusal {
        assert_refused(&out, code);
    } else {
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert!(plan(&out).get("report").is_none());
    }
    assert_eq!(world.spawns(), 0, "the plugin started");
    assert_lifting_the_gate_starts_it_once(&world, flags, &lift);
}

/// A plugin that declares no provider hook is not a model provider, and it never starts. The
/// same plugin with a provider hook starts.
#[test]
fn a_plugin_without_a_provider_hook_never_starts() {
    let world = World::with_hooks(REWRITE, &configured(), "[]");
    world.trust();

    let out = world.review(&[]);

    assert_refused(&out, "review-provider-missing");
    assert_eq!(world.spawns(), 0, "the plugin started");
    let control = World::new(REWRITE, &configured());
    control.trust();
    assert_lifting_the_gate_starts_it_once(&control, &[], &Lift::Flags(&[]));
}

/// `lint` leaves a configured provider idle (policies.md N11).
#[test]
fn lint_leaves_a_configured_provider_idle() {
    let world = World::new(REWRITE, &configured());
    world.trust();

    let out = world.slopvac().args(["lint", "new.md"]).output().unwrap();

    assert!(matches!(out.status.code(), Some(0 | 1)), "{}", stderr(&out));
    assert_eq!(world.spawns(), 0, "lint started the provider");
    assert_lifting_the_gate_starts_it_once(&world, &[], &Lift::Flags(&[]));
}

/// The model sent is the one `--reviewer-model` names, else `[review] model`.
#[rstest]
#[case::flag_over_config(Some("flag-model"), Some("config-model"), "flag-model")]
#[case::config_alone(None, Some("config-model"), "config-model")]
#[case::flag_alone(Some("flag-model"), None, "flag-model")]
fn the_reviewer_model_flag_wins_over_the_config(
    #[case] flag: Option<&str>,
    #[case] configured: Option<&str>,
    #[case] sent: &str,
) {
    let model = configured.map_or_else(String::new, |m| format!("model = \"{m}\"\n"));
    let world = World::new(
        REWRITE,
        &config(&format!("provider = \"provider\"\n{model}")),
    );
    world.trust();
    let mut flags = Vec::new();
    if let Some(flag) = flag {
        flags.extend(["--reviewer-model", flag]);
    }

    let out = world.review(&flags);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let requests = world.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["model"], sent);
    assert_eq!(plan(&out)["reviewer_model"], sent);
}

/// The JSON text of each value of a request, as written: in the plan that `--dry-run` prints,
/// or in the params the plugin received. Unknown members fail the read, so neither side holds a
/// value the other lacks (the params add only the hook id).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRequest<'a> {
    #[serde(borrow)]
    task: &'a RawValue,
    #[serde(borrow)]
    model: &'a RawValue,
    #[serde(borrow)]
    items: Vec<RawItem<'a>>,
    /// The hook id, which only the params hold.
    #[serde(rename = "provider", default)]
    _provider: Option<serde::de::IgnoredAny>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawItem<'a> {
    #[serde(borrow)]
    id: &'a RawValue,
    #[serde(borrow)]
    instruction: &'a RawValue,
    #[serde(borrow)]
    text: &'a RawValue,
}

#[derive(Deserialize)]
struct RawPlan<'a> {
    #[serde(borrow)]
    request: RawRequest<'a>,
}

impl RawRequest<'_> {
    /// Each value's JSON text, named by its place in the request.
    fn values(&self) -> Vec<(String, &str)> {
        let mut values = vec![
            ("task".to_owned(), self.task.get()),
            ("model".to_owned(), self.model.get()),
        ];
        for (n, item) in self.items.iter().enumerate() {
            values.push((format!("items[{n}].id"), item.id.get()));
            values.push((format!("items[{n}].instruction"), item.instruction.get()));
            values.push((format!("items[{n}].text"), item.text.get()));
        }
        values
    }
}

/// TM-19: the `task`, `model` and `items` the provider receives equal the `request` in the
/// `--dry-run` plan of the same project. The params the plugin received, without the hook id,
/// must equal the plan's `request` as one whole value, so a missing, extra or reordered item
/// or member fails. The JSON text of every value must also match byte for byte on both sides,
/// the plan on stdout and the request body. Only the whitespace between values differs,
/// because the plan is indented.
#[rstest]
#[case::update(UPDATE, "update")]
#[case::rewrite(REWRITE, "rewrite")]
#[case::escaped_text(ESCAPED, "rewrite")]
fn the_provider_receives_the_request_that_a_dry_run_prints(
    #[case] docs: (&str, &str),
    #[case] task: &str,
) {
    let world = World::new(docs, &configured());
    world.trust();
    let dry = world.review(&["--type", task, "--dry-run"]);
    assert_eq!(dry.status.code(), Some(0), "{}", stderr(&dry));
    assert!(
        plan(&dry)["request"].is_object(),
        "the dry run prints a request: {}",
        String::from_utf8_lossy(&dry.stdout)
    );

    let live = world.review(&["--type", task]);

    let bodies = world.raw_requests();
    assert_eq!(bodies.len(), 1);
    let mut received: Value = serde_json::from_str(&bodies[0]).expect("the request body");
    let mut params = received["params"].take();
    let members = params.as_object_mut().expect("the params are an object");
    assert!(
        members.remove("provider").is_some(),
        "the params name the hook: {}",
        bodies[0]
    );
    assert_eq!(
        params,
        plan(&dry)["request"],
        "the params, without the hook id, equal the dry run's request"
    );
    assert_eq!(live.status.code(), Some(0), "{}", stderr(&live));
    let printed: RawPlan<'_> = serde_json::from_slice(&dry.stdout).expect("the dry-run plan");
    let body: RawBody<'_> = serde_json::from_str(&bodies[0]).expect("the request body");
    assert_eq!(body.params.values(), printed.request.values());
    assert_eq!(plan(&live)["request"], plan(&dry)["request"]);
}

/// A request message as the plugin received it.
#[derive(Deserialize)]
struct RawBody<'a> {
    #[serde(borrow)]
    params: RawRequest<'a>,
}

/// A dry run with no reviewer model still prints the request a call would send, without a
/// `model`.
#[test]
fn a_dry_run_without_a_reviewer_model_prints_the_request_without_one() {
    let world = World::new(REWRITE, &config("provider = \"provider\"\n"));
    world.trust();

    let out = world.review(&["--dry-run"]);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let plan = plan(&out);
    assert_eq!(plan["request"]["task"], "rewrite", "{plan}");
    assert!(plan["request"].get("model").is_none(), "{plan}");
}

/// A plan that would call no provider holds no `request`: a mechanical review and a payload with
/// no items. The control, the same project as a `rewrite`, holds one.
#[rstest]
#[case::mechanical(MECHANICAL, &["--dry-run"][..])]
#[case::no_items(UNCHANGED, &["--dry-run", "--type", "update"][..])]
fn a_plan_that_calls_no_provider_holds_no_request(
    #[case] docs: (&str, &str),
    #[case] flags: &[&str],
) {
    let world = World::new(docs, &configured());

    let out = world.review(flags);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let printed = plan(&out);
    assert!(printed.get("request").is_none(), "{printed}");
    let control = world.review(&["--dry-run", "--type", "rewrite"]);
    assert_eq!(control.status.code(), Some(0), "{}", stderr(&control));
    assert!(
        plan(&control).get("request").is_some(),
        "control: the rewrite plan holds a request"
    );
}

fn notices(plan: &Value) -> Vec<Value> {
    plan["notices"].as_array().cloned().unwrap_or_default()
}

fn notice<'a>(list: &'a [Value], code: &str) -> &'a Value {
    let found: Vec<&Value> = list.iter().filter(|n| n["code"] == code).collect();
    assert_eq!(found.len(), 1, "one {code} notice: {list:?}");
    found[0]
}

/// A reviewer named as the author's model gets the pre-call notice, located nowhere, and the
/// review still passes. The provider then reports that model for every item, so the second
/// phase adds its own located notice (policies.md section 1.5).
#[test]
fn a_reviewer_on_the_author_model_gets_a_pre_call_notice_and_the_review_passes() {
    let world = World::new(
        REWRITE,
        &config("provider = \"provider\"\nmodel = \"same-model\"\nauthor_model = \"same-model\"\n"),
    );
    world.trust();

    let out = world.review(&[]);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let plan = plan(&out);
    let list = notices(&plan);
    let same: Vec<&Value> = list
        .iter()
        .filter(|n| n["code"] == "review-model-same")
        .collect();
    assert_eq!(same.len(), 2, "one notice per phase: {list:?}");
    assert_eq!(same[0]["level"], "warning");
    assert_eq!(same[0]["data"]["reviewer_source"], "config");
    assert!(same[0]["data"].get("items").is_none(), "{}", same[0]);
    assert_eq!(same[1]["data"]["reviewer_source"], "reported");
    assert_eq!(
        same[1]["data"]["items"],
        json!([
            {"role": "original_document", "line": 1},
            {"role": "revised_document", "line": 1}
        ])
    );
    assert!(plan["report"].is_string());
}

/// A model the provider reports for an item is compared after the call. The notices name the
/// item by its place in the payload, whatever order the reply came in, and the review passes.
#[test]
fn reported_models_get_located_notices_and_the_review_passes() {
    let world = World::new(
        REWRITE,
        &config(
            "provider = \"provider\"\nmodel = \"reviewer-model\"\nauthor_model = \"author-model\"\n",
        ),
    );
    world.trust();
    world.script(&json!({"order": "reverse", "models": {"2": "vendor/author-model#high"}}));

    let out = world.review(&[]);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let plan = plan(&out);
    let list = notices(&plan);
    let located = json!([{"role": "revised_document", "line": 1}]);
    let same = notice(&list, "review-model-same");
    assert_eq!(same["level"], "warning");
    assert_eq!(same["data"]["reviewer_source"], "reported");
    assert_eq!(same["data"]["items"], located);
    let mismatch = notice(&list, "review-model-reported-mismatch");
    assert_eq!(mismatch["level"], "info");
    assert_eq!(mismatch["data"]["requested"], "reviewer-model");
    assert_eq!(mismatch["data"]["requested_source"], "config");
    assert_eq!(mismatch["data"]["reported"], "vendor/author-model#high");
    assert_eq!(mismatch["data"]["items"], located);
}

/// The provider's text and the model it reports are plugin-origin text. They reach the plan
/// with every bidirectional and other unsafe control escaped (threat model O1 to O4), so no
/// control is live in the JSON on stdout.
#[test]
fn provider_text_and_reported_models_reach_the_plan_escaped() {
    let world = World::new(REWRITE, &configured());
    world.trust();
    world.script(&json!({
        "text": {"1": "safe\u{202e}txet"},
        "models": {"2": "vendor\u{2066}model"}
    }));

    let out = world.review(&[]);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let raw = String::from_utf8_lossy(&out.stdout);
    assert!(!raw.contains('\u{202e}'), "a live override on stdout");
    assert!(!raw.contains('\u{2066}'), "a live isolate on stdout");
    let plan = plan(&out);
    assert!(
        plan["report"]
            .as_str()
            .is_some_and(|r| r.contains("safe\\u{202e}txet")),
        "{plan}"
    );
    let list = notices(&plan);
    let mismatch = notice(&list, "review-model-reported-mismatch");
    assert_eq!(mismatch["data"]["reported"], "vendor\\u{2066}model");
}

/// A reply that passes, then a framing violation in answer to `shutdown`: the plugin broke the
/// protocol, so the review stops under the host code and prints no plan.
#[test]
fn a_shutdown_violation_after_a_valid_reply_stops_the_review() {
    let world = World::new(REWRITE, &configured());
    world.trust();
    world.script(&json!({"shutdown": "garbage"}));

    let out = world.review(&[]);

    assert_refused(&out, "plugin-protocol-violation");
    assert!(out.stdout.is_empty(), "no plan on a failed run");
}

/// A request that failed first keeps its own code when the shutdown fails too.
#[test]
fn a_request_failure_keeps_its_code_when_the_shutdown_fails_too() {
    let world = World::new(REWRITE, &configured());
    world.trust();
    world.script(&json!({"omit": ["2"], "shutdown": "garbage"}));

    let out = world.review(&[]);

    assert_refused(&out, "review-provider-reply-invalid");
    assert!(
        !stderr(&out).contains("error[plugin-protocol-violation]"),
        "{}",
        stderr(&out)
    );
}

/// The manifest declares two provider hooks and the handshake offers only the second. The
/// request goes to the hook the plugin offered.
#[test]
fn the_request_goes_to_the_provider_hook_the_handshake_offered() {
    let world = World::with_hooks(
        REWRITE,
        &configured(),
        "[{ kind = \"provider\", id = \"first\" }, { kind = \"provider\", id = \"review\" }]",
    );
    world.trust();
    world.script(&json!({"offer_hooks": [{"kind": "provider", "id": "review"}]}));

    let out = world.review(&[]);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let requests = world.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["provider"], "review");
}

const UNCHANGED: (&str, &str) = (
    "# Cache\n\nThe cache holds 10 entries. It evicts the oldest entry first.\n",
    "# Cache\n\nThe cache holds 10 entries. It evicts the oldest entry first.\n",
);

/// An update of identical files has nothing to send, so it needs no reviewer model and starts
/// no plugin.
#[test]
fn an_update_with_nothing_to_send_needs_no_reviewer_model() {
    let world = World::new(UNCHANGED, &config("provider = \"provider\"\n"));
    world.trust();

    let out = world.review(&["--type", "update"]);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let plan = plan(&out);
    assert_eq!(plan["payload"]["items"], json!([]), "{plan}");
    assert!(plan.get("report").is_none(), "{plan}");
    assert_eq!(world.spawns(), 0, "the plugin started");
}

/// A failure of the plugin, from the start to the call, shows its host code and none of the
/// text the plugin supplied: an error answer, an exit with a standard error tail, a framing
/// error that quotes a key, and a handshake that names a feature or a version.
#[rstest]
#[case::initialize_error(json!({"init": {"error": -32603}}), "internal-error")]
#[case::initialize_version(
    json!({"init": {"error": -32001, "supported": "PLUGIN-SECRET-MESSAGE"}}),
    "protocol-version-unsupported"
)]
#[case::exit_before_the_handshake(json!({"init": "exit"}), "plugin-exited")]
#[case::duplicate_key_in_the_handshake(json!({"init": "duplicate_key"}), "plugin-protocol-violation")]
#[case::required_feature(json!({"init": "required_feature"}), "required-feature-missing")]
#[case::exit_during_the_call(json!({"complete": "exit"}), "plugin-exited")]
#[case::duplicate_key_in_the_reply(json!({"complete": "duplicate_key"}), "plugin-protocol-violation")]
fn a_plugin_failure_shows_its_code_and_none_of_the_plugins_text(
    #[case] script: Value,
    #[case] code: &str,
) {
    let world = World::new(REWRITE, &configured());
    world.trust();
    world.script(&script);

    let out = world.review(&[]);

    assert_refused(&out, code);
    let err = stderr(&out);
    for secret in [
        "PLUGIN-SECRET-MESSAGE",
        "PLUGIN-SECRET-KEY",
        "PLUGIN-SECRET-FEATURE",
    ] {
        assert!(!err.contains(secret), "{secret} on stderr:\n{err}");
    }
}

/// An item that the provider failed keeps its code when the shutdown fails after it.
#[test]
fn an_item_error_keeps_its_code_when_the_shutdown_fails() {
    let world = World::new(REWRITE, &configured());
    world.trust();
    world.script(&json!({"errors": {"1": -32006}, "shutdown": "garbage"}));

    let out = world.review(&[]);

    assert_refused(&out, "document-invalid");
    assert!(
        !stderr(&out).contains("error[plugin-protocol-violation]"),
        "{}",
        stderr(&out)
    );
}

/// A `shutdown` that the plugin does not acknowledge reports why, under the host code of that
/// failure: an error answer, an exit before the answer, or an answer that is not null.
#[rstest]
#[case::error_answer("error", "internal-error")]
#[case::exit_before_the_answer("exit", "plugin-exited")]
#[case::answer_not_null("not_null", "plugin-protocol-violation")]
fn a_shutdown_without_an_acknowledgement_reports_why(#[case] how: &str, #[case] code: &str) {
    let world = World::new(REWRITE, &configured());
    world.trust();
    world.script(&json!({"shutdown": how}));

    let out = world.review(&[]);

    assert_refused(&out, code);
    assert!(out.stdout.is_empty(), "no plan on a failed run");
}

/// An update of identical files has nothing to send, so it needs no network and no provider.
#[rstest]
#[case::offline(configured(), true)]
#[case::no_provider(config("model = \"reviewer-model\"\n"), false)]
fn an_update_with_nothing_to_send_needs_no_provider_and_no_network(
    #[case] config: String,
    #[case] offline: bool,
) {
    let world = World::new(UNCHANGED, &config);
    world.trust();
    let mut cmd = world.slopvac().args([
        "review",
        "new.md",
        "--original",
        "old.md",
        "--type",
        "update",
    ]);
    if offline {
        cmd = cmd.env("SLOPVAC_OFFLINE", "1");
    }

    let out = cmd.output().expect("run");

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let plan = plan(&out);
    assert!(plan.get("report").is_none(), "{plan}");
    assert_eq!(world.spawns(), 0, "the plugin started");
}

/// A reviewer model whose name holds a backslash is the model the provider reports, so no
/// mismatch notice appears.
#[test]
fn a_reported_model_with_a_backslash_matches_the_requested_one() {
    let world = World::new(
        REWRITE,
        &config("provider = \"provider\"\nmodel = 'team\\reviewer'\n"),
    );
    world.trust();

    let out = world.review(&[]);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let plan = plan(&out);
    let list = notices(&plan);
    assert!(
        list.iter()
            .all(|n| n["code"] != "review-model-reported-mismatch"),
        "{plan}"
    );
}

/// An answer that ends with CRLF loses the whole line ending, so no escaped carriage return
/// shows in the report.
#[test]
fn an_answer_with_a_crlf_ending_leaves_no_escaped_carriage_return() {
    let world = World::new(REWRITE, &configured());
    world.trust();
    world.script(&json!({"text": {"1": "first answer\r\n"}}));

    let out = world.review(&[]);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let plan = plan(&out);
    let report = plan["report"].as_str().expect("a report");
    assert!(report.contains("first answer"), "{report}");
    assert!(!report.contains("\\u{d}"), "{report}");
}

/// The plan's `report` starts with the untrusted marker from the catalog and a blank line, then
/// the first item's heading, and the plan gains no field for it (threat model TM-21).
#[test]
fn the_report_starts_with_the_untrusted_marker_and_the_plan_gains_no_field() {
    let world = World::new(REWRITE, &configured());
    world.trust();

    let out = world.review(&[]);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let plan = plan(&out);
    let report = plan["report"].as_str().unwrap_or_default();
    let start = format!("{}\n\n### Item 1 (", untrusted_line());
    assert!(report.starts_with(&start), "{report:?}");
    let mut keys: Vec<&str> = plan
        .as_object()
        .map(|o| o.keys().map(String::as_str).collect())
        .unwrap_or_default();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "notices",
            "payload",
            "reason",
            "report",
            "request",
            "review_type",
            "reviewer_model",
            "reviewer_model_checked",
            "schema_version",
            "scope",
            "stats",
            "uses_provider",
        ],
        "{plan}"
    );
}

/// With trace logging on, the provider's standard error reaches the log as a length only, so
/// a provider cannot echo the document into the log (policies.md section 1.7).
#[test]
fn the_providers_standard_error_reaches_the_log_as_a_length_only() {
    let world = World::new(REWRITE, &configured());
    world.trust();
    world.script(&json!({"stderr": "PLUGIN-SECRET-MESSAGE"}));

    let out = world
        .slopvac()
        .args(["review", "new.md", "--original", "old.md"])
        .env("SLOPVAC_LOG", "trace")
        .output()
        .expect("run");

    let err = stderr(&out);
    assert!(!err.contains("PLUGIN-SECRET-MESSAGE"), "{err}");
    assert!(
        err.lines()
            .any(|l| l.contains("DEBUG") && l.contains("bytes=21")),
        "the line is logged by its length:\n{err}"
    );
    assert_eq!(out.status.code(), Some(0), "{err}");
}

/// An `offline` inherited through `extends` keeps a trusted provider unstarted.
#[test]
fn an_offline_setting_inherited_through_extends_starts_no_provider() {
    let world = World::new(
        REWRITE,
        "schema_version = 3\nextends = [\"base.toml\"]\n\n[review]\n\
         provider = \"provider\"\nmodel = \"reviewer-model\"\n",
    );
    fs::write(
        world.project().join("base.toml"),
        "schema_version = 3\n\n[network]\noffline = true\n",
    )
    .unwrap();
    world.trust();

    let out = world.review(&[]);

    assert_refused(&out, "offline-network-needed");
    assert_eq!(world.spawns(), 0, "the plugin started");
}

/// A reported model whose name holds a backslash shows it escaped once in the notice text.
#[test]
fn a_reported_model_with_a_backslash_is_escaped_once_in_the_notice() {
    let world = World::new(REWRITE, &configured());
    world.trust();
    world.script(&json!({"models": {"1": "vendor\\model"}}));

    let out = world.review(&[]);

    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let plan = plan(&out);
    let list = notices(&plan);
    let message = notice(&list, "review-model-reported-mismatch")["message"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(message.contains(r#""vendor\\model""#), "{message}");
    assert!(!message.contains(r"vendor\\\\model"), "{message}");
}
