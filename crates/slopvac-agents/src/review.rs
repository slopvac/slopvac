//! Review workflow: type selection, scope budget, reviewer model rule, payloads.
//!
//! Policy: `docs/contract/policies.md` section 1.5. A model provider is a plugin.
//! It stays off until trusted and named under `[review] provider`, never runs
//! during lint or offline, and receives only the text the review type needs.

use std::path::PathBuf;

use clap::{Arg, ArgAction, ArgMatches, Command};
use serde::Serialize;

use crate::claims::{self, Report};
use crate::error::AgentsError;

// STUB(slopvac-8l4.11.8): `ReviewEnv` is filled by the caller; config and the provider hook replace it.

/// Genres whose claims carry contract weight. A review of one never runs as `mechanical`.
pub const STRICT_GENRES: [&str; 4] = ["reference", "specification", "api", "decision-record"];

/// Largest change, in sentences, that `mechanical` covers.
const MECHANICAL_MAX_CHURN: usize = 20;
/// Share of the original that a `mechanical` change may touch.
const MECHANICAL_MAX_SHARE: f64 = 0.25;
/// Share of the original at which a change becomes a `rewrite`.
const REWRITE_SHARE: f64 = 0.5;
/// Extra checks every review may add before the user must approve.
const BASE_EXTRA_BUDGET: usize = 5;

/// How deep a review goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewType {
    /// Wording-only change: lint and the claims diff, no reviewer.
    Mechanical,
    /// A bounded change with changed facts: check the changed claims.
    Update,
    /// Most of the document changed: check the whole document.
    Rewrite,
}

impl ReviewType {
    /// The name used on the command line and in prime.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Mechanical => "mechanical",
            Self::Update => "update",
            Self::Rewrite => "rewrite",
        }
    }

    /// Parse a type name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        [Self::Mechanical, Self::Update, Self::Rewrite]
            .into_iter()
            .find(|t| t.name() == name)
    }
}

/// Whether going past the scope budget needs the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Approval {
    /// Ask the user to confirm the scope. The default.
    #[default]
    Ask,
    /// Proceed without asking (`review_approval = "auto"`).
    Auto,
}

impl Approval {
    /// Parse the config value.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ask" => Some(Self::Ask),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }
}

/// What the claims diff says about the size and nature of a change.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ChangeStats {
    /// Sentences in the original.
    pub original_sentences: usize,
    /// Changed, added and dropped sentences.
    pub churn: usize,
    /// Changed or added claims that change a literal or negation.
    pub literal_claims: usize,
    /// Dropped sentences that look lost.
    pub fact_candidates: usize,
}

impl ChangeStats {
    /// Read the stats off a claims report.
    #[must_use]
    pub fn from_report(report: &Report) -> Self {
        let literal_claims = report
            .claims
            .iter()
            .filter(|c| {
                !c.literals_added.is_empty() || !c.literals_removed.is_empty() || c.negation_changed
            })
            .count();
        Self {
            original_sentences: report.summary.original_sentences,
            churn: report.summary.changed + report.summary.added + report.summary.dropped,
            literal_claims,
            fact_candidates: report.dropped.iter().filter(|d| d.fact_candidate).count(),
        }
    }

    #[allow(clippy::cast_precision_loss, reason = "sentence counts are tiny")]
    fn share(&self) -> f64 {
        if self.original_sentences == 0 {
            1.0
        } else {
            self.churn as f64 / self.original_sentences as f64
        }
    }
}

/// Choose the review type, with the reason in one sentence.
///
/// - `rewrite`: no original, or the change touches half of it or more.
/// - `mechanical`: no changed literal or negation, no lost fact, at most 20
///   sentences and a quarter of the original, and a genre without contract weight.
/// - `update`: everything else.
#[must_use]
pub fn select_type(stats: &ChangeStats, genre: Option<&str>) -> (ReviewType, String) {
    if stats.share() >= REWRITE_SHARE {
        return (
            ReviewType::Rewrite,
            "the change touches half of the document or more".into(),
        );
    }
    if genre.is_some_and(|g| STRICT_GENRES.contains(&g)) {
        return (
            ReviewType::Update,
            "the genre carries contract claims, so a wording change still needs a check".into(),
        );
    }
    if stats.literal_claims > 0 {
        return (
            ReviewType::Update,
            "a changed sentence changes a literal or negation".into(),
        );
    }
    if stats.fact_candidates > 0 {
        return (
            ReviewType::Update,
            "a sentence with facts is missing".into(),
        );
    }
    if stats.churn > MECHANICAL_MAX_CHURN || stats.share() > MECHANICAL_MAX_SHARE {
        return (ReviewType::Update, "the change is large".into());
    }
    (
        ReviewType::Mechanical,
        "the change rewords sentences and changes no literal, negation or fact".into(),
    )
}

/// The checks a review must do and the extra checks it may add unasked.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Scope {
    /// Changed and added claims plus lost-fact candidates. Always in scope.
    pub required: usize,
    /// Extra checks allowed without the user: 5 plus half of `required`, rounded up.
    pub extra_budget: usize,
}

impl Scope {
    /// Scope for a claims report.
    #[must_use]
    pub fn for_report(report: &Report) -> Self {
        let required = report.summary.changed
            + report.summary.added
            + report.dropped.iter().filter(|d| d.fact_candidate).count();
        Self {
            required,
            extra_budget: BASE_EXTRA_BUDGET + required.div_ceil(2),
        }
    }

    /// Decide whether `extra` more checks (unchanged claims, more sources) can run.
    pub fn admit(
        &self,
        extra: usize,
        approval: Approval,
        user_confirmed: bool,
    ) -> Result<(), AgentsError> {
        if extra <= self.extra_budget || approval == Approval::Auto || user_confirmed {
            return Ok(());
        }
        Err(AgentsError::Coded {
            code: "review-scope-approval",
            message: format!(
                "the review would add {extra} checks beyond the {} changed claims; the budget is {}. \
                 Ask the user to confirm this scope, then rerun with --scope-approved, \
                 or set review_approval = \"auto\".",
                self.required, self.extra_budget
            ),
        })
    }
}

fn model_key(model: &str) -> String {
    let no_variant = model.split('#').next().unwrap_or(model);
    let name = no_variant.rsplit('/').next().unwrap_or(no_variant);
    name.trim().to_lowercase()
}

/// Refuse a reviewer that is the author's model. Provider prefixes and `#variant`
/// suffixes do not make a model different. Without an author model the rule cannot
/// be checked, and this returns `Ok(false)`.
pub fn check_reviewer_model(author: Option<&str>, reviewer: &str) -> Result<bool, AgentsError> {
    let Some(author) = author else {
        return Ok(false);
    };
    if model_key(author) == model_key(reviewer) {
        return Err(AgentsError::Coded {
            code: "review-same-model",
            message: format!(
                "reviewer model {reviewer:?} is the author's model; pick a different one"
            ),
        });
    }
    Ok(true)
}

/// One piece of text sent to a provider.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PayloadItem {
    /// 1-based line in the revision.
    pub line: usize,
    /// Revised text.
    pub text: String,
    /// The text before the change, when it is part of the payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original: Option<String>,
}

/// The exact text a provider would receive.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Payload {
    /// `claims` (mechanical), `passages` (update) or `document` (rewrite).
    pub kind: &'static str,
    /// The items.
    pub items: Vec<PayloadItem>,
}

/// Build the payload for a review type. `mechanical` sends changed claims only,
/// `update` sends each changed passage with its original, `rewrite` sends the document.
#[must_use]
pub fn payload(kind: ReviewType, report: &Report, revised: &str) -> Payload {
    match kind {
        ReviewType::Mechanical => Payload {
            kind: "claims",
            items: report
                .claims
                .iter()
                .map(|c| PayloadItem {
                    line: c.line,
                    text: c.text.clone(),
                    original: None,
                })
                .collect(),
        },
        ReviewType::Update => Payload {
            kind: "passages",
            items: report
                .claims
                .iter()
                .map(|c| PayloadItem {
                    line: c.line,
                    text: c.text.clone(),
                    original: c.original.clone(),
                })
                .collect(),
        },
        ReviewType::Rewrite => Payload {
            kind: "document",
            items: vec![PayloadItem {
                line: 1,
                text: revised.to_owned(),
                original: None,
            }],
        },
    }
}

/// A model provider, supplied by the plugin host once it exists.
pub trait ModelProvider {
    /// The provider's name as in `[review] provider`.
    fn name(&self) -> &str;
    /// The model the provider reports it will use.
    fn model(&self) -> &str;
    /// Review the payload and return the reviewer's report.
    fn review(&self, payload: &Payload) -> Result<String, AgentsError>;
}

/// Everything about the environment that review depends on.
#[derive(Default)]
pub struct ReviewEnv<'a> {
    /// Offline mode blocks every provider call.
    pub offline: bool,
    /// The trusted provider named under `[review] provider`, if any.
    pub provider: Option<&'a dyn ModelProvider>,
    /// `review_approval`.
    pub approval: Approval,
    /// The author's model, if known.
    pub author_model: Option<String>,
}

/// The plan a review run prints.
#[derive(Debug, Serialize)]
pub struct Plan {
    /// Chosen type.
    pub review_type: ReviewType,
    /// Why.
    pub reason: String,
    /// Change stats.
    pub stats: ChangeStats,
    /// Scope and budget.
    pub scope: Scope,
    /// The exact provider payload.
    pub payload: Payload,
    /// True when the reviewer model was compared with the author's.
    pub reviewer_model_checked: bool,
    /// The provider's report; absent on `--dry-run`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
}

/// The `review` subcommand, for the CLI epic to mount.
#[must_use]
pub fn command() -> Command {
    let path = || clap::value_parser!(PathBuf);
    Command::new("review")
        .about("Plan or run the review of a revision")
        .arg(
            Arg::new("revised")
                .value_name("REVISED")
                .required(true)
                .value_parser(path())
                .help("The revised document"),
        )
        .arg(
            Arg::new("original")
                .long("original")
                .value_name("FILE")
                .required(true)
                .value_parser(path())
                .help("The document before the revision"),
        )
        .arg(
            Arg::new("type")
                .long("type")
                .value_parser(["auto", "mechanical", "update", "rewrite"])
                .default_value("auto")
                .help("Review type; auto picks from the change"),
        )
        .arg(
            Arg::new("genre")
                .long("genre")
                .value_name("GENRE")
                .help("The document's genre"),
        )
        .arg(
            Arg::new("extra")
                .long("extra")
                .value_name("N")
                .value_parser(clap::value_parser!(usize))
                .default_value("0")
                .help("Checks to add beyond the changed claims"),
        )
        .arg(
            Arg::new("scope-approved")
                .long("scope-approved")
                .action(ArgAction::SetTrue)
                .help("The user confirmed the scope"),
        )
        .arg(
            Arg::new("reviewer-model")
                .long("reviewer-model")
                .value_name("MODEL")
                .help("Model that reviews; it must differ from the author's"),
        )
        .arg(
            Arg::new("dry-run")
                .long("dry-run")
                .action(ArgAction::SetTrue)
                .help("Print the exact payload and call no provider"),
        )
}

/// Run the parsed `review` command.
///
/// With `--dry-run` it prints the plan and the payload and calls nothing. Without
/// it, a trusted provider must be present and the run must be online.
pub fn run(matches: &ArgMatches, env: &ReviewEnv<'_>) -> Result<String, AgentsError> {
    let read = |id: &str| -> Result<String, AgentsError> {
        let path = matches.get_one::<PathBuf>(id).cloned().unwrap_or_default();
        std::fs::read_to_string(&path).map_err(|e| AgentsError::io(path, e))
    };
    let (original, revised) = (read("original")?, read("revised")?);
    let report = claims::compare(&original, &revised);
    let stats = ChangeStats::from_report(&report);
    let (auto_type, auto_reason) = select_type(
        &stats,
        matches.get_one::<String>("genre").map(String::as_str),
    );
    let (review_type, reason) = match matches
        .get_one::<String>("type")
        .and_then(|t| ReviewType::parse(t))
    {
        Some(t) => (t, "chosen with --type".to_owned()),
        None => (auto_type, auto_reason),
    };
    let scope = Scope::for_report(&report);
    scope.admit(
        matches.get_one::<usize>("extra").copied().unwrap_or(0),
        env.approval,
        matches.get_flag("scope-approved"),
    )?;
    let reviewer = matches.get_one::<String>("reviewer-model").cloned();
    let checked = match &reviewer {
        Some(r) => check_reviewer_model(env.author_model.as_deref(), r)?,
        None => false,
    };
    let mut plan = Plan {
        review_type,
        reason,
        stats,
        scope,
        payload: payload(review_type, &report, &revised),
        reviewer_model_checked: checked,
        report: None,
    };
    if !matches.get_flag("dry-run") {
        plan.report = Some(call_provider(env, &plan)?);
    }
    let mut json = serde_json::to_string_pretty(&plan)
        .map_err(|e| AgentsError::Usage(format!("review json: {e}")))?;
    json.push('\n');
    Ok(json)
}

fn call_provider(env: &ReviewEnv<'_>, plan: &Plan) -> Result<String, AgentsError> {
    if env.offline {
        return Err(AgentsError::Coded {
            code: "offline-network-needed",
            message: "offline: review needs the network".into(),
        });
    }
    let provider = env.provider.ok_or_else(|| AgentsError::Coded {
        code: "review-provider-missing",
        message: "no trusted model provider: run `slopvac plugins trust` for one and name it under [review] provider, or use --dry-run".into(),
    })?;
    check_reviewer_model(env.author_model.as_deref(), provider.model())?;
    provider.review(&plan.payload)
}
