//! Review workflow: type selection, scope budget, reviewer model rule, payloads.
//!
//! Policy: `docs/contract/policies.md` section 1.5. A model provider is a plugin.
//! It stays off until trusted and named under `[review] provider`, never runs
//! during lint or offline, and receives only the text the review type needs.

use std::path::PathBuf;

use clap::{Arg, ArgAction, ArgMatches, Command};
use schemars::JsonSchema;
use serde::Serialize;

use slopvac_messages::Catalog;
use slopvac_model::{Notice, NoticeLevel, SchemaVersion};
use slopvac_update::messages::UpdateMessage;

use crate::claims::{self, Report};
use crate::error::{AgentsError, notice};
use crate::messages::{AgentsMessage, AgentsPhrase, AgentsText, quoted};

// STUB(slopvac-8l4.11.8): `ReviewEnv` is filled by the caller; config and the provider hook replace it.
// The `[review] author_model` and `[review] model` lookups sit behind the same seam.

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
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
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
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
            AgentsPhrase::ReasonRewrite.as_static().into(),
        );
    }
    if genre.is_some_and(|g| STRICT_GENRES.contains(&g)) {
        return (
            ReviewType::Update,
            AgentsPhrase::ReasonGenre.as_static().into(),
        );
    }
    if stats.literal_claims > 0 {
        return (
            ReviewType::Update,
            AgentsPhrase::ReasonLiteral.as_static().into(),
        );
    }
    if stats.fact_candidates > 0 {
        return (
            ReviewType::Update,
            AgentsPhrase::ReasonFact.as_static().into(),
        );
    }
    if stats.churn > MECHANICAL_MAX_CHURN || stats.share() > MECHANICAL_MAX_SHARE {
        return (
            ReviewType::Update,
            AgentsPhrase::ReasonLarge.as_static().into(),
        );
    }
    (
        ReviewType::Mechanical,
        AgentsPhrase::ReasonMechanical.as_static().into(),
    )
}

/// The checks a review must do and the extra checks it may add unasked.
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
// A rule's `Scope` already owns that `$defs` name in `rule.schema.json`.
#[schemars(rename = "ReviewScope")]
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
        Err(AgentsError::coded(&AgentsMessage::ScopeApproval {
            extra,
            required: self.required,
            budget: self.extra_budget,
        }))
    }
}

fn model_key(model: &str) -> String {
    let no_variant = model.split('#').next().unwrap_or(model);
    let name = no_variant.rsplit('/').next().unwrap_or(no_variant);
    name.trim().to_lowercase()
}

/// How the reviewer's model compares with the author's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCheck {
    /// The author's model is unknown, so the rule cannot be checked.
    Unchecked,
    /// The two models differ.
    Different,
    /// The reviewer is the author's model.
    Same,
}

/// Compare the reviewer's model with the author's. Provider prefixes and `#variant`
/// suffixes do not make a model different. A match never stops a review. The plan carries
/// a notice for it, because slopvac cannot see which model a provider really runs.
#[must_use]
pub fn compare_models(author: Option<&str>, reviewer: &str) -> ModelCheck {
    match author {
        None => ModelCheck::Unchecked,
        Some(a) if model_key(a) == model_key(reviewer) => ModelCheck::Same,
        Some(_) => ModelCheck::Different,
    }
}

/// One piece of text sent to a provider.
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
#[schemars(transform = crate::schema::payload_item)]
pub struct PayloadItem {
    /// What the text is: `changed`, `added`, `dropped`, `original_document` or
    /// `revised_document`.
    #[schemars(extend("type" = "string", "enum" = ["changed", "added", "dropped", "original_document", "revised_document"]))]
    pub role: &'static str,
    /// Line of `text` in its document: the revision, except for `dropped` and
    /// `original_document`, which sit in the original.
    #[schemars(range(min = 1))]
    pub line: usize,
    /// The text.
    pub text: String,
    /// For `changed`, the sentence before the change.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(transform = slopvac_model::absent_not_null)]
    pub original: Option<String>,
}

/// The exact text a provider would receive.
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
#[schemars(transform = crate::schema::payload)]
pub struct Payload {
    /// `none` (mechanical), `passages` (update) or `document` (rewrite).
    #[schemars(extend("type" = "string", "enum" = ["none", "passages", "document"]))]
    pub kind: &'static str,
    /// The items.
    pub items: Vec<PayloadItem>,
}

/// Build the payload for a review type.
///
/// - `mechanical` sends nothing: lint and the claims diff are the whole review.
/// - `update` sends every changed passage with its original, every added sentence,
///   and every dropped original sentence, so a deletion is reviewed too.
/// - `rewrite` sends the original document as the baseline and the revised document.
#[must_use]
pub fn payload(kind: ReviewType, report: &Report, original: &str, revised: &str) -> Payload {
    match kind {
        ReviewType::Mechanical => Payload {
            kind: "none",
            items: Vec::new(),
        },
        ReviewType::Update => {
            let mut items: Vec<PayloadItem> = report
                .claims
                .iter()
                .map(|c| PayloadItem {
                    role: if c.change == "added" {
                        "added"
                    } else {
                        "changed"
                    },
                    line: c.line,
                    text: c.text.clone(),
                    original: c.original.clone(),
                })
                .collect();
            items.extend(report.dropped.iter().map(|d| PayloadItem {
                role: "dropped",
                line: d.line,
                text: d.text.clone(),
                original: None,
            }));
            Payload {
                kind: "passages",
                items,
            }
        }
        ReviewType::Rewrite => Payload {
            kind: "document",
            items: vec![
                PayloadItem {
                    role: "original_document",
                    line: 1,
                    text: original.to_owned(),
                    original: None,
                },
                PayloadItem {
                    role: "revised_document",
                    line: 1,
                    text: revised.to_owned(),
                    original: None,
                },
            ],
        },
    }
}

/// Refuse an explicit type that is lighter than the change needs. `auto` is the
/// least type that covers the change, so a lighter type under-reviews it.
pub fn check_eligible(
    chosen: ReviewType,
    required: ReviewType,
    reason: &str,
) -> Result<(), AgentsError> {
    if rank(chosen) >= rank(required) {
        return Ok(());
    }
    Err(AgentsError::coded(&AgentsMessage::TypeIneligible {
        chosen: chosen.name().to_owned(),
        needed: required.name().to_owned(),
        reason: reason.to_owned(),
    }))
}

fn rank(kind: ReviewType) -> u8 {
    match kind {
        ReviewType::Mechanical => 0,
        ReviewType::Update => 1,
        ReviewType::Rewrite => 2,
    }
}

/// A model provider, supplied by the plugin host once it exists.
pub trait ModelProvider {
    /// The provider's name as in `[review] provider`.
    fn name(&self) -> &str;
    /// The model the provider reports it will use.
    fn model(&self) -> &str;
    /// Review the payload and return the reviewer's report with the model each item ran on.
    ///
    /// # Errors
    /// The provider could not review the payload.
    fn review(&self, payload: &Payload) -> Result<ProviderReply, AgentsError>;
}

/// The model a provider reports it used for one payload item.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReportedItem {
    /// Index into [`Payload::items`].
    pub index: usize,
    /// The model the provider reports for that item.
    pub model: String,
}

impl ReportedItem {
    /// Report `model` for the payload item at `index`.
    #[must_use]
    pub fn new(index: usize, model: impl Into<String>) -> Self {
        Self {
            index,
            model: model.into(),
        }
    }
}

/// A provider's answer: the report text and the models it says it used. This in-process type
/// accepts a partial list. It does not relax the plugin protocol, whose result must be complete.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProviderReply {
    /// The reviewer's report.
    pub report: String,
    /// Reported models, at most one entry per payload item.
    pub items: Vec<ReportedItem>,
}

impl ProviderReply {
    /// A reply with `report` and the reported `items`.
    #[must_use]
    pub fn new(report: impl Into<String>, items: Vec<ReportedItem>) -> Self {
        Self {
            report: report.into(),
            items,
        }
    }
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
    /// The author's model from `[review] author_model`, if set. `--author-model` wins.
    pub author_model: Option<String>,
    /// The reviewer model from `[review] model`, if set. `--reviewer-model` wins.
    pub reviewer_model: Option<String>,
}

/// The plan a review run prints.
#[derive(Debug, Serialize, JsonSchema)]
#[schemars(transform = crate::schema::plan)]
pub struct Plan {
    /// Always 3.
    pub schema_version: SchemaVersion,
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
    /// False for `mechanical`, which never calls a provider.
    pub uses_provider: bool,
    /// The author's model, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(transform = slopvac_model::absent_not_null)]
    pub author_model: Option<String>,
    /// The reviewer's model, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(transform = slopvac_model::absent_not_null)]
    pub reviewer_model: Option<String>,
    /// True when the reviewer model was compared with the author's.
    pub reviewer_model_checked: bool,
    /// Notices that leave the run intact, such as an unchecked model rule.
    pub notices: Vec<Notice>,
    /// The provider's report; absent on `--dry-run`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(transform = slopvac_model::absent_not_null)]
    pub report: Option<String>,
}

/// The `review` subcommand, for the CLI epic to mount.
#[must_use]
pub fn command() -> Command {
    let path = || clap::value_parser!(PathBuf);
    Command::new("review")
        .about(AgentsPhrase::ReviewAbout.as_static())
        .arg(
            Arg::new("revised")
                .value_name("REVISED")
                .required(true)
                .value_parser(path())
                .help(AgentsPhrase::Revised.as_static()),
        )
        .arg(
            Arg::new("original")
                .long("original")
                .value_name("FILE")
                .required(true)
                .value_parser(path())
                .help(AgentsPhrase::Original.as_static()),
        )
        .arg(
            Arg::new("type")
                .long("type")
                .value_parser(["auto", "mechanical", "update", "rewrite"])
                .default_value("auto")
                .help(AgentsPhrase::ReviewType.as_static()),
        )
        .arg(
            Arg::new("genre")
                .long("genre")
                .value_name("GENRE")
                .help(AgentsPhrase::ReviewGenre.as_static()),
        )
        .arg(
            Arg::new("extra")
                .long("extra")
                .value_name("N")
                .value_parser(clap::value_parser!(usize))
                .default_value("0")
                .help(AgentsPhrase::ReviewExtra.as_static()),
        )
        .arg(
            Arg::new("scope-approved")
                .long("scope-approved")
                .action(ArgAction::SetTrue)
                .help(AgentsPhrase::ReviewScopeApproved.as_static()),
        )
        .arg(
            Arg::new("reviewer-model")
                .long("reviewer-model")
                .value_name("MODEL")
                .help(AgentsPhrase::ReviewReviewerModel.as_static()),
        )
        .arg(
            Arg::new("author-model")
                .long("author-model")
                .value_name("MODEL")
                .help(AgentsPhrase::ReviewAuthorModel.as_static()),
        )
        .arg(
            Arg::new("dry-run")
                .long("dry-run")
                .action(ArgAction::SetTrue)
                .help(AgentsPhrase::ReviewDryRun.as_static()),
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
    let report = claims::compare(&original, &revised)?;
    let stats = ChangeStats::from_report(&report);
    let (auto_type, auto_reason) = select_type(
        &stats,
        matches.get_one::<String>("genre").map(String::as_str),
    );
    let (review_type, reason) = match matches
        .get_one::<String>("type")
        .and_then(|t| ReviewType::parse(t))
    {
        Some(t) => {
            check_eligible(t, auto_type, &auto_reason)?;
            (t, AgentsPhrase::ReasonChosen.as_static().to_owned())
        }
        None => (auto_type, auto_reason),
    };
    let scope = Scope::for_report(&report);
    scope.admit(
        matches.get_one::<usize>("extra").copied().unwrap_or(0),
        env.approval,
        matches.get_flag("scope-approved"),
    )?;
    let uses_provider = review_type != ReviewType::Mechanical;
    // A mechanical review calls no reviewer, so the models and their rule do not apply.
    let (author, reviewer, author_source, reviewer_source) = if uses_provider {
        let pick = |flag: Option<String>, config: Option<String>| match (flag, config) {
            (Some(v), _) => (Some(v), "flag"),
            (None, Some(v)) => (Some(v), "config"),
            (None, None) => (None, "none"),
        };
        let (author, author_source) = pick(
            matches.get_one::<String>("author-model").cloned(),
            env.author_model.clone(),
        );
        let (reviewer, reviewer_source) = match pick(
            matches.get_one::<String>("reviewer-model").cloned(),
            env.reviewer_model.clone(),
        ) {
            (None, _) => (env.provider.map(|p| p.model().to_owned()), "provider"),
            found => found,
        };
        (author, reviewer, author_source, reviewer_source)
    } else {
        (None, None, "none", "none")
    };
    let (checked, notices) = if uses_provider {
        precall_notices(
            author.as_deref(),
            author_source,
            reviewer.as_deref(),
            reviewer_source,
            env.provider,
        )?
    } else {
        (false, Vec::new())
    };
    let mut plan = Plan {
        schema_version: SchemaVersion,
        review_type,
        reason,
        stats,
        scope,
        payload: payload(review_type, &report, &original, &revised),
        uses_provider,
        author_model: author,
        reviewer_model: reviewer,
        reviewer_model_checked: checked,
        notices,
        report: None,
    };
    if plan.uses_provider && !matches.get_flag("dry-run") {
        let sources = Sources {
            author: author_source,
            reviewer: reviewer_source,
        };
        plan.report = Some(call_provider(env, &mut plan, &sources)?);
    }
    let mut json = serde_json::to_string_pretty(&plan).map_err(|e| {
        AgentsError::usage(
            AgentsText::ReviewJsonFailed {
                detail: e.to_string(),
            }
            .to_string(),
        )
    })?;
    json.push('\n');
    Ok(json)
}

/// Where the plan got the author and reviewer models: `flag`, `config` or `provider`.
struct Sources {
    author: &'static str,
    reviewer: &'static str,
}

/// Where a payload item sits, as a notice names it under `items`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct ItemLocation {
    /// The payload item's role.
    role: &'static str,
    /// The payload item's line.
    line: usize,
}

/// The `{role, line}` locations of payload items, for a notice's `items` key.
fn item_locations(payload: &Payload, indexes: &[usize]) -> Result<serde_json::Value, AgentsError> {
    let locations: Vec<ItemLocation> = indexes
        .iter()
        .filter_map(|&i| payload.items.get(i))
        .map(|item| ItemLocation {
            role: item.role,
            line: item.line,
        })
        .collect();
    serde_json::to_value(locations).map_err(|e| {
        AgentsError::usage_from(
            AgentsText::ReviewJsonFailed {
                detail: e.to_string(),
            }
            .to_string(),
            e,
        )
    })
}

/// The notices known before any call, and whether the reviewer was compared with the author.
///
/// A reviewer that is the author's model gets `review-model-same`. The provider declares its own
/// model too. When that one matches the author, a reviewer flag naming another model does not
/// hide it. A dry run reports both at no cost.
fn precall_notices(
    author: Option<&str>,
    author_source: &str,
    reviewer: Option<&str>,
    reviewer_source: &str,
    provider: Option<&dyn ModelProvider>,
) -> Result<(bool, Vec<Notice>), AgentsError> {
    let comparison = match (author, reviewer) {
        (Some(a), Some(r)) => compare_models(Some(a), r),
        _ => ModelCheck::Unchecked,
    };
    let mut notices: Vec<Notice> = unchecked_notice(author.is_some(), reviewer.is_some())?
        .into_iter()
        .collect();
    if let (ModelCheck::Same, Some(a), Some(r)) = (comparison, author, reviewer) {
        notices.push(same_model_notice(
            a,
            author_source,
            r,
            reviewer_source,
            None,
        )?);
    }
    if let (ModelCheck::Different, Some(a), Some(p)) = (comparison, author, provider)
        && compare_models(Some(a), p.model()) == ModelCheck::Same
    {
        notices.push(same_model_notice(
            a,
            author_source,
            p.model(),
            "provider",
            None,
        )?);
    }
    Ok((comparison != ModelCheck::Unchecked, notices))
}

/// The notice for a reviewer that is the author's model. `items` is set only when the
/// provider reported the model for those payload items.
fn same_model_notice(
    author: &str,
    author_source: &str,
    reviewer: &str,
    reviewer_source: &str,
    items: Option<serde_json::Value>,
) -> Result<Notice, AgentsError> {
    let mut data = serde_json::Map::new();
    data.insert("author".into(), author.into());
    data.insert("author_source".into(), author_source.into());
    data.insert("reviewer".into(), reviewer.into());
    data.insert("reviewer_source".into(), reviewer_source.into());
    if let Some(items) = items {
        data.insert("items".into(), items);
    }
    Ok(notice(
        NoticeLevel::Warning,
        &AgentsMessage::ModelSame {
            reviewer: quoted(reviewer),
        },
    )?
    .with_data(data))
}

/// The notice for a run whose "different model" rule could not be checked.
fn unchecked_notice(
    author_known: bool,
    reviewer_known: bool,
) -> Result<Option<Notice>, AgentsError> {
    let missing = match (author_known, reviewer_known) {
        (true, true) => return Ok(None),
        (false, true) => AgentsPhrase::MissingAuthor,
        (true, false) => AgentsPhrase::MissingReviewer,
        (false, false) => AgentsPhrase::MissingBoth,
    };
    notice(
        NoticeLevel::Warning,
        &AgentsMessage::ModelUnchecked {
            missing: missing.as_static().to_owned(),
        },
    )
    .map(Some)
}

fn call_provider(
    env: &ReviewEnv<'_>,
    plan: &mut Plan,
    sources: &Sources,
) -> Result<String, AgentsError> {
    if env.offline {
        // The code has one home, in the `slopvac-update` catalog, with the command as a parameter.
        let message = UpdateMessage::OfflineNetworkNeeded {
            command: "review".to_owned(),
        };
        return Err(AgentsError::Coded {
            code: message.code(),
            message: message.to_string(),
            source: None,
        });
    }
    let provider = env
        .provider
        .ok_or_else(|| AgentsError::coded(&AgentsMessage::ProviderMissing))?;
    let declared = provider.model();
    let reply = provider.review(&plan.payload)?;
    check_reply(&reply, &plan.payload)?;
    // Group the reported items by model, in the order models first appear.
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for item in &reply.items {
        match groups
            .iter_mut()
            .find(|(m, _)| model_key(m) == model_key(&item.model))
        {
            Some((_, indexes)) => indexes.push(item.index),
            None => groups.push((item.model.clone(), vec![item.index])),
        }
    }
    let (requested, requested_source) = match plan.reviewer_model.clone() {
        Some(r) => (r, sources.reviewer),
        None => (declared.to_owned(), "provider"),
    };
    for (reported, indexes) in groups {
        let items = item_locations(&plan.payload, &indexes)?;
        if let Some(author) = plan.author_model.clone()
            && compare_models(Some(&author), &reported) == ModelCheck::Same
        {
            plan.notices.push(same_model_notice(
                &author,
                sources.author,
                &reported,
                "reported",
                Some(items.clone()),
            )?);
        }
        if model_key(&requested) != model_key(&reported) {
            let mut data = serde_json::Map::new();
            data.insert("requested".into(), requested.clone().into());
            data.insert("requested_source".into(), requested_source.into());
            data.insert("reported".into(), reported.clone().into());
            data.insert("items".into(), items);
            plan.notices.push(
                notice(
                    NoticeLevel::Info,
                    &AgentsMessage::ModelReportedMismatch {
                        requested: quoted(&requested),
                        reported: quoted(&reported),
                    },
                )?
                .with_data(data),
            );
        }
    }
    Ok(reply.report)
}

/// Reject a reply that names an item the payload lacks, names one twice or reports an empty
/// model. This owned in-process trait accepts a partial list. The production adapter for
/// `provider/complete` must refuse every result that the plugin protocol refuses, with this
/// same error code.
fn check_reply(reply: &ProviderReply, payload: &Payload) -> Result<(), AgentsError> {
    let mut seen = std::collections::BTreeSet::new();
    for item in &reply.items {
        if item.index >= payload.items.len()
            || !seen.insert(item.index)
            || item.model.trim().is_empty()
        {
            return Err(AgentsError::coded(&AgentsMessage::ProviderReplyInvalid {
                index: item.index,
                count: payload.items.len(),
            }));
        }
    }
    Ok(())
}
