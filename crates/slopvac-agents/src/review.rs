//! Review workflow: type selection, scope budget, reviewer model rule, payloads.
//!
//! Policy: `docs/contract/policies.md` section 1.5. A model provider is a plugin.
//! It stays off until trusted and named under `[review] provider`, never runs
//! during lint or offline, and receives only the text the review type needs.

use std::collections::BTreeSet;
use std::path::PathBuf;

use clap::{Arg, ArgAction, ArgMatches, Command};
use schemars::JsonSchema;
use serde::Serialize;

use slopvac_messages::Catalog;
use slopvac_model::{Notice, NoticeLevel, SchemaVersion};
use slopvac_plugins::hooks::{CompleteItem, CompleteRequest, ProviderTask};
use slopvac_update::messages::UpdateMessage;

use crate::claims::{self, Report};
use crate::error::{AgentsError, notice};
use crate::messages::{AgentsMessage, AgentsPhrase, AgentsText, quoted};

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

    /// The `task` a provider is asked for this type, or `None` for `mechanical`, which calls
    /// no provider.
    #[must_use]
    pub fn task(self) -> Option<ProviderTask> {
        match self {
            Self::Mechanical => None,
            Self::Update => Some(ProviderTask::Update),
            Self::Rewrite => Some(ProviderTask::Rewrite),
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

    /// Decide whether `extra` more checks (unchanged claims, more sources) can run. Above the
    /// budget they need the user's confirmation, which `--scope-approved` gives.
    ///
    /// # Errors
    /// `review-scope-approval` when `extra` is over the budget and the user has not confirmed.
    pub fn admit(&self, extra: usize, user_confirmed: bool) -> Result<(), AgentsError> {
        if extra <= self.extra_budget || user_confirmed {
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

/// A model provider: the plugin host's adapter in the binary, or a stand-in in a test.
pub trait ModelProvider {
    /// The provider's name as in `[review] provider`.
    fn name(&self) -> &str;
    /// The model the provider declares before any call, if it declares one. A plugin declares
    /// none: each of its answers reports the model that wrote it.
    fn model(&self) -> Option<&str>;
    /// Send `request` and return the answers.
    ///
    /// # Errors
    /// The provider could not be asked, or its reply breaks the plugin protocol.
    fn review(&self, request: &ProviderRequest<'_>) -> Result<ProviderReply, AgentsError>;
}

/// One item sent to a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SentItem {
    /// The id, unique in the request: the payload position counted from 1, or
    /// `<position>-original` for the sentence before a change.
    pub id: String,
    /// Fixed host text that says what the item is. It holds no document text.
    pub instruction: &'static str,
    /// Document text, exactly as the payload holds it.
    pub text: String,
    /// The payload item this one belongs to, as an index into [`Payload::items`].
    pub item: usize,
    /// True for the sentence before a change, which is context and not part of the report.
    pub context: bool,
}

/// What a provider is asked.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct ProviderRequest<'a> {
    /// The task as the protocol names it: `update` or `rewrite`.
    pub task: &'static str,
    /// The reviewer model from `--reviewer-model`, else `[review] model`.
    pub model: &'a str,
    /// The items, from [`provider_items`], with what each one belongs to.
    pub items: &'a [SentItem],
    /// The `provider/complete` request: the plan's own `request`, sent as it is.
    pub call: &'a CompleteRequest,
}

/// A provider's answer for one sent item.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Answer {
    /// The model's text, which is untrusted, and the model the provider reports for it.
    Text {
        /// The text.
        text: String,
        /// The model the provider reports.
        model: String,
    },
    /// The provider failed this item. `code` is the host's code for the failure, never the
    /// plugin's own.
    Failed {
        /// The host's code.
        code: &'static str,
    },
}

/// The answer for the sent item at `index`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReplyItem {
    /// Index into [`ProviderRequest::items`].
    pub index: usize,
    /// The answer.
    pub answer: Answer,
}

impl ReplyItem {
    /// `text` from `model` for the sent item at `index`.
    #[must_use]
    pub fn text(index: usize, text: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            index,
            answer: Answer::Text {
                text: text.into(),
                model: model.into(),
            },
        }
    }

    /// A failure with the host's `code` for the sent item at `index`.
    #[must_use]
    pub fn failed(index: usize, code: &'static str) -> Self {
        Self {
            index,
            answer: Answer::Failed { code },
        }
    }
}

/// A provider's reply: one answer per sent item, in any order. A reply that leaves an item
/// out, names one twice or names one that was not sent is refused, as the plugin protocol
/// refuses it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProviderReply {
    /// The answers.
    pub items: Vec<ReplyItem>,
}

impl ProviderReply {
    /// A reply with these answers.
    #[must_use]
    pub fn new(items: Vec<ReplyItem>) -> Self {
        Self { items }
    }
}

/// The `review-provider-reply-invalid` refusal, for an adapter whose own protocol check
/// refused the result for `count` sent items (plugin-protocol.md section 5.3).
#[must_use]
pub fn reply_invalid(count: usize) -> AgentsError {
    AgentsError::coded(&AgentsMessage::ProviderReplyInvalid { count })
}

/// A failure of the whole request under the host's `code`, with the provider's own message
/// withheld, because it can quote the document (policies.md section 1.7).
#[must_use]
pub fn request_failed(code: &'static str) -> AgentsError {
    AgentsError::new_coded(
        code,
        AgentsText::ProviderRequestFailed {
            code: code.to_owned(),
        }
        .to_string(),
    )
}

/// A provider that answered and then did not shut down cleanly, under the host's `code`.
#[must_use]
pub fn shutdown_failed(code: &'static str) -> AgentsError {
    AgentsError::new_coded(
        code,
        AgentsText::ProviderShutdownFailed {
            code: code.to_owned(),
        }
        .to_string(),
    )
}

/// The `review-provider-missing` refusal, for a plugin named under `[review] provider` that
/// declares no provider hook, so it is not a model provider.
#[must_use]
pub fn not_a_provider() -> AgentsError {
    AgentsError::coded(&AgentsMessage::ProviderMissing)
}

/// The items a provider receives for `payload`: each payload item's text in an item of its
/// own, and the sentence before a change in a context item right after it. Each item carries a
/// fixed instruction, so no document text reaches the instructions.
#[must_use]
pub fn provider_items(payload: &Payload) -> Vec<SentItem> {
    let mut items = Vec::new();
    for (index, item) in payload.items.iter().enumerate() {
        let position = index + 1;
        items.push(SentItem {
            id: position.to_string(),
            instruction: instruction(item.role, item.original.is_some()).as_static(),
            text: item.text.clone(),
            item: index,
            context: false,
        });
        if let Some(original) = &item.original {
            items.push(SentItem {
                id: format!("{position}-original"),
                instruction: AgentsPhrase::InstructionOriginalContext.as_static(),
                text: original.clone(),
                item: index,
                context: true,
            });
        }
    }
    items
}

/// The `provider/complete` request for `items`, or `None` when nothing would be sent: a type
/// with no task, or no items. The plan prints this value, and a call sends the same value.
/// `model` is the reviewer model a call names, from the flag or the config.
#[must_use]
pub fn provider_request(
    review_type: ReviewType,
    items: &[SentItem],
    model: Option<&str>,
) -> Option<CompleteRequest> {
    let task = review_type.task()?;
    if items.is_empty() {
        return None;
    }
    let sent = items
        .iter()
        .map(|item| CompleteItem {
            id: item.id.clone(),
            instruction: Some(item.instruction.to_owned()),
            text: item.text.clone(),
        })
        .collect();
    Some(CompleteRequest::new(task, model.map(str::to_owned), sent))
}

/// The instruction for a payload item of `role`. The roles are the closed set that
/// [`payload`] builds.
fn instruction(role: &str, with_original: bool) -> AgentsPhrase {
    match (role, with_original) {
        ("changed", true) => AgentsPhrase::InstructionChangedWithOriginal,
        ("changed", false) => AgentsPhrase::InstructionChanged,
        ("added", _) => AgentsPhrase::InstructionAdded,
        ("dropped", _) => AgentsPhrase::InstructionDropped,
        ("original_document", _) => AgentsPhrase::InstructionOriginalDocument,
        _ => AgentsPhrase::InstructionRevisedDocument,
    }
}

/// Everything about the environment that review depends on.
#[derive(Default)]
pub struct ReviewEnv<'a> {
    /// Offline mode blocks every provider call.
    pub offline: bool,
    /// The provider named under `[review] provider`, if any. It starts only when a call is due.
    pub provider: Option<&'a dyn ModelProvider>,
    /// The author's model from `[review] author_model`, if set. `--author-model` wins.
    pub author_model: Option<String>,
    /// The reviewer model from `[review] model`, if set. `--reviewer-model` wins.
    pub reviewer_model: Option<String>,
}

/// Why a review stopped, with the notices that were known before it stopped.
///
/// No plan is printed for a stopped review, so the caller shows `notices` as diagnostics.
#[derive(Debug, thiserror::Error)]
#[error("{error}")]
#[non_exhaustive]
pub struct Stopped {
    /// The failure.
    pub error: AgentsError,
    /// The notices known before the failure, such as `review-model-same`.
    pub notices: Vec<Notice>,
}

impl Stopped {
    /// The stable code of the failure.
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.error.code()
    }
}

impl From<AgentsError> for Box<Stopped> {
    fn from(error: AgentsError) -> Self {
        Box::new(Stopped {
            error,
            notices: Vec::new(),
        })
    }
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
    /// The `provider/complete` request that a call sends: present when the review uses a provider
    /// and the payload has items, also on `--dry-run`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(transform = slopvac_model::absent_not_null)]
    pub request: Option<CompleteRequest>,
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
/// it, a trusted provider must be present, the run must be online and the reviewer model
/// must be named.
///
/// # Errors
/// Why the review stopped, with the notices known before it stopped.
pub fn run(matches: &ArgMatches, env: &ReviewEnv<'_>) -> Result<String, Box<Stopped>> {
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
        matches.get_flag("scope-approved"),
    )?;
    let uses_provider = review_type != ReviewType::Mechanical;
    // A mechanical review calls no reviewer, so the models and their rule do not apply.
    let (author, reviewer, author_source, reviewer_source) = if uses_provider {
        models(matches, env)
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
    let payload = payload(review_type, &report, &original, &revised);
    // The model a call names: only the flag or the config, never one the provider declares.
    let call_model = match reviewer_source {
        "flag" | "config" => reviewer.as_deref(),
        _ => None,
    };
    let request = if uses_provider {
        provider_request(review_type, &provider_items(&payload), call_model)
    } else {
        None
    };
    let mut plan = Plan {
        schema_version: SchemaVersion,
        review_type,
        reason,
        stats,
        scope,
        payload,
        request,
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
        match call_provider(env, &mut plan, &sources) {
            Ok(report) => plan.report = report,
            Err(error) => {
                return Err(Box::new(Stopped {
                    error,
                    notices: plan.notices,
                }));
            }
        }
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
/// The author and reviewer models with where each came from: the flag, then the config. A
/// blank reviewer model counts as unset, so the next source decides; with neither, the model
/// the provider declares stands in.
fn models(
    matches: &ArgMatches,
    env: &ReviewEnv<'_>,
) -> (Option<String>, Option<String>, &'static str, &'static str) {
    let pick = |flag: Option<String>, config: Option<String>| match (flag, config) {
        (Some(v), _) => (Some(v), "flag"),
        (None, Some(v)) => (Some(v), "config"),
        (None, None) => (None, "none"),
    };
    let (author, author_source) = pick(
        matches.get_one::<String>("author-model").cloned(),
        env.author_model.clone(),
    );
    let named = |v: Option<String>| v.filter(|m| !m.trim().is_empty());
    let (reviewer, reviewer_source) = match pick(
        named(matches.get_one::<String>("reviewer-model").cloned()),
        named(env.reviewer_model.clone()),
    ) {
        (None, _) => (
            env.provider.and_then(|p| p.model()).map(str::to_owned),
            "provider",
        ),
        found => found,
    };
    (author, reviewer, author_source, reviewer_source)
}

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
    if let (ModelCheck::Different, Some(a), Some(declared)) =
        (comparison, author, provider.and_then(|p| p.model()))
        && compare_models(Some(a), declared) == ModelCheck::Same
    {
        notices.push(same_model_notice(
            a,
            author_source,
            declared,
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

/// Call the provider and return its report, after every gate that keeps it idle: offline
/// mode, a missing provider and a missing reviewer model. The report is built only once every
/// answer has passed the check and none failed. A payload with no items calls nothing.
fn call_provider(
    env: &ReviewEnv<'_>,
    plan: &mut Plan,
    sources: &Sources,
) -> Result<Option<String>, AgentsError> {
    if env.offline {
        // The code has one home, in the `slopvac-update` catalog, with the command as a parameter.
        let message = UpdateMessage::OfflineNetworkNeeded {
            command: "review".to_owned(),
        };
        return Err(AgentsError::new_coded(message.code(), message.to_string()));
    }
    let provider = env
        .provider
        .ok_or_else(|| AgentsError::coded(&AgentsMessage::ProviderMissing))?;
    // A payload with nothing to send has no request and makes no call, so it needs no model.
    let Some(call) = plan.request.clone() else {
        return Ok(None);
    };
    let model = call
        .model
        .as_deref()
        .ok_or_else(|| AgentsError::coded(&AgentsMessage::ModelMissing))?;
    let model_source = sources.reviewer;
    let items = provider_items(&plan.payload);
    let reply = provider.review(&ProviderRequest {
        task: call.task.as_str(),
        model,
        items: &items,
        call: &call,
    })?;
    let answers = check_reply(&reply, &items)?;
    refuse_failed_items(&answers, &items)?;
    // The text and the model a provider reports are plugin-origin text. They are escaped once,
    // here, for the JSON plan, so the report and the notices carry no live control.
    let escaped: Vec<(&SentItem, String, String)> = items
        .iter()
        .zip(&answers)
        .filter_map(|(sent, answer)| match answer {
            Answer::Text { text, model } => Some((
                sent,
                slopvac_sanitize::structured(text),
                slopvac_sanitize::structured(model),
            )),
            Answer::Failed { .. } => None,
        })
        .collect();
    let answered: Vec<(&SentItem, &str, &str)> = escaped
        .iter()
        .map(|(sent, text, model)| (*sent, text.as_str(), model.as_str()))
        .collect();
    let notices = reported_notices(plan, sources.author, model, model_source, &answered)?;
    plan.notices.extend(notices);
    Ok(Some(assemble_report(&plan.payload, &answered)))
}

/// Group the answers by the model they report, in payload order, and add the notices for a
/// model that is the author's and for one that is not the requested model.
fn reported_notices(
    plan: &Plan,
    author_source: &str,
    requested: &str,
    requested_source: &str,
    answered: &[(&SentItem, &str, &str)],
) -> Result<Vec<Notice>, AgentsError> {
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (sent, _, model) in answered {
        match groups
            .iter_mut()
            .find(|(m, _)| model_key(m) == model_key(model))
        {
            Some((_, indexes)) => {
                if !indexes.contains(&sent.item) {
                    indexes.push(sent.item);
                }
            }
            None => groups.push(((*model).to_owned(), vec![sent.item])),
        }
    }
    let mut notices = Vec::new();
    for (reported, indexes) in groups {
        let items = item_locations(&plan.payload, &indexes)?;
        if let Some(author) = plan.author_model.as_deref()
            && compare_models(Some(author), &reported) == ModelCheck::Same
        {
            notices.push(same_model_notice(
                author,
                author_source,
                &reported,
                "reported",
                Some(items.clone()),
            )?);
        }
        if model_key(requested) != model_key(&reported) {
            let mut data = serde_json::Map::new();
            data.insert("requested".into(), requested.into());
            data.insert("requested_source".into(), requested_source.into());
            data.insert("reported".into(), reported.clone().into());
            data.insert("items".into(), items);
            notices.push(
                notice(
                    NoticeLevel::Info,
                    &AgentsMessage::ModelReportedMismatch {
                        requested: quoted(requested),
                        reported: quoted(&reported),
                    },
                )?
                .with_data(data),
            );
        }
    }
    Ok(notices)
}

/// The answer for each sent item, in the order sent. A reply that names an item twice, names
/// one that was not sent, leaves one out or reports an empty model is refused: the plugin
/// protocol refuses the same results (plugin-protocol.md section 5.3).
fn check_reply<'a>(
    reply: &'a ProviderReply,
    items: &[SentItem],
) -> Result<Vec<&'a Answer>, AgentsError> {
    let invalid = || reply_invalid(items.len());
    let mut slots: Vec<Option<&Answer>> = vec![None; items.len()];
    for answered in &reply.items {
        let slot = slots.get_mut(answered.index).ok_or_else(invalid)?;
        let empty_model =
            matches!(&answered.answer, Answer::Text { model, .. } if model.trim().is_empty());
        if slot.is_some() || empty_model {
            return Err(invalid());
        }
        *slot = Some(&answered.answer);
    }
    slots.into_iter().map(|s| s.ok_or_else(invalid)).collect()
}

/// Stop the review when the provider failed any item. The error has the code of the first
/// failed item in payload order and names every failed item by its position, never with the
/// provider's own text.
fn refuse_failed_items(answers: &[&Answer], items: &[SentItem]) -> Result<(), AgentsError> {
    let mut failed: Vec<(usize, &'static str)> = Vec::new();
    let mut seen = BTreeSet::new();
    for (answer, sent) in answers.iter().zip(items) {
        if let Answer::Failed { code } = answer
            && seen.insert((sent.item, *code))
        {
            failed.push((sent.item + 1, *code));
        }
    }
    let Some(&(_, first)) = failed.first() else {
        return Ok(());
    };
    let listed: Vec<String> = failed
        .iter()
        .map(|&(position, code)| {
            AgentsText::ProviderItemFailed {
                position,
                code: code.to_owned(),
            }
            .to_string()
        })
        .collect();
    Err(AgentsError::new_coded(
        first,
        AgentsText::ProviderItemsFailed {
            items: listed.join(", "),
        }
        .to_string(),
    ))
}

/// The report: one block per payload item in payload order, whatever order the reply came in.
/// A block is the item's heading, then a blank line and the answer with its trailing newlines
/// removed; an empty answer leaves the heading alone. Blocks are separated by a blank line and
/// the report ends with one newline.
fn assemble_report(payload: &Payload, answered: &[(&SentItem, &str, &str)]) -> String {
    let blocks: Vec<String> = answered
        .iter()
        .filter(|(sent, _, _)| !sent.context)
        .filter_map(|(sent, text, _)| {
            let item = payload.items.get(sent.item)?;
            let heading = AgentsText::ReportItem {
                position: sent.item + 1,
                role: item.role.to_owned(),
                line: item.line,
            }
            .to_string();
            let body = text.trim_end_matches(['\n', '\r']);
            Some(if body.is_empty() {
                heading
            } else {
                format!("{heading}\n\n{body}")
            })
        })
        .collect();
    let mut report = blocks.join("\n\n");
    report.push('\n');
    report
}
