//! `lint --mode code-comments` reads the comments of source files and TOML files through the
//! engine (`docs/contract/policies.md`, 3.5). A rule that is on by default and cannot read
//! comments goes into the `info` notice `comment-rules-not-run`, and the run stays complete.
//! A rule that a config `rules` entry names and that cannot run makes the run incomplete.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a panic is a failed test"
)]

use std::fs;

use rstest::rstest;
use serde_json::{Value, json};
use snapbox::cmd::{Command, cargo_bin};

/// A project config with no setting. No core rule limits its genres, so the genre of a file does
/// not change which rules run.
const CONFIG: &str = "schema_version = 3\n";

const CLEAN: &str = "// Parse the input.\nfn main() {}\n";

/// The default rules that read no comment, sorted: the rules of
/// kind metric, structure and vocabulary in `crates/slopvac-rules/tests/fixtures/core-v3`
/// and no capitalization rule that checks the sentence style, and the vocabulary rule
/// `core:terms.casing` of the terms rule set (`slopvac-engine/src/terms.rs`).
const NOT_RUN_BY_DEFAULT: [&str; 19] = [
    "core:ai-tells-content-shape.adjective-per-noun-spray",
    "core:ai-tells-formatting.bold-spray",
    "core:ai-tells-formatting.em-dash-density",
    "core:ai-tells-formatting.heading-hierarchy",
    "core:ai-tells-formatting.inline-header-list",
    "core:prose-craft.sentence-length",
    "core:prose-discipline.run-on",
    "core:prose-format.prose-block",
    "core:ste-descriptive.paragraph-too-many-sentences",
    "core:ste-descriptive.sentence-too-long-descriptive",
    "core:ste-nouns.multiword-noun-too-long",
    "core:ste-procedural.sentence-too-long-procedural",
    "core:ste-punctuation.colon-terminates-sentence-for-count",
    "core:ste-sentences.complex-text-not-in-vertical-list",
    "core:ste-verbs.verb-form-not-listed",
    "core:ste-words.verb-or-adjective-form-not-permitted",
    "core:ste-words.word-outside-controlled-vocabulary",
    "core:ste-words.word-used-in-wrong-part-of-speech",
    "core:terms.casing",
];

/// Lint `file` (relative to a fresh project with `config`) and return the exit code and report.
fn lint(config: &str, file: &str, source: &str) -> (Option<i32>, Value) {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("slopvac.toml"), config).expect("write config");
    let path = dir.path().join(file);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(&path, source).expect("write source");
    let mut cmd = Command::new(cargo_bin!("slopvac"));
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd = cmd.env_remove(var);
    }
    let out = cmd
        .env("NO_COLOR", "1")
        .env("SLOPVAC_NO_UPDATE_CHECK", "1")
        .current_dir(dir.path())
        .args(["lint", "--mode", "code-comments", "--format", "json", file])
        .output()
        .expect("run");
    let report = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("a JSON report ({e}): {out:?}"));
    (out.status.code(), report)
}

/// The `(code, level, data.rules)` of each `unchecked` notice of the first document.
fn unchecked(report: &Value) -> Vec<(String, String, Value)> {
    report["documents"][0]["unchecked"]
        .as_array()
        .expect("unchecked")
        .iter()
        .map(|n| {
            (
                n["code"].as_str().expect("code").to_owned(),
                n["level"].as_str().expect("level").to_owned(),
                n["data"]["rules"].clone(),
            )
        })
        .collect()
}

#[rstest]
#[case::rust("src/a.rs", CLEAN)]
#[case::toml("src/a.toml", "# Name of the package.\nname = \"a\"\n")]
fn a_clean_comment_passes_and_lists_the_rules_that_read_no_comment(
    #[case] file: &str,
    #[case] source: &str,
) {
    let (code, report) = lint(CONFIG, file, source);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(
        unchecked(&report),
        [(
            "comment-rules-not-run".to_owned(),
            "info".to_owned(),
            json!(NOT_RUN_BY_DEFAULT)
        )]
    );
}

#[test]
fn a_comment_finding_over_the_gate_fails_the_run() {
    let (code, report) = lint(
        CONFIG,
        "src/a.rs",
        "// A silver bullet for parsing.\nfn main() {}\n",
    );
    assert_eq!(code, Some(1), "{report}");
    let rules: Vec<&str> = report["documents"][0]["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|f| f["rule_id"].as_str().expect("rule_id"))
        .collect();
    assert_eq!(rules, ["core:orwell.stale-figure"], "{report}");
}

#[test]
fn a_rule_named_in_config_that_reads_no_comment_makes_the_run_incomplete() {
    let config = format!("{CONFIG}\n[rules]\n\"core:prose-craft.sentence-length\" = \"warning\"\n");
    let (code, report) = lint(&config, "src/a.rs", CLEAN);
    assert_eq!(code, Some(2), "{report}");
    let notices = unchecked(&report);
    let named: Vec<_> = notices
        .iter()
        .filter(|(code, ..)| code == "comment-rule-named-not-run")
        .collect();
    assert_eq!(
        named,
        [&(
            "comment-rule-named-not-run".to_owned(),
            "warning".to_owned(),
            json!(["core:prose-craft.sentence-length"])
        )]
    );
}

#[test]
fn a_category_option_alone_names_no_rule() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("slopvac.toml"), CONFIG).expect("write config");
    fs::create_dir_all(dir.path().join("src")).expect("mkdir");
    fs::write(dir.path().join("src/a.rs"), CLEAN).expect("write source");
    let out = Command::new(cargo_bin!("slopvac"))
        .env("NO_COLOR", "1")
        .env("SLOPVAC_NO_UPDATE_CHECK", "1")
        .current_dir(dir.path())
        .args([
            "lint",
            "--mode",
            "code-comments",
            "--format",
            "json",
            "--category",
            "prose-craft",
            "src/a.rs",
        ])
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let report: Value = serde_json::from_slice(&out.stdout).expect("a JSON report");
    assert_eq!(
        unchecked(&report)
            .iter()
            .map(|n| n.0.as_str())
            .collect::<Vec<_>>(),
        ["comment-rules-not-run"],
        "{report}"
    );
}
