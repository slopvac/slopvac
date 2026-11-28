//! Behaviour tests for the review workflow: type, scope, models, payloads, consent.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

use std::cell::Cell;
use std::fs;
use std::path::Path;

use rstest::rstest;
use slopvac_agents::claims::compare;
use slopvac_agents::error::AgentsError;
use slopvac_agents::prime::{self, Level, Settings};
use slopvac_agents::review::{
    ChangeStats, ModelCheck, ModelProvider, ProviderReply, ProviderRequest, ReplyItem, ReviewEnv,
    ReviewType, Scope, SentItem, command, compare_models, payload, run, select_type,
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
    assert!(scope.admit(8, false).is_ok());
    let err = scope.admit(9, false).unwrap_err();
    assert_eq!(err.code(), "review-scope-approval");
    assert!(err.to_string().contains("--scope-approved"));
    assert!(scope.admit(9, true).is_ok());
    let bigger = Scope::for_report(&compare("A one. B two.", "").unwrap());
    assert!(bigger.extra_budget >= 5);
}

#[rstest]
#[case::variant_and_prefix(
    Some("anthropic/claude-sonnet-5-5#high"),
    "claude-sonnet-5-5",
    ModelCheck::Same
)]
#[case::case_and_provider(
    Some("Claude-Sonnet-5-5"),
    "amazon-bedrock/claude-sonnet-5-5",
    ModelCheck::Same
)]
#[case::other_model(Some("gpt-5"), "claude-sonnet-5-5", ModelCheck::Different)]
#[case::unknown_author(None, "claude-sonnet-5-5", ModelCheck::Unchecked)]
fn reviewer_model_is_compared_with_the_author_model_without_a_refusal(
    #[case] author: Option<&str>,
    #[case] reviewer: &str,
    #[case] expected: ModelCheck,
) {
    assert_eq!(compare_models(author, reviewer), expected);
}

#[test]
fn payload_is_only_what_the_review_type_needs() {
    let report = compare(ORIGINAL, REVISED).unwrap();
    let mechanical = payload(ReviewType::Mechanical, &report, ORIGINAL, REVISED);
    assert_eq!(mechanical.kind, "none");
    assert_eq!(mechanical.items.len(), 0);

    let update = payload(ReviewType::Update, &report, ORIGINAL, REVISED);
    assert_eq!(update.kind, "passages");
    let roles = |role: &str| update.items.iter().filter(|i| i.role.as_str() == role).count();
    assert_eq!(
        (roles("changed"), roles("added"), roles("dropped")),
        (2, 1, 3)
    );
    assert!(
        update
            .items
            .iter()
            .filter(|i| i.role.as_str() == "changed")
            .all(|i| i.original.is_some())
    );
    let dropped: Vec<&str> = update
        .items
        .iter()
        .filter(|i| i.role.as_str() == "dropped")
        .map(|i| i.text.as_str())
        .collect();
    assert!(dropped.contains(&"Entries expire after 24 hours."));

    let rewrite = payload(ReviewType::Rewrite, &report, ORIGINAL, REVISED);
    assert_eq!(rewrite.kind, "document");
    let texts: Vec<(&str, &str)> = rewrite
        .items
        .iter()
        .map(|i| (i.role.as_str(), i.text.as_str()))
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
        (update.items[0].role.as_str(), update.items[0].text.as_str()),
        ("dropped", "Remove the 90 day limit.")
    );
}

/// Answer every sent item with `text`, on the requested model unless `reported` names
/// another model for that item.
fn answer_all(
    request: &ProviderRequest<'_>,
    text: &str,
    reported: &[(usize, &str)],
) -> ProviderReply {
    ProviderReply::new(
        (0..request.items.len())
            .map(|i| {
                let model = reported
                    .iter()
                    .find(|(at, _)| *at == i)
                    .map_or(request.model, |(_, m)| m);
                ReplyItem::text(i, text, model)
            })
            .collect(),
    )
}

struct Mock {
    model: &'static str,
    calls: Cell<usize>,
}

impl ModelProvider for Mock {
    fn name(&self) -> &'static str {
        "mock"
    }
    fn model(&self) -> Option<&str> {
        Some(self.model)
    }
    fn review(&self, request: &ProviderRequest<'_>) -> Result<ProviderReply, AgentsError> {
        self.calls.set(self.calls.get() + 1);
        Ok(answer_all(request, "no findings", &[]))
    }
}

/// A provider that declares one model and reports the given model for each listed item.
struct Reporting {
    declared: &'static str,
    reported: Vec<(usize, &'static str)>,
}

impl ModelProvider for Reporting {
    fn name(&self) -> &'static str {
        "reporting"
    }
    fn model(&self) -> Option<&str> {
        Some(self.declared)
    }
    fn review(&self, request: &ProviderRequest<'_>) -> Result<ProviderReply, AgentsError> {
        Ok(answer_all(request, "reported", &self.reported))
    }
}

/// A provider whose reply is exactly `items`, whatever was sent.
struct Fixed {
    items: Vec<ReplyItem>,
}

impl ModelProvider for Fixed {
    fn name(&self) -> &'static str {
        "fixed"
    }
    fn model(&self) -> Option<&str> {
        None
    }
    fn review(&self, _: &ProviderRequest<'_>) -> Result<ProviderReply, AgentsError> {
        Ok(ProviderReply::new(self.items.clone()))
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

    // A model that only the provider declares is not enough: the request must name one.
    let unnamed = ReviewEnv {
        provider: Some(&mock),
        ..ReviewEnv::default()
    };
    let refused = run(&args(dir.path(), &[]), &unnamed)
        .err()
        .map(|e| e.code());
    assert_eq!(refused, Some("review-model-missing"));
    assert_eq!(mock.calls.get(), 0);

    let online = ReviewEnv {
        provider: Some(&mock),
        author_model: Some("claude-sonnet-5-5".into()),
        reviewer_model: Some("gpt-5".into()),
        ..ReviewEnv::default()
    };
    let out = run(&args(dir.path(), &[]), &online).unwrap();
    assert!(out.contains("no findings"), "{out}");
    assert_eq!(mock.calls.get(), 1);
}

#[test]
fn a_provider_on_the_authors_model_still_runs_and_the_plan_carries_a_notice() {
    let dir = tempfile::tempdir().unwrap();
    let mock = Mock {
        model: "anthropic/claude-sonnet-5-5",
        calls: Cell::new(0),
    };
    let env = ReviewEnv {
        provider: Some(&mock),
        author_model: Some("claude-sonnet-5-5#high".into()),
        reviewer_model: Some("gpt-5".into()),
        ..ReviewEnv::default()
    };
    let json = plan_json(dir.path(), &[], &env);
    assert!(notice_codes(&json).contains(&"review-model-same"), "{json}");
    assert!(json["report"].is_string(), "{json}");
    assert_eq!(mock.calls.get(), 1);
}

#[test]
fn extra_checks_above_the_budget_need_the_scope_flag() {
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

fn review_topic(review_type: Option<ReviewType>, level: Level) -> String {
    prime::render(
        Some("review"),
        &Settings {
            level,
            review_type,
            ..Settings::default()
        },
        &tree(),
        Path::new("/nonexistent"),
    )
    .unwrap()
}

#[test]
fn prime_review_renders_the_steps_of_each_type() {
    let auto = review_topic(None, Level::Default);
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
        let text = review_topic(Some(kind), Level::Default);
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
    let ask = review_topic(None, Level::Default);
    assert!(ask.contains("ask the user to confirm the exact scope"));
    assert!(ask.contains("--scope-approved"));
    assert!(ask.contains("Use a reviewer model that differs from the author's"));
}

/// The flag is the only way to approve a larger scope: no config key can approve it for
/// everyone who runs a review in a checkout.
#[test]
fn prime_review_offers_only_the_flag_to_approve_a_larger_scope() {
    let text = prime::render(
        Some("review"),
        &Settings::default(),
        &tree(),
        Path::new("/nonexistent"),
    )
    .unwrap();
    assert!(
        text.contains("Then rerun with `--scope-approved`."),
        "{text}"
    );
    assert!(!text.contains("review_approval"), "{text}");
    assert!(!text.contains("in the config"), "{text}");
}

#[test]
fn prime_review_names_only_real_commands_and_flags() {
    let problems: Vec<String> = prime::check_topics(&tree())
        .into_iter()
        .filter(|p| p.starts_with("review:"))
        .collect();
    assert_eq!(problems, Vec::<String>::new());
}
/// A provider that keeps the task and the items of every request.
struct Recorder {
    seen: std::cell::RefCell<Vec<(&'static str, Vec<SentItem>)>>,
}

impl ModelProvider for Recorder {
    fn name(&self) -> &'static str {
        "recorder"
    }
    fn model(&self) -> Option<&str> {
        Some("gpt-5")
    }
    fn review(&self, request: &ProviderRequest<'_>) -> Result<ProviderReply, AgentsError> {
        self.seen
            .borrow_mut()
            .push((request.task, request.items.to_vec()));
        Ok(answer_all(request, "ok", &[]))
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
        reviewer_model: Some("gpt-5".into()),
        ..ReviewEnv::default()
    };

    let old = format!("{} Remove the 90 day limit.", prose("handles"));
    run(
        &pair(dir.path(), &old, &prose("handles"), &["--type", "update"]),
        &env,
    )
    .unwrap();
    let (task, sent) = rec.seen.borrow()[0].clone();
    assert_eq!(task, "update");
    let texts: Vec<(&str, &str)> = sent
        .iter()
        .map(|i| (i.id.as_str(), i.text.as_str()))
        .collect();
    assert_eq!(texts, [("1", "Remove the 90 day limit.")]);

    run(
        &pair(dir.path(), ORIGINAL, REVISED, &["--type", "rewrite"]),
        &env,
    )
    .unwrap();
    let (task, sent) = rec.seen.borrow()[1].clone();
    assert_eq!(task, "rewrite");
    let texts: Vec<&str> = sent.iter().map(|i| i.text.as_str()).collect();
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
fn the_author_flag_names_the_author_for_the_same_model_notice() {
    let dir = tempfile::tempdir().unwrap();
    let json = plan_json(
        dir.path(),
        &[
            "--dry-run",
            "--author-model",
            "anthropic/gpt-5#high",
            "--reviewer-model",
            "gpt-5",
        ],
        &ReviewEnv::default(),
    );
    assert!(notice_codes(&json).contains(&"review-model-same"), "{json}");
    assert_eq!(json["reviewer_model_checked"], true);
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
    let text = review_topic(None, Level::Default);
    assert!(text.contains(
        "Reviewer: <model the harness used, or unknown>; author <model or unknown>; different: yes | no | not checked"
    ));
    assert!(text.contains("name the model the harness ran, or `unknown`. Do not guess."));
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
    assert!(out.contains("no findings"), "{out}");
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

fn rewrite_plan(
    reported: Vec<(usize, &'static str)>,
    declared: &'static str,
    extra: &[&str],
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let provider = Reporting { declared, reported };
    let env = ReviewEnv {
        provider: Some(&provider),
        author_model: Some("claude-sonnet-5-5".into()),
        reviewer_model: Some("gpt-5".into()),
        ..ReviewEnv::default()
    };
    let mut argv = vec!["--type", "rewrite"];
    argv.extend_from_slice(extra);
    let out = run(&pair(dir.path(), ORIGINAL, REVISED, &argv), &env)?;
    Ok(serde_json::from_str(&out)?)
}

fn notice_with<'a>(json: &'a serde_json::Value, code: &str) -> Vec<&'a serde_json::Value> {
    json["notices"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|n| n["code"] == code)
        .collect()
}

#[test]
fn a_reported_author_model_gets_a_notice_that_locates_its_items()
-> Result<(), Box<dyn std::error::Error>> {
    let json = rewrite_plan(vec![(1, "anthropic/claude-sonnet-5-5#high")], "gpt-5", &[])?;
    let found = notice_with(&json, "review-model-same");
    assert_eq!(found.len(), 1, "{json}");
    assert_eq!(found[0]["level"], "warning");
    assert_eq!(found[0]["data"]["reviewer_source"], "reported");
    assert_eq!(found[0]["data"]["author_source"], "config");
    assert_eq!(
        found[0]["data"]["items"],
        serde_json::json!([{"role": "revised_document", "line": 1}])
    );
    assert!(found[0].get("path").is_none());
    Ok(())
}

#[test]
fn a_reported_model_other_than_the_requested_one_gets_an_info_notice()
-> Result<(), Box<dyn std::error::Error>> {
    let json = rewrite_plan(
        vec![(0, "gemini-3"), (1, "gemini-3")],
        "gpt-5",
        &["--reviewer-model", "gpt-5"],
    )?;
    let found = notice_with(&json, "review-model-reported-mismatch");
    assert_eq!(found.len(), 1, "{json}");
    assert_eq!(found[0]["level"], "info");
    let data = &found[0]["data"];
    assert_eq!(data["requested"], "gpt-5");
    assert_eq!(data["requested_source"], "flag");
    assert_eq!(data["reported"], "gemini-3");
    assert_eq!(
        data["items"],
        serde_json::json!([
            {"role": "original_document", "line": 1},
            {"role": "revised_document", "line": 1}
        ])
    );
    Ok(())
}

#[test]
fn a_reported_model_that_matches_the_request_adds_no_notice()
-> Result<(), Box<dyn std::error::Error>> {
    let json = rewrite_plan(vec![(0, "openai/gpt-5#high"), (1, "GPT-5")], "gpt-5", &[])?;
    assert!(json["notices"].as_array().unwrap().is_empty(), "{json}");
    Ok(())
}

#[test]
fn reported_models_group_by_normalized_name_across_spellings()
-> Result<(), Box<dyn std::error::Error>> {
    let json = rewrite_plan(
        vec![(0, "claude-sonnet-5-5"), (1, "bedrock/Claude-Sonnet-5-5#x")],
        "gpt-5",
        &[],
    )?;
    let same = notice_with(&json, "review-model-same");
    assert_eq!(same.len(), 1, "{json}");
    assert_eq!(same[0]["data"]["items"].as_array().unwrap().len(), 2);
    Ok(())
}

#[rstest]
#[case::outside_the_request(vec![ReplyItem::text(0, "t", "m"), ReplyItem::text(1, "t", "m"), ReplyItem::text(2, "t", "m")])]
#[case::repeated_item(vec![ReplyItem::text(0, "t", "m"), ReplyItem::text(0, "t", "m"), ReplyItem::text(1, "t", "m")])]
#[case::missing_item(vec![ReplyItem::text(0, "t", "m")])]
#[case::empty_model(vec![ReplyItem::text(0, "t", ""), ReplyItem::text(1, "t", "m")])]
#[case::blank_model(vec![ReplyItem::text(0, "t", "  "), ReplyItem::text(1, "t", "m")])]
fn a_reply_the_protocol_would_refuse_fails_with_a_code(#[case] items: Vec<ReplyItem>) {
    let dir = tempfile::tempdir().unwrap();
    let provider = Fixed { items };
    let setting = ReviewEnv {
        provider: Some(&provider),
        reviewer_model: Some("gpt-5".into()),
        ..ReviewEnv::default()
    };
    let argv = pair(dir.path(), ORIGINAL, REVISED, &["--type", "rewrite"]);
    let refused = run(&argv, &setting).err().map(|e| e.code());
    assert_eq!(refused, Some("review-provider-reply-invalid"));
}

#[test]
fn a_dry_run_between_different_models_reports_no_model_notice() {
    let dir = tempfile::tempdir().unwrap();
    let json = plan_json(
        dir.path(),
        &["--dry-run", "--reviewer-model", "gpt-5"],
        &ReviewEnv {
            author_model: Some("claude-sonnet-5-5".into()),
            ..ReviewEnv::default()
        },
    );
    assert!(notice_codes(&json).is_empty(), "{json}");
}

#[test]
fn a_declared_provider_on_the_author_model_adds_one_notice_with_its_source()
-> Result<(), Box<dyn std::error::Error>> {
    let json = rewrite_plan(vec![], "claude-sonnet-5-5", &[])?;
    let found = notice_with(&json, "review-model-same");
    assert_eq!(found.len(), 1, "{json}");
    assert_eq!(found[0]["data"]["reviewer_source"], "provider");
    assert!(found[0]["data"].get("items").is_none());
    Ok(())
}

#[test]
fn every_data_key_a_model_notice_carries_is_documented() -> Result<(), Box<dyn std::error::Error>> {
    let plans = [
        rewrite_plan(vec![], "claude-sonnet-5-5", &[])?,
        rewrite_plan(vec![(1, "claude-sonnet-5-5")], "gpt-5", &[])?,
        rewrite_plan(
            vec![(0, "gemini-3")],
            "gpt-5",
            &["--reviewer-model", "gpt-5"],
        )?,
    ];
    let contract = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/contract/lint-report.md"
    ))?;
    let mut seen = std::collections::BTreeSet::new();
    for plan in &plans {
        for code in ["review-model-same", "review-model-reported-mismatch"] {
            for notice in notice_with(plan, code) {
                assert!(notice.get("path").is_none(), "{notice}");
                for key in notice["data"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(k, _)| k)
                {
                    seen.insert((code, key.clone()));
                }
            }
        }
    }
    assert!(seen.len() >= 8, "the scenarios emit too few keys: {seen:?}");
    for (code, key) in &seen {
        let row = contract
            .lines()
            .find(|l| l.starts_with(&format!("| `data.{key}` |")))
            .ok_or_else(|| format!("data.{key} is not documented"))?;
        assert!(row.contains(&format!("`{code}`")), "{code} {key}: {row}");
    }
    Ok(())
}

/// Policies section 1.5: the same model, detected before the call, adds `review-model-same`
/// (warning). Here the reviewer flag names another model, so only the provider's declared model
/// matches the author. A dry run reports the notice and calls nothing.
#[rstest]
#[case::real_run(false, 1)]
#[case::dry_run(true, 0)]
fn a_declared_provider_on_the_author_model_adds_one_notice_when_the_reviewer_flag_differs(
    #[case] dry_run: bool,
    #[case] calls: usize,
) {
    let dir = tempfile::tempdir().unwrap();
    let mock = Mock {
        model: "claude-sonnet-5-5",
        calls: Cell::new(0),
    };
    let setting = ReviewEnv {
        provider: Some(&mock),
        author_model: Some("claude-sonnet-5-5#high".into()),
        ..ReviewEnv::default()
    };
    let mut extra = vec!["--reviewer-model", "gpt-5"];
    if dry_run {
        extra.push("--dry-run");
    }
    let json = plan_json(dir.path(), &extra, &setting);
    let found = notice_with(&json, "review-model-same");
    assert_eq!(found.len(), 1, "{json}");
    assert_eq!(found[0]["level"], "warning");
    assert_eq!(found[0]["data"]["reviewer"], "claude-sonnet-5-5");
    assert_eq!(found[0]["data"]["reviewer_source"], "provider");
    assert_eq!(found[0]["data"]["author_source"], "config");
    assert!(found[0]["data"].get("items").is_none());
    assert_eq!(json.get("report").is_some(), !dry_run, "{json}");
    assert_eq!(mock.calls.get(), calls);
}

/// The `*_source` values of notice data, per the notice-data table in `lint-report.md`.
/// A case names where the author and the reviewer come from, and which notice carries which.
#[rstest]
#[case::author_and_reviewer_flags(
    &["--author-model", "gpt-5", "--reviewer-model", "gpt-5"], None, None, "author_source", "flag", "reviewer_source", "flag"
)]
#[case::author_and_reviewer_config(
    &[], Some("gpt-5"), Some("gpt-5"), "author_source", "config", "reviewer_source", "config"
)]
fn the_same_model_notice_names_where_each_model_came_from(
    #[case] flags: &[&str],
    #[case] author_config: Option<&str>,
    #[case] reviewer_config: Option<&str>,
    #[case] author_key: &str,
    #[case] author_source: &str,
    #[case] reviewer_key: &str,
    #[case] reviewer_source: &str,
) {
    let dir = tempfile::tempdir().unwrap();
    let setting = ReviewEnv {
        author_model: author_config.map(Into::into),
        reviewer_model: reviewer_config.map(Into::into),
        ..ReviewEnv::default()
    };
    let mut extra = vec!["--dry-run"];
    extra.extend_from_slice(flags);
    let json = plan_json(dir.path(), &extra, &setting);
    let found = notice_with(&json, "review-model-same");
    assert_eq!(found.len(), 1, "{json}");
    assert_eq!(found[0]["data"][author_key], author_source, "{json}");
    assert_eq!(found[0]["data"][reviewer_key], reviewer_source, "{json}");
}

#[rstest]
#[case::requested_from_config(&[][..], Some("gpt-5"), "config")]
#[case::requested_from_the_flag(&["--reviewer-model", "gpt-5"][..], None, "flag")]
fn the_mismatch_notice_names_where_the_requested_model_came_from(
    #[case] flags: &[&str],
    #[case] reviewer_config: Option<&str>,
    #[case] requested_source: &str,
) {
    let dir = tempfile::tempdir().unwrap();
    let provider = Reporting {
        declared: "claude-sonnet-5-5",
        reported: vec![(0, "gemini-3")],
    };
    let setting = ReviewEnv {
        provider: Some(&provider),
        reviewer_model: reviewer_config.map(Into::into),
        ..ReviewEnv::default()
    };
    let mut argv = vec!["--type", "rewrite"];
    argv.extend_from_slice(flags);
    let argv = pair(dir.path(), ORIGINAL, REVISED, &argv);
    let out = run(&argv, &setting).unwrap();
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    let found = notice_with(&json, "review-model-reported-mismatch");
    assert_eq!(found.len(), 1, "{json}");
    assert_eq!(
        found[0]["data"]["requested_source"], requested_source,
        "{json}"
    );
    assert_eq!(found[0]["data"]["reported"], "gemini-3", "{json}");
}

/// Policies section 1.5: the two phases give two separate non-gating `review-model-same`
/// warnings. A provider that declares the author's model and also reports it for items gets
/// one notice before the call, which names the provider and carries no items, and one after
/// it, which locates each reported item. The reported model is not the requested `gpt-5`, so
/// the info notice follows.
#[test]
fn a_provider_that_declares_and_reports_the_author_model_adds_one_notice_per_phase()
-> Result<(), Box<dyn std::error::Error>> {
    let json = rewrite_plan(
        vec![
            (0, "anthropic/claude-sonnet-5-5"),
            (1, "anthropic/claude-sonnet-5-5#high"),
        ],
        "claude-sonnet-5-5",
        &[],
    )?;
    assert_eq!(
        notice_codes(&json),
        [
            "review-model-same",
            "review-model-same",
            "review-model-reported-mismatch"
        ],
        "{json}"
    );
    let found = notice_with(&json, "review-model-same");
    assert_eq!(found[0]["level"], "warning", "{json}");
    assert_eq!(found[0]["data"]["reviewer_source"], "provider", "{json}");
    assert_eq!(found[0]["data"]["author_source"], "config", "{json}");
    assert!(found[0]["data"].get("items").is_none(), "{json}");
    assert_eq!(found[1]["level"], "warning", "{json}");
    assert_eq!(found[1]["data"]["reviewer_source"], "reported", "{json}");
    assert_eq!(found[1]["data"]["author_source"], "config", "{json}");
    assert_eq!(
        found[1]["data"]["items"],
        serde_json::json!([
            {"role": "original_document", "line": 1},
            {"role": "revised_document", "line": 1}
        ]),
        "{json}"
    );
    Ok(())
}

/// A stopped review exposes its failure as the error source.
#[test]
fn a_stopped_review_gives_its_failure_as_the_source() {
    let stopped = slopvac_agents::review::Stopped::new(
        AgentsError::new_coded("review-provider-missing", "no provider"),
        Vec::new(),
    );

    let source = std::error::Error::source(&stopped)
        .and_then(|s| s.downcast_ref::<AgentsError>())
        .map(AgentsError::code);

    assert_eq!(source, Some("review-provider-missing"));
}
