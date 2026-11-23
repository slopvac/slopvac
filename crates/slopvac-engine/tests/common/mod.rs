//! Builders shared by the engine tests.

#![allow(
    dead_code,
    unreachable_pub,
    missing_docs,
    reason = "test support: each test binary uses a subset of the helpers; test support: items are shared across test binaries; test crate: nothing public to document"
)]
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test support: a panic in a helper is a failed test, and the helpers are shared across test binaries"
)]

use std::fmt::Write as _;
use std::path::PathBuf;

use serde_json::{Value, json};
use slopvac_engine::{CategoryDials, Engine, Level, Settings};
use slopvac_model::{
    CategoryId, Document, GenreId, GenreResult, GenreSource, NonEmptyString, PackId, RuleId,
    Severity,
};
use slopvac_rules::{CompileOptions, RuleSet};
use slopvac_terms::Blocklist;

pub fn id(text: &str) -> RuleId {
    RuleId::new(text).unwrap()
}

/// A tokens rule `cat.<id>` with the given extra keys merged in.
pub fn tokens(id: &str, token: &str, extra: &Value) -> Value {
    rule(id, &json!({"kind": "tokens", "tokens": [token]}), extra)
}

pub fn rule(id: &str, matcher: &Value, extra: &Value) -> Value {
    let mut rule = json!({
        "id": id, "name": "n", "message": "Found {match}.", "severity": "warning",
        "match": matcher,
        "examples": [],
        "citation": {"source": "s"}
    });
    rule.as_object_mut().unwrap().remove("examples");
    for (k, v) in extra.as_object().into_iter().flatten() {
        rule[k] = v.clone();
    }
    rule
}

pub fn category(id: &str, rules: &[Value]) -> (PathBuf, String) {
    let doc = json!({
        "schema_version": 3, "id": id, "title": "t", "description": "d", "rules": rules
    });
    (PathBuf::from(format!("{id}.json")), doc.to_string())
}

pub fn ruleset(files: Vec<(PathBuf, String)>) -> RuleSet {
    RuleSet::from_sources(
        &PackId::new("core").unwrap(),
        files,
        &CompileOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"))
}

pub fn settings() -> Settings {
    Settings::new(
        NonEmptyString::new("normal").unwrap(),
        GenreResult {
            id: GenreId::new("reference").unwrap(),
            source: GenreSource::Default,
            reasons: Vec::new(),
        },
    )
}

/// An engine over one category `cat` holding `rules`.
pub fn engine(rules: &[Value]) -> Engine {
    Engine::new(vec![ruleset(vec![category("cat", rules)])], settings()).unwrap()
}

/// `(rule id, line, column)` of each finding, in report order.
pub fn places(doc: &Document) -> Vec<(String, usize, usize)> {
    doc.findings
        .iter()
        .map(|f| (f.rule_id.to_string(), f.start.line(), f.start.column()))
        .collect()
}

/// The matched text of each finding that is not a directive error.
pub fn matched(doc: &Document) -> Vec<&str> {
    doc.findings
        .iter()
        .filter(|f| !f.rule_id.as_str().contains("meta."))
        .map(|f| f.matched_text.as_str())
        .collect()
}

/// A directory allocated for this caller alone, under the cargo test tmp directory.
///
/// `cargo nextest` runs every test in its own process, so two processes must never share
/// a directory, and nothing here deletes a directory it did not create. The name is unique
/// (`tempfile` picks it), and the directory stays until `cargo clean`: a test process has
/// no hook that runs after its last test, and its helper cannot be dropped earlier because
/// other tests of the same process still use the scripts in it.
pub fn private_dir(prefix: &str) -> PathBuf {
    tempfile::Builder::new()
        .prefix(&format!("{prefix}-"))
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
        .keep()
}

pub fn root() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn read_json(path: &std::path::Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn level(name: &str) -> Level {
    match name {
        "error" => Level::On(Severity::Error),
        "warning" => Level::On(Severity::Warning),
        "suggestion" => Level::On(Severity::Suggestion),
        "off" => Level::Off,
        other => panic!("unknown level {other}"),
    }
}

/// The rule that the converted 2.x data still writes as a pattern. Rust runs it as the native
/// `capitalization` kind, so its findings check that kind against the 2.x pattern.
pub const TITLE_CASE: &str = "title-case-heading";

fn core_sources() -> Vec<(PathBuf, String)> {
    let core = root().join("crates/slopvac-rules/tests/fixtures/core-v3");
    let mut files: Vec<PathBuf> = std::fs::read_dir(core)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    let mut swapped = 0;
    let sources = files
        .into_iter()
        .map(|path| {
            let mut doc = read_json(&path);
            for rule in doc["rules"].as_array_mut().into_iter().flatten() {
                if rule["id"] == TITLE_CASE {
                    rule["match"] = json!({"kind": "capitalization", "style": "sentence"});
                    swapped += 1;
                }
            }
            (path, doc.to_string())
        })
        .collect();
    assert_eq!(swapped, 1, "{TITLE_CASE} is one rule of the core data");
    sources
}

/// The engine for one profile: the category dials and rule levels of `core:legacy-<profile>`
/// and the thresholds of the profile, both from the profile mapping contract.
pub fn legacy_engine(profile: &str) -> Engine {
    Engine::new(vec![core_set()], legacy_settings(profile)).unwrap()
}

/// The engine for a 2.x config that sets a word blocklist: the legacy style, then its
/// vocabulary companion `core:legacy-<profile>-vocabulary`, which is what `slopvac migrate
/// --preserve-behavior` lists when a blocklist moves. The legacy style leaves the vocabulary
/// rules off.
pub fn legacy_engine_with_blocklist(profile: &str, blocklist: &Blocklist) -> Engine {
    let mut settings = legacy_settings(profile);
    let companion: toml::Table = std::fs::read_to_string(root().join(format!(
        "docs/contract/compat/profiles/legacy-{profile}-vocabulary.toml"
    )))
    .unwrap()
    .parse()
    .unwrap();
    for (id, value) in companion["rules"].as_table().unwrap() {
        settings.rule_levels.insert(
            RuleId::new(format!("core:{id}")).unwrap(),
            level(value.as_str().unwrap()),
        );
    }
    Engine::new(vec![core_set()], settings)
        .unwrap()
        .with_blocklist(blocklist)
        .unwrap()
}

/// The core data with the title-case swap, as the engine reads it.
pub fn core_set() -> RuleSet {
    RuleSet::from_sources(
        &PackId::new("core").unwrap(),
        core_sources(),
        &CompileOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{e}"))
}

/// The settings of `core:legacy-<profile>` and the profile thresholds.
pub fn legacy_settings(profile: &str) -> Settings {
    let mut settings = Settings::new(
        NonEmptyString::new(profile).unwrap(),
        GenreResult {
            id: GenreId::new("reference").unwrap(),
            source: GenreSource::Default,
            reasons: Vec::new(),
        },
    );
    let profiles = root().join("docs/contract/compat/profiles");
    let style: toml::Table =
        std::fs::read_to_string(profiles.join(format!("legacy-{profile}.toml")))
            .unwrap()
            .parse()
            .unwrap();
    for (name, dials) in style["categories"].as_table().unwrap() {
        settings.categories.insert(
            CategoryId::new(name).unwrap(),
            CategoryDials::default()
                .with_level(Some(level(dials["severity"].as_str().unwrap())))
                .with_weight(dials.get("weight").and_then(toml::Value::as_float))
                .with_max_per_100_words(
                    dials
                        .get("max_per_100_words")
                        .and_then(toml::Value::as_float),
                ),
        );
    }
    for (id, value) in style["rules"].as_table().unwrap() {
        settings.rule_levels.insert(
            RuleId::new(format!("core:{id}")).unwrap(),
            level(value.as_str().unwrap()),
        );
    }
    let mapping = read_json(&profiles.join("mapping.json"));
    let t = &mapping["profiles"][profile]["thresholds"];
    let count = |k: &str| t[k].as_u64().map(|n| usize::try_from(n).unwrap());
    settings.thresholds.max_errors = count("max_errors");
    settings.thresholds.max_warnings = count("max_warnings");
    settings.thresholds.max_unicode_dashes = count("max_unicode_dashes");
    settings.thresholds.max_total_per_100_words = t["max_total_per_100_words"].as_f64();
    settings.thresholds.min_score = t["min_score"].as_f64();
    settings
}

/// An engine over the one category file `category` with `settings`.
pub fn engine_with(category: &Value, settings: Settings) -> Engine {
    let files = vec![(PathBuf::from("cat.json"), category.to_string())];
    Engine::new(vec![ruleset(files)], settings).unwrap()
}

/// Run `f` on a worker thread and return its result, failing the test when it blocks.
///
/// - The worker ends early without a result (a panic): join at once and re-raise its panic.
/// - Five seconds pass with no result: open the FIFO's write end without blocking, which wakes a
///   reader stuck in `open`. `ENXIO` means no reader is waiting, so there is nothing to wake.
///   Then wait once more, join the worker and fail; a worker that is still stuck is left behind,
///   so recovery never hangs.
pub fn within_timeout<T: Send + 'static>(
    fifo: &std::path::Path,
    f: impl FnOnce() -> T + Send + 'static,
) -> T {
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::Duration;

    const O_NONBLOCK: i32 = if cfg!(target_os = "linux") {
        0o4000
    } else {
        0x4
    };
    const LIMIT: Duration = Duration::from_secs(5);

    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(LIMIT) {
        Ok(value) => {
            worker.join().expect("the worker finished");
            value
        }
        Err(RecvTimeoutError::Disconnected) => match worker.join() {
            Err(panic) => std::panic::resume_unwind(panic),
            Ok(()) => panic!("the worker ended without a result"),
        },
        Err(RecvTimeoutError::Timeout) => {
            let _writer = std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(O_NONBLOCK)
                .open(fifo);
            match rx.recv_timeout(LIMIT) {
                Ok(_) | Err(RecvTimeoutError::Disconnected) => {
                    let _ = worker.join();
                    panic!("the read blocked on a FIFO until it was woken");
                }
                Err(RecvTimeoutError::Timeout) => panic!("the read blocked on a FIFO"),
            }
        }
    }
}

/// The table: a left cell of `lines` lines that each read `Must go.`, and `x` beside the first.
/// An RST grid table with a left cell of `lines` lines that each read `Must go.`, and `x`
/// beside the first line.
pub fn wrapped_table(lines: usize) -> String {
    let border = format!("+{}+{}+\n", "-".repeat(12), "-".repeat(5));
    let mut source = border.clone();
    for line in 0..lines {
        let right = if line == 0 { "x" } else { " " };
        writeln!(source, "| Must go.   | {right}   |").unwrap();
    }
    source.push_str(&border);
    source
}

/// The driver output for the table.
fn wrapped_driver_output(lines: usize) -> String {
    let left = vec!["Must go."; lines].join("\\n");
    let cell = |end: usize, column: usize, text: &str| {
        format!(
            r#"{{"tag":"paragraph","start":2,"end":{end},"containers":[],"cell":[0,{column},1],"pieces":[{{"k":"t","t":"{text}"}}]}}"#
        )
    };
    format!(
        r#"{{"docutils":"0.21.2","nodes":[{{"tag":"table","table":true,"start":1,"end":{},"containers":[]}},{},{}]}}"#,
        lines + 2,
        cell(lines + 1, 0, &left),
        cell(2, 1, "x"),
    )
}

/// RST options whose interpreter prints the driver output for [`wrapped_table`]. Both cells
/// are paragraphs, and the left one wraps over `lines` lines.
#[cfg(unix)]
pub fn wrapped_rst(lines: usize) -> slopvac_engine::rst::RstOptions {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = private_dir("rst-split");
    let json_path = dir.join(format!("wrapped-{lines}.json"));
    std::fs::write(&json_path, wrapped_driver_output(lines)).unwrap();
    let python = dir.join(format!("wrapped-python-{lines}"));
    std::fs::write(
        &python,
        format!("#!/bin/sh\ncat >/dev/null\ncat '{}'\n", json_path.display()),
    )
    .unwrap();
    std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut options = slopvac_engine::rst::RstOptions::default();
    options.interpreter = Some(python);
    options
}
