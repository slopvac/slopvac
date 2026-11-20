//! Behaviour tests for the review workflow: type, scope, models, payloads, consent.

use std::cell::Cell;
use std::fs;
use std::path::Path;

use rstest::rstest;
use slopvac_agents::claims::compare;
use slopvac_agents::error::AgentsError;
use slopvac_agents::prime::{self, Level, Settings};
use slopvac_agents::review::{
    Approval, ChangeStats, ModelProvider, Payload, ReviewEnv, ReviewType, Scope,
    check_reviewer_model, command, payload, run, select_type,
};

const ORIGINAL: &str = include_str!("fixtures/claims/original.md");
const REVISED: &str = include_str!("fixtures/claims/revised.md");

fn stats(
    original_sentences: usize,
    churn: usize,
    literal_claims: usize,
    fact_candidates: usize,
) -> ChangeStats {
    ChangeStats {
        original_sentences,
        churn,
        literal_claims,
        fact_candidates,
    }
}

#[test]
fn type_is_chosen_from_change_size_claim_density_and_genre() {
    let cases = [
        (stats(40, 4, 0, 0), None, ReviewType::Mechanical),
        (stats(100, 20, 0, 0), None, ReviewType::Mechanical),
        (stats(100, 21, 0, 0), None, ReviewType::Update),
        (stats(70, 20, 0, 0), None, ReviewType::Update),
        (stats(40, 4, 1, 0), None, ReviewType::Update),
        (stats(40, 4, 0, 1), None, ReviewType::Update),
        (stats(40, 2, 0, 0), Some("reference"), ReviewType::Update),
        (stats(40, 2, 0, 0), Some("readme"), ReviewType::Mechanical),
        (stats(40, 20, 0, 0), None, ReviewType::Rewrite),
        (stats(40, 39, 3, 2), None, ReviewType::Rewrite),
        (stats(0, 3, 0, 0), None, ReviewType::Rewrite),
    ];
    for (s, genre, expected) in cases {
        assert_eq!(select_type(&s, genre).0, expected, "{s:?} {genre:?}");
    }
}

#[test]
fn fixture_change_is_an_update_because_literals_and_facts_changed() {
    let report = compare(ORIGINAL, REVISED).unwrap();
    let s = ChangeStats::from_report(&report);
    assert_eq!(s, stats(15, 6, 3, 3));
    let (kind, reason) = select_type(&s, None);
    assert_eq!(kind, ReviewType::Update);
    assert!(reason.contains("literal"), "{reason}");
}

#[test]
fn scope_budget_scales_with_the_change_and_asks_above_it() {
    let scope = Scope::for_report(&compare(ORIGINAL, REVISED).unwrap());
    assert_eq!((scope.required, scope.extra_budget), (6, 8));
    assert!(scope.admit(8, Approval::Ask, false).is_ok());
    let err = scope.admit(9, Approval::Ask, false).unwrap_err();
    assert_eq!(err.code(), "review-scope-approval");
    assert!(err.to_string().contains("--scope-approved"));
    assert!(scope.admit(9, Approval::Auto, false).is_ok());
    assert!(scope.admit(9, Approval::Ask, true).is_ok());
    let bigger = Scope::for_report(&compare("A one. B two.", "").unwrap());
    assert!(bigger.extra_budget >= 5);
}

#[test]
fn reviewer_model_must_differ_from_the_author_model() {
    let same = [
        ("anthropic/claude-sonnet-5-5#high", "claude-sonnet-5-5"),
        ("Claude-Sonnet-5-5", "amazon-bedrock/claude-sonnet-5-5"),
    ];
    for (author, reviewer) in same {
        let err = check_reviewer_model(Some(author), reviewer).unwrap_err();
        assert_eq!(err.code(), "review-same-model", "{author} {reviewer}");
    }
    assert!(check_reviewer_model(Some("gpt-5"), "claude-sonnet-5-5").unwrap());
    assert!(!check_reviewer_model(None, "claude-sonnet-5-5").unwrap());
}

#[test]
fn payload_is_only_what_the_review_type_needs() {
    let report = compare(ORIGINAL, REVISED).unwrap();
    let mechanical = payload(ReviewType::Mechanical, &report, ORIGINAL, REVISED);
    assert_eq!(mechanical.kind, "none");
    assert_eq!(mechanical.items.len(), 0);

    let update = payload(ReviewType::Update, &report, ORIGINAL, REVISED);
    assert_eq!(update.kind, "passages");
    let roles = |role: &str| update.items.iter().filter(|i| i.role == role).count();
    assert_eq!(
        (roles("changed"), roles("added"), roles("dropped")),
        (2, 1, 3)
    );
    assert!(
        update
            .items
            .iter()
            .filter(|i| i.role == "changed")
            .all(|i| i.original.is_some())
    );
    let dropped: Vec<&str> = update
        .items
        .iter()
        .filter(|i| i.role == "dropped")
        .map(|i| i.text.as_str())
        .collect();
    assert!(dropped.contains(&"Entries expire after 24 hours."));

    let rewrite = payload(ReviewType::Rewrite, &report, ORIGINAL, REVISED);
    assert_eq!(rewrite.kind, "document");
    let texts: Vec<(&str, &str)> = rewrite
        .items
        .iter()
        .map(|i| (i.role, i.text.as_str()))
        .collect();
    assert_eq!(
        texts,
        [
            ("original_document", ORIGINAL),
            ("revised_document", REVISED)
        ]
    );
}

#[test]
fn a_deletion_only_update_still_sends_the_deleted_sentence() {
    let original = "Keep this one. Remove the 90 day limit. Keep that one.";
    let report = compare(original, "Keep this one. Keep that one.").unwrap();
    let update = payload(
        ReviewType::Update,
        &report,
        original,
        "Keep this one. Keep that one.",
    );
    assert_eq!(update.items.len(), 1);
    assert_eq!(
        (update.items[0].role, update.items[0].text.as_str()),
        ("dropped", "Remove the 90 day limit.")
    );
}

struct Mock {
    model: &'static str,
    calls: Cell<usize>,
}

impl ModelProvider for Mock {
    fn name(&self) -> &'static str {
        "mock"
    }
    fn model(&self) -> &str {
        self.model
    }
    fn review(&self, _: &Payload) -> Result<String, AgentsError> {
        self.calls.set(self.calls.get() + 1);
        Ok("no findings".into())
    }
}

fn args(dir: &Path, extra: &[&str]) -> clap::ArgMatches {
    fs::write(dir.join("old.md"), ORIGINAL).unwrap();
    fs::write(dir.join("new.md"), REVISED).unwrap();
    let mut argv = vec![
        "review".to_owned(),
        dir.join("new.md").display().to_string(),
        "--original".into(),
        dir.join("old.md").display().to_string(),
    ];
    argv.extend(extra.iter().map(|s| (*s).to_owned()));
    command().try_get_matches_from(argv).unwrap()
}

#[test]
fn dry_run_prints_the_payload_and_never_calls_the_provider() {
    let dir = tempfile::tempdir().unwrap();
    let mock = Mock {
        model: "gpt-5",
        calls: Cell::new(0),
    };
    let env = ReviewEnv {
        provider: Some(&mock),
        ..ReviewEnv::default()
    };
    let out = run(&args(dir.path(), &["--dry-run"]), &env).unwrap();
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["review_type"], "update");
    assert_eq!(json["payload"]["kind"], "passages");
    assert!(json.get("report").is_none());
    assert_eq!(mock.calls.get(), 0);
}

#[test]
fn a_run_needs_a_trusted_provider_and_an_online_session() {
    let dir = tempfile::tempdir().unwrap();
    let none = run(&args(dir.path(), &[]), &ReviewEnv::default()).unwrap_err();
    assert_eq!(none.code(), "review-provider-missing");

    let mock = Mock {
        model: "gpt-5",
        calls: Cell::new(0),
    };
    let offline = ReviewEnv {
        offline: true,
        provider: Some(&mock),
        ..ReviewEnv::default()
    };
    let err = run(&args(dir.path(), &[]), &offline).unwrap_err();
    assert_eq!(err.code(), "offline-network-needed");
    assert_eq!(err.to_string(), "offline: review needs the network");
    assert_eq!(mock.calls.get(), 0);

    let online = ReviewEnv {
        provider: Some(&mock),
        author_model: Some("claude-sonnet-5-5".into()),
        ..ReviewEnv::default()
    };
    let out = run(&args(dir.path(), &[]), &online).unwrap();
    assert!(out.contains("\"report\": \"no findings\""));
    assert_eq!(mock.calls.get(), 1);
}

#[test]
fn a_provider_on_the_authors_model_is_refused_before_any_call() {
    let dir = tempfile::tempdir().unwrap();
    let mock = Mock {
        model: "anthropic/claude-sonnet-5-5",
        calls: Cell::new(0),
    };
    let env = ReviewEnv {
        provider: Some(&mock),
        author_model: Some("claude-sonnet-5-5#high".into()),
        ..ReviewEnv::default()
    };
    let err = run(&args(dir.path(), &[]), &env).unwrap_err();
    assert_eq!(err.code(), "review-same-model");
    assert_eq!(mock.calls.get(), 0);
}

#[test]
fn extra_checks_above_the_budget_need_approval_unless_auto() {
    let dir = tempfile::tempdir().unwrap();
    let ask = ReviewEnv::default();
    let err = run(&args(dir.path(), &["--dry-run", "--extra", "9"]), &ask).unwrap_err();
    assert_eq!(err.code(), "review-scope-approval");
    run(
        &args(
            dir.path(),
            &["--dry-run", "--extra", "9", "--scope-approved"],
        ),
        &ask,
    )
    .unwrap();
    let auto = ReviewEnv {
        approval: Approval::Auto,
        ..ReviewEnv::default()
    };
    run(&args(dir.path(), &["--dry-run", "--extra", "9"]), &auto).unwrap();
}

#[test]
fn type_flag_overrides_the_choice() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(
        &args(dir.path(), &["--dry-run", "--type", "rewrite"]),
        &ReviewEnv::default(),
    )
    .unwrap();
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["review_type"], "rewrite");
    assert_eq!(json["payload"]["kind"], "document");
}

fn tree() -> clap::Command {
    clap::Command::new("slopvac")
        .subcommand(command())
        .subcommand(slopvac_agents::claims::command())
        .subcommand(clap::Command::new("lint").arg(clap::Arg::new("format").long("format")))
        .subcommand(prime::command())
        .subcommand(clap::Command::new("plugins").subcommand(clap::Command::new("trust")))
}

fn review_topic(review_type: Option<ReviewType>, approval: Approval, level: Level) -> String {
    prime::render(
        Some("review"),
        &Settings {
            level,
            review_type,
            review_approval: approval,
            ..Settings::default()
        },
        &tree(),
        Path::new("/nonexistent"),
    )
    .unwrap()
}

#[test]
fn prime_review_renders_the_steps_of_each_type() {
    let auto = review_topic(None, Approval::Ask, Level::Default);
    for name in ["mechanical", "update", "rewrite"] {
        assert!(auto.contains(&format!("- `{name}`")), "{name}");
    }
    assert!(!auto.contains("## Steps for"));
    let expect = [
        (
            ReviewType::Mechanical,
            "Confirm that no claim lists a changed literal",
        ),
        (ReviewType::Update, "Check each changed and added claim"),
        (ReviewType::Rewrite, "List the facts in the original"),
    ];
    for (kind, marker) in expect {
        let text = review_topic(Some(kind), Approval::Ask, Level::Default);
        assert!(text.contains(&format!("## Steps for {}", kind.name())));
        assert!(text.contains(marker), "{kind:?}");
        for (other, other_marker) in expect {
            if other != kind {
                assert!(!text.contains(other_marker), "{kind:?} leaks {other:?}");
            }
        }
    }
}

#[test]
fn prime_review_documents_scope_approval_and_the_reviewer_rule() {
    let ask = review_topic(None, Approval::Ask, Level::Default);
    assert!(ask.contains("ask the user to confirm the exact scope"));
    assert!(ask.contains("--scope-approved"));
    assert!(ask.contains("`review_approval = \"auto\"` in the config skips the question"));
    assert!(ask.contains("must differ from the author's"));
    let auto = review_topic(None, Approval::Auto, Level::Default);
    assert!(auto.contains("so go ahead without asking"));
}

#[test]
fn prime_review_names_only_real_commands_and_flags() {
    let problems: Vec<String> = prime::check_topics(&tree())
        .into_iter()
        .filter(|p| p.starts_with("review:"))
        .collect();
    assert_eq!(problems, Vec::<String>::new());
}
struct Recorder {
    seen: std::cell::RefCell<Vec<Payload>>,
}

impl ModelProvider for Recorder {
    fn name(&self) -> &'static str {
        "recorder"
    }
    fn model(&self) -> &'static str {
        "gpt-5"
    }
    fn review(&self, payload: &Payload) -> Result<String, AgentsError> {
        self.seen.borrow_mut().push(payload.clone());
        Ok("ok".into())
    }
}

const WORDS: [&str; 10] = [
    "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet",
];

/// Ten sentences with no literals, so a reworded one is a mechanical change.
fn prose(verb: &str) -> String {
    WORDS
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let v = if i == 2 { verb } else { "handles" };
            format!("The {w} module {v} routine requests.")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn pair(dir: &Path, old: &str, new: &str, extra: &[&str]) -> clap::ArgMatches {
    fs::write(dir.join("old.md"), old).unwrap();
    fs::write(dir.join("new.md"), new).unwrap();
    let mut argv = vec![
        "review".to_owned(),
        dir.join("new.md").display().to_string(),
        "--original".into(),
        dir.join("old.md").display().to_string(),
    ];
    argv.extend(extra.iter().map(|s| (*s).to_owned()));
    command().try_get_matches_from(argv).unwrap()
}

#[test]
fn a_mechanical_review_never_calls_a_provider_online_or_offline() {
    let dir = tempfile::tempdir().unwrap();
    let rec = Recorder {
        seen: std::cell::RefCell::new(Vec::new()),
    };
    for offline in [false, true] {
        let env = ReviewEnv {
            offline,
            provider: Some(&rec),
            ..ReviewEnv::default()
        };
        let out = run(
            &pair(dir.path(), &prose("handles"), &prose("serves"), &[]),
            &env,
        )
        .unwrap();
        let json: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(json["review_type"], "mechanical", "offline={offline}");
        assert_eq!(json["uses_provider"], false);
        assert_eq!(json["payload"]["kind"], "none");
        assert!(json.get("report").is_none());
    }
    let none = run(
        &pair(dir.path(), &prose("handles"), &prose("serves"), &[]),
        &ReviewEnv::default(),
    );
    assert!(
        none.is_ok(),
        "no provider is needed for a mechanical review"
    );
    assert_eq!(rec.seen.borrow().len(), 0);
}

#[test]
fn an_explicit_type_lighter_than_the_change_needs_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let v = |verb: &str| format!("{} Retries stop after 7 attempts.", prose(verb));
    let base = v("handles");
    let cases: [(String, Vec<&str>, &str); 6] = [
        (
            v("serves").replace("serves", "does not handle"),
            vec!["--type", "mechanical"],
            "negation",
        ),
        (
            v("serves").replace("serves", "handles 90"),
            vec!["--type", "mechanical"],
            "literal",
        ),
        (
            v("serves"),
            vec!["--type", "mechanical", "--genre", "reference"],
            "genre",
        ),
        (
            base.replace(" Retries stop after 7 attempts.", ""),
            vec!["--type", "mechanical"],
            "lost fact",
        ),
        (
            (0..10)
                .map(|i| format!("Wholly new text number {i} appears here."))
                .collect::<Vec<_>>()
                .join(" "),
            vec!["--type", "update"],
            "rewrite size",
        ),
        (
            (0..10)
                .map(|i| format!("Wholly new text number {i} appears here."))
                .collect::<Vec<_>>()
                .join(" "),
            vec!["--type", "mechanical"],
            "rewrite size",
        ),
    ];
    for (revised, extra, why) in cases {
        let err = run(
            &pair(dir.path(), &base, &revised, &extra),
            &ReviewEnv::default(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "review-type-ineligible", "{why}");
    }
    let ok = run(
        &pair(
            dir.path(),
            &base,
            &v("serves"),
            &["--type", "mechanical", "--dry-run"],
        ),
        &ReviewEnv::default(),
    );
    assert!(ok.is_ok());
    let heavier = run(
        &pair(
            dir.path(),
            &base,
            &v("serves"),
            &["--type", "rewrite", "--dry-run"],
        ),
        &ReviewEnv::default(),
    );
    assert!(heavier.is_ok(), "a heavier type is allowed");
}

#[test]
fn the_provider_receives_dropped_sentences_and_the_rewrite_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let rec = Recorder {
        seen: std::cell::RefCell::new(Vec::new()),
    };
    let env = ReviewEnv {
        provider: Some(&rec),
        ..ReviewEnv::default()
    };

    let old = format!("{} Remove the 90 day limit.", prose("handles"));
    run(
        &pair(dir.path(), &old, &prose("handles"), &["--type", "update"]),
        &env,
    )
    .unwrap();
    let sent = rec.seen.borrow()[0].clone();
    assert_eq!(sent.kind, "passages");
    assert_eq!(sent.items.len(), 1);
    assert_eq!(
        (sent.items[0].role, sent.items[0].text.as_str()),
        ("dropped", "Remove the 90 day limit.")
    );

    run(
        &pair(dir.path(), ORIGINAL, REVISED, &["--type", "rewrite"]),
        &env,
    )
    .unwrap();
    let sent = rec.seen.borrow()[1].clone();
    let texts: Vec<&str> = sent.items.iter().map(|i| i.text.as_str()).collect();
    assert_eq!(texts, [ORIGINAL, REVISED]);
}
fn plan_json(dir: &Path, extra: &[&str], env: &ReviewEnv<'_>) -> serde_json::Value {
    let out = run(&args(dir, extra), env).unwrap();
    serde_json::from_str(&out).unwrap()
}

fn notice_codes(json: &serde_json::Value) -> Vec<&str> {
    json["notices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["code"].as_str().unwrap())
        .collect()
}

#[test]
fn both_models_known_and_different_are_checked_without_a_notice() {
    let dir = tempfile::tempdir().unwrap();
    let json = plan_json(
        dir.path(),
        &[
            "--dry-run",
            "--author-model",
            "claude-sonnet-5-5",
            "--reviewer-model",
            "gpt-5",
        ],
        &ReviewEnv::default(),
    );
    assert_eq!(json["reviewer_model_checked"], true);
    assert_eq!(notice_codes(&json), Vec::<&str>::new());
}

#[test]
fn the_author_flag_names_the_author_for_the_same_model_rule() {
    let dir = tempfile::tempdir().unwrap();
    let argv = args(
        dir.path(),
        &[
            "--dry-run",
            "--author-model",
            "anthropic/gpt-5#high",
            "--reviewer-model",
            "gpt-5",
        ],
    );
    let err = run(&argv, &ReviewEnv::default()).unwrap_err();
    assert_eq!(err.code(), "review-same-model");
}

#[test]
fn author_model_comes_from_the_flag_before_the_config_value() {
    let dir = tempfile::tempdir().unwrap();
    let env = ReviewEnv {
        author_model: Some("gpt-5".into()),
        ..ReviewEnv::default()
    };
    // The flag names a different author, so a gpt-5 reviewer passes.
    let json = plan_json(
        dir.path(),
        &[
            "--dry-run",
            "--author-model",
            "claude-opus-5-5",
            "--reviewer-model",
            "gpt-5",
        ],
        &env,
    );
    assert_eq!(json["author_model"], "claude-opus-5-5");
    assert_eq!(json["reviewer_model_checked"], true);
}

#[test]
fn reviewer_model_comes_from_flag_then_config_then_provider() {
    let dir = tempfile::tempdir().unwrap();
    let mock = Mock {
        model: "from-provider",
        calls: Cell::new(0),
    };
    let env = ReviewEnv {
        provider: Some(&mock),
        reviewer_model: Some("from-config".into()),
        author_model: Some("author".into()),
        ..ReviewEnv::default()
    };
    let flag = plan_json(
        dir.path(),
        &["--dry-run", "--reviewer-model", "from-flag"],
        &env,
    );
    assert_eq!(flag["reviewer_model"], "from-flag");
    let config = plan_json(dir.path(), &["--dry-run"], &env);
    assert_eq!(config["reviewer_model"], "from-config");
    let env = ReviewEnv {
        reviewer_model: None,
        ..env
    };
    let provider = plan_json(dir.path(), &["--dry-run"], &env);
    assert_eq!(provider["reviewer_model"], "from-provider");
}

#[test]
fn an_unknown_author_leaves_the_rule_unchecked_and_names_the_author() {
    let dir = tempfile::tempdir().unwrap();
    let json = plan_json(
        dir.path(),
        &["--dry-run", "--reviewer-model", "gpt-5"],
        &ReviewEnv::default(),
    );
    assert_eq!(json["reviewer_model_checked"], false);
    assert_eq!(notice_codes(&json), ["review-model-unchecked"]);
    let message = json["notices"][0]["message"].as_str().unwrap();
    assert!(message.contains("the author model is unknown"), "{message}");
}

#[test]
fn an_unknown_reviewer_leaves_the_rule_unchecked_and_names_the_reviewer() {
    let dir = tempfile::tempdir().unwrap();
    let json = plan_json(
        dir.path(),
        &["--dry-run", "--author-model", "gpt-5"],
        &ReviewEnv::default(),
    );
    assert_eq!(json["reviewer_model_checked"], false);
    assert_eq!(notice_codes(&json), ["review-model-unchecked"]);
    let message = json["notices"][0]["message"].as_str().unwrap();
    assert!(
        message.contains("the reviewer model is unknown"),
        "{message}"
    );
}

#[test]
fn a_mechanical_review_has_no_reviewer_so_no_unchecked_notice() {
    let dir = tempfile::tempdir().unwrap();
    let out = run(
        &pair(dir.path(), &prose("handles"), &prose("serves"), &[]),
        &ReviewEnv::default(),
    )
    .unwrap();
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(notice_codes(&json), Vec::<&str>::new());
}

#[test]
fn prime_review_verdict_reports_both_models_and_forbids_guessing() {
    let text = review_topic(None, Approval::Ask, Level::Default);
    assert!(text.contains(
        "Reviewer: <model the harness used, or unknown>; author <model or unknown>; different: yes | no | not checked"
    ));
    assert!(text.contains("name the model the harness actually ran, or `unknown`; never guess."));
}
#[test]
fn the_author_flag_overrides_the_config_author_at_the_provider_check() {
    let dir = tempfile::tempdir().unwrap();
    // The provider runs `gpt-5`. The config author is also `gpt-5`, which would be refused,
    // but the flag names a different author, so the provider check passes and the call runs.
    let mock = Mock {
        model: "gpt-5",
        calls: Cell::new(0),
    };
    let env = ReviewEnv {
        provider: Some(&mock),
        author_model: Some("gpt-5".into()),
        ..ReviewEnv::default()
    };
    let out = run(
        &args(
            dir.path(),
            &[
                "--author-model",
                "claude-opus-5-5",
                "--reviewer-model",
                "mistral-large",
            ],
        ),
        &env,
    )
    .unwrap();
    assert!(out.contains("\"report\": \"no findings\""), "{out}");
    assert_eq!(mock.calls.get(), 1);
}

#[test]
fn a_mechanical_review_ignores_matching_author_and_provider_models() {
    let dir = tempfile::tempdir().unwrap();
    let mock = Mock {
        model: "gpt-5",
        calls: Cell::new(0),
    };
    for offline in [false, true] {
        let env = ReviewEnv {
            offline,
            provider: Some(&mock),
            author_model: Some("gpt-5".into()),
            ..ReviewEnv::default()
        };
        let out = run(
            &pair(dir.path(), &prose("handles"), &prose("serves"), &[]),
            &env,
        )
        .unwrap();
        let json: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(json["review_type"], "mechanical", "offline={offline}");
        assert_eq!(json["reviewer_model_checked"], false);
        assert_eq!(notice_codes(&json), Vec::<&str>::new());
    }
    assert_eq!(mock.calls.get(), 0);
}

/// The bytes of the `payload` member in a printed plan, from its key to the next member.
fn printed_payload(plan: &str) -> &str {
    let key = "\"payload\": ";
    let start = plan.find(key).expect("plan prints a payload") + key.len();
    let end = plan
        .find(",\n  \"uses_provider\"")
        .expect("payload is followed by uses_provider");
    &plan[start..end]
}

/// TM19, partial coverage only: `--dry-run` and a live run agree on the payload, and the
/// provider boundary receives the expected texts.
///
/// The provider is a recording double at the `ModelProvider` trait, which takes a
/// structured `Payload`, not bytes. The plugin host adapter (not yet available) is the
/// only place wire bytes exist, so this does not show that the host receives the bytes
/// that `--dry-run` prints. That check stays open until the adapter lands. Expected
/// texts below are literals worked out from the inputs, not produced by `payload()`.
#[rstest]
#[case::update_fixture(ORIGINAL, REVISED, &["--type", "update"], None)]
#[case::update_dropped_sentence(
    "Keep this one. Remove the 90 day limit.\n",
    "Keep this one.\n",
    &["--type", "update"],
    Some(&[("dropped", "Remove the 90 day limit.")][..]),
)]
#[case::rewrite_escapes(
    "Say \"hi\"\\ caf\u{e9} \u{1f600}\tend\n\nTwo.\r\n",
    "Say \"bye\"\\ caf\u{e9} \u{1f600}\tend\n\nTwo <b>&</b>.\r\n",
    &["--type", "rewrite"],
    Some(&[
        ("original_document", "Say \"hi\"\\ caf\u{e9} \u{1f600}\tend\n\nTwo.\r\n"),
        ("revised_document", "Say \"bye\"\\ caf\u{e9} \u{1f600}\tend\n\nTwo <b>&</b>.\r\n"),
    ][..]),
)]
fn tm19_dry_run_and_provider_call_share_one_payload(
    #[case] old: &str,
    #[case] new: &str,
    #[case] flags: &[&str],
    #[case] expected: Option<&[(&str, &str)]>,
) {
    let dir = tempfile::tempdir().unwrap();
    let rec = Recorder {
        seen: std::cell::RefCell::new(Vec::new()),
    };
    let env = ReviewEnv {
        provider: Some(&rec),
        ..ReviewEnv::default()
    };

    let mut dry_flags = flags.to_vec();
    dry_flags.push("--dry-run");
    let dry = run(&pair(dir.path(), old, new, &dry_flags), &env).unwrap();
    assert_eq!(
        rec.seen.borrow().len(),
        0,
        "dry run must not call the provider"
    );

    let live = run(&pair(dir.path(), old, new, flags), &env).unwrap();
    let seen = rec.seen.borrow();
    assert_eq!(seen.len(), 1);
    assert!(!seen[0].items.is_empty(), "case must send something");
    assert_eq!(
        printed_payload(&dry),
        printed_payload(&live),
        "dry run and live run print different payloads"
    );
    if let Some(expected) = expected {
        let got: Vec<(&str, &str)> = seen[0]
            .items
            .iter()
            .map(|i| (i.role, i.text.as_str()))
            .collect();
        assert_eq!(got, expected);
    }
}
