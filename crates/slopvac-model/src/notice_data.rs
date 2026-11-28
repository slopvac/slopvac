//! The typed `data` of the notices that name their keys.
//!
//! A notice carries its details as a JSON object. This module gives each shape that the
//! engine writes a type, so the keys and their value types are fixed in one place and the
//! published `lint-report.schema.json` declares them per notice code.

use std::num::NonZeroUsize;

use schemars::{JsonSchema, Schema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::NoticeError;
use crate::report::Count;
use crate::schema::{absent_not_null, if_then};
use crate::values::present;

const RULE_KIND_NOT_IMPLEMENTED: &str = "rule-kind-not-implemented";
const RULE_STYLE_NOT_IMPLEMENTED: &str = "rule-style-not-implemented";
const COMMENT_RULES_NOT_RUN: &str = "comment-rules-not-run";
const COMMENT_RULE_NAMED_NOT_RUN: &str = "comment-rule-named-not-run";
const CASES_RULE_UNCHECKED: &str = "cases-rule-unchecked";
const EXTRACT_UNSUPPORTED: &str = "extract-unsupported";
const EXTRACT_BUDGET: &str = "extract-budget";
const CONTEXT_BUDGET_EXCEEDED: &str = "context-budget-exceeded";
const CONFIG_BUDGET_EXCEEDED: &str = "config-budget-exceeded";
const CONFIG_UNKNOWN_RULE: &str = "config-unknown-rule";
const REVIEW_MODEL_SAME: &str = "review-model-same";
const REVIEW_MODEL_REPORTED_MISMATCH: &str = "review-model-reported-mismatch";

/// Notice codes whose `data` is a [`RulesNoticeData`].
const RULES_CODES: &[&str] = &[
    RULE_KIND_NOT_IMPLEMENTED,
    RULE_STYLE_NOT_IMPLEMENTED,
    COMMENT_RULES_NOT_RUN,
    COMMENT_RULE_NAMED_NOT_RUN,
    CASES_RULE_UNCHECKED,
];

/// Notice codes whose `data` is an [`UnsupportedNoticeData`].
const UNSUPPORTED_CODES: &[&str] = &[EXTRACT_UNSUPPORTED];

/// Notice codes whose `data` is a [`BudgetNoticeData`].
const BUDGET_CODES: &[&str] = &[EXTRACT_BUDGET];

/// Notice codes whose `data` is a [`PathBudgetNoticeData`].
const PATH_BUDGET_CODES: &[&str] = &[CONTEXT_BUDGET_EXCEEDED, CONFIG_BUDGET_EXCEEDED];

/// Notice codes whose `data` is an [`UnknownRuleNoticeData`].
const UNKNOWN_RULE_CODES: &[&str] = &[CONFIG_UNKNOWN_RULE];

/// Notice codes whose `data` is a [`ModelSameNoticeData`].
const MODEL_SAME_CODES: &[&str] = &[REVIEW_MODEL_SAME];

/// Notice codes whose `data` is a [`ModelMismatchNoticeData`].
const MODEL_MISMATCH_CODES: &[&str] = &[REVIEW_MODEL_REPORTED_MISMATCH];

/// The rules a notice is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct RulesNoticeData {
    /// Ids of the rules the notice names, in the order the notice lists them.
    pub rules: Vec<String>,
}

/// The input that no extractor reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct UnsupportedNoticeData {
    /// Path of the document that holds the unsupported content.
    pub document: String,
    /// Name of the unsupported format or diagram type.
    pub kind: String,
}

/// The cap that stopped an extraction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct BudgetNoticeData {
    /// Name of the cap that was reached.
    pub budget: String,
    /// Value of the cap: a count, or a number of bytes, as the cap name says.
    #[schemars(with = "Count")]
    pub limit: u64,
}

/// A budget that stopped work at a path, such as a document or a config file.
///
/// A sibling of [`BudgetNoticeData`]: an extraction budget never names a path, and a budget
/// code that dropped its path must not match the schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct PathBudgetNoticeData {
    /// Name of the budget that was passed. The list of names grows in 3.x.
    pub budget: String,
    /// Limit of the budget, in the unit that policies.md, section 1.9, gives for it.
    #[schemars(with = "Count")]
    pub limit: u64,
    /// The document or config file at which the budget tripped, as the report shows it:
    /// relative to the root, `<external>/<file name>` outside it, or `<stdin>`.
    pub path: String,
}

/// A rule id that a config layer names and no rule has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct UnknownRuleNoticeData {
    /// The id as the config wrote it, qualified or bare.
    pub rule: String,
    /// One-based line of the key. Absent without a line.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[schemars(transform = absent_not_null)]
    pub line: Option<NonZeroUsize>,
    /// One-based column of the key. Absent without a column.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[schemars(transform = absent_not_null)]
    pub column: Option<NonZeroUsize>,
}

/// What a review payload item is.
///
/// The review plan's payload items and the `items` of the review notices share this list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(inline)]
pub enum ItemRole {
    /// A sentence the revision changed.
    Changed,
    /// A sentence the revision added.
    Added,
    /// An original sentence the revision dropped.
    Dropped,
    /// The whole original document.
    OriginalDocument,
    /// The whole revised document.
    RevisedDocument,
}

impl ItemRole {
    /// Every role, in the order the schema lists them.
    pub const ALL: [Self; 5] = [
        Self::Changed,
        Self::Added,
        Self::Dropped,
        Self::OriginalDocument,
        Self::RevisedDocument,
    ];

    /// The name the role has on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Changed => "changed",
            Self::Added => "added",
            Self::Dropped => "dropped",
            Self::OriginalDocument => "original_document",
            Self::RevisedDocument => "revised_document",
        }
    }
}

/// Where a payload item sits, as a review notice names it under `items`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ItemLocation {
    /// The payload item's role.
    pub role: ItemRole,
    /// The payload item's one-based line: in the revision, except for `dropped` and
    /// `original_document`, which sit in the original.
    pub line: NonZeroUsize,
}

impl ItemLocation {
    /// A location with its role and line.
    #[must_use]
    pub const fn new(role: ItemRole, line: NonZeroUsize) -> Self {
        Self { role, line }
    }
}

/// Where the author model of a review came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthorSource {
    /// The `--author-model` flag.
    Flag,
    /// The config.
    Config,
    /// No author model is set.
    #[serde(rename = "none")]
    Unset,
}

/// Where the reviewer model that matches the author came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewerSource {
    /// The `--reviewer-model` flag.
    Flag,
    /// The config.
    Config,
    /// The model the provider declares.
    Provider,
    /// The model the provider reported for the items.
    Reported,
}

/// Where the model a review asked for came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequestedSource {
    /// The `--reviewer-model` flag.
    Flag,
    /// The config.
    Config,
}

/// A reviewer that is the author's model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = model_same_conditions)]
#[non_exhaustive]
pub struct ModelSameNoticeData {
    /// The author model name.
    pub author: String,
    /// Where the author model came from.
    pub author_source: AuthorSource,
    /// The reviewer model name that matches the author.
    pub reviewer: String,
    /// Where the reviewer model came from.
    pub reviewer_source: ReviewerSource,
    /// The payload items the reported model covers. Present when `reviewer_source` is
    /// `reported`, and absent otherwise.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[schemars(transform = absent_not_null, length(min = 1))]
    pub items: Option<Vec<ItemLocation>>,
}

/// A reported model that is not the model the review asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ModelMismatchNoticeData {
    /// The reviewer model the review asked for.
    pub requested: String,
    /// Where the requested model came from.
    pub requested_source: RequestedSource,
    /// The model the provider reports.
    pub reported: String,
    /// The payload items the reported model covers.
    #[schemars(length(min = 1))]
    pub items: Vec<ItemLocation>,
}

/// `items` is required when the reviewer is the reported model, and absent otherwise.
fn model_same_conditions(schema: &mut Schema) {
    let conditions = vec![
        if_then(
            &json!({"properties": {"reviewer_source": {"const": "reported"}}, "required": ["reviewer_source"]}),
            &json!({"required": ["items"]}),
        ),
        if_then(
            &json!({
                "properties": {"reviewer_source": {"enum": ["flag", "config", "provider"]}},
                "required": ["reviewer_source"],
            }),
            &json!({"not": {"required": ["items"]}}),
        ),
    ];
    schema.insert("allOf".into(), Value::Array(conditions));
}

impl ModelSameNoticeData {
    /// Whether `items` agrees with `reviewer_source`: at least one item for a reported
    /// reviewer, none for any other.
    fn items_agree(&self) -> bool {
        match (&self.reviewer_source, &self.items) {
            (ReviewerSource::Reported, Some(items)) => !items.is_empty(),
            (ReviewerSource::Reported, None) => false,
            (_, items) => items.is_none(),
        }
    }
}

/// The data of a typed notice code, with that code. Each constructor serves one code, so a
/// code and a shape that do not match cannot be built. [`crate::Notice::from_catalog_data`]
/// takes the notice code from here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeData {
    code: &'static str,
    shape: Shape,
}

/// The shape behind a [`NoticeData`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum Shape {
    Rules(RulesNoticeData),
    Unsupported(UnsupportedNoticeData),
    Budget(BudgetNoticeData),
    PathBudget(PathBudgetNoticeData),
    UnknownRule(UnknownRuleNoticeData),
    ModelSame(ModelSameNoticeData),
    ModelMismatch(ModelMismatchNoticeData),
}

impl NoticeData {
    fn rules<I, S>(code: &'static str, rules: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            code,
            shape: Shape::Rules(RulesNoticeData {
                rules: rules.into_iter().map(Into::into).collect(),
            }),
        }
    }

    /// Data for `rule-kind-not-implemented`: the rules of a kind the engine does not run.
    #[must_use]
    pub fn rule_kind_not_implemented<I, S>(rules: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::rules(RULE_KIND_NOT_IMPLEMENTED, rules)
    }

    /// Data for `rule-style-not-implemented`: the capitalization rules of a style the engine
    /// does not run.
    #[must_use]
    pub fn rule_style_not_implemented<I, S>(rules: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::rules(RULE_STYLE_NOT_IMPLEMENTED, rules)
    }

    /// Data for `comment-rules-not-run`: the default rules that cannot run on comments.
    #[must_use]
    pub fn comment_rules_not_run<I, S>(rules: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::rules(COMMENT_RULES_NOT_RUN, rules)
    }

    /// Data for `comment-rule-named-not-run`: the rules named by the user that cannot run on
    /// comments.
    #[must_use]
    pub fn comment_rule_named_not_run<I, S>(rules: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::rules(COMMENT_RULE_NAMED_NOT_RUN, rules)
    }

    /// Data for `cases-rule-unchecked`: the rules the case runner could not check.
    #[must_use]
    pub fn cases_rule_unchecked<I, S>(rules: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::rules(CASES_RULE_UNCHECKED, rules)
    }

    /// Data for `extract-unsupported`: content that no extractor reads.
    #[must_use]
    pub fn extract_unsupported(document: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            code: EXTRACT_UNSUPPORTED,
            shape: Shape::Unsupported(UnsupportedNoticeData {
                document: document.into(),
                kind: kind.into(),
            }),
        }
    }

    /// Data for `extract-budget`: an extraction that stopped at a cap.
    #[must_use]
    pub fn extract_budget(budget: impl Into<String>, limit: u64) -> Self {
        Self {
            code: EXTRACT_BUDGET,
            shape: Shape::Budget(BudgetNoticeData {
                budget: budget.into(),
                limit,
            }),
        }
    }

    /// Data for `context-budget-exceeded`: a document that passed a context budget. `path` is
    /// the document as the report shows it.
    #[must_use]
    pub fn context_budget_exceeded(
        budget: impl Into<String>,
        limit: u64,
        path: impl Into<String>,
    ) -> Self {
        Self::path_budget(CONTEXT_BUDGET_EXCEEDED, budget, limit, path)
    }

    /// Data for `config-budget-exceeded`: a config load that passed a budget. `path` is the
    /// config file as the report shows it.
    #[must_use]
    pub fn config_budget_exceeded(
        budget: impl Into<String>,
        limit: u64,
        path: impl Into<String>,
    ) -> Self {
        Self::path_budget(CONFIG_BUDGET_EXCEEDED, budget, limit, path)
    }

    fn path_budget(
        code: &'static str,
        budget: impl Into<String>,
        limit: u64,
        path: impl Into<String>,
    ) -> Self {
        Self {
            code,
            shape: Shape::PathBudget(PathBudgetNoticeData {
                budget: budget.into(),
                limit,
                path: path.into(),
            }),
        }
    }

    /// Data for `config-unknown-rule`: a rule id that a config layer names and no rule has.
    #[must_use]
    pub fn config_unknown_rule(
        rule: impl Into<String>,
        line: Option<NonZeroUsize>,
        column: Option<NonZeroUsize>,
    ) -> Self {
        Self {
            code: CONFIG_UNKNOWN_RULE,
            shape: Shape::UnknownRule(UnknownRuleNoticeData {
                rule: rule.into(),
                line,
                column,
            }),
        }
    }

    /// Data for `review-model-same`: a reviewer that is the author's model. `items` lists
    /// the payload items the reported model covers, and is empty unless `reviewer_source` is
    /// [`ReviewerSource::Reported`], when it needs at least one.
    ///
    /// # Errors
    /// [`NoticeError::DataShape`] when `items` and `reviewer_source` do not agree.
    pub fn review_model_same(
        author: impl Into<String>,
        author_source: AuthorSource,
        reviewer: impl Into<String>,
        reviewer_source: ReviewerSource,
        items: Vec<ItemLocation>,
    ) -> Result<Self, NoticeError> {
        let data = ModelSameNoticeData {
            author: author.into(),
            author_source,
            reviewer: reviewer.into(),
            reviewer_source,
            items: (!items.is_empty()).then_some(items),
        };
        if !data.items_agree() {
            return Err(NoticeError::DataShape(REVIEW_MODEL_SAME.to_owned()));
        }
        Ok(Self {
            code: REVIEW_MODEL_SAME,
            shape: Shape::ModelSame(data),
        })
    }

    /// Data for `review-model-reported-mismatch`: a reported model that is not the requested
    /// one. `items` lists the payload items the reported model covers, at least one.
    ///
    /// # Errors
    /// [`NoticeError::DataShape`] when `items` is empty.
    pub fn review_model_reported_mismatch(
        requested: impl Into<String>,
        requested_source: RequestedSource,
        reported: impl Into<String>,
        items: Vec<ItemLocation>,
    ) -> Result<Self, NoticeError> {
        if items.is_empty() {
            return Err(NoticeError::DataShape(
                REVIEW_MODEL_REPORTED_MISMATCH.to_owned(),
            ));
        }
        Ok(Self {
            code: REVIEW_MODEL_REPORTED_MISMATCH,
            shape: Shape::ModelMismatch(ModelMismatchNoticeData {
                requested: requested.into(),
                requested_source,
                reported: reported.into(),
                items,
            }),
        })
    }

    /// The notice code this data belongs to.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }

    /// Whether `code` is the code of this data.
    #[must_use]
    pub fn serves(&self, code: &str) -> bool {
        self.code == code
    }

    /// Whether `data` has the shape that `code` declares. A code with no typed shape accepts
    /// any object. Unknown keys are allowed, as in the schema. The check reads the same
    /// shape types that generate the schema.
    pub(crate) fn fits(code: &str, data: &Value) -> bool {
        if RULES_CODES.contains(&code) {
            RulesNoticeData::deserialize(data).is_ok()
        } else if UNSUPPORTED_CODES.contains(&code) {
            UnsupportedNoticeData::deserialize(data).is_ok()
        } else if BUDGET_CODES.contains(&code) {
            BudgetNoticeData::deserialize(data).is_ok()
        } else if PATH_BUDGET_CODES.contains(&code) {
            PathBudgetNoticeData::deserialize(data).is_ok()
        } else if UNKNOWN_RULE_CODES.contains(&code) {
            UnknownRuleNoticeData::deserialize(data).is_ok()
        } else if MODEL_SAME_CODES.contains(&code) {
            ModelSameNoticeData::deserialize(data).is_ok_and(|d| d.items_agree())
        } else if MODEL_MISMATCH_CODES.contains(&code) {
            ModelMismatchNoticeData::deserialize(data).is_ok_and(|d| !d.items.is_empty())
        } else {
            true
        }
    }

    /// The object a notice writes for this data.
    pub(crate) fn into_map(self) -> Map<String, Value> {
        // The shapes hold only strings and integers, so they always serialize to an object.
        // A failure would drop the data, never panic, and the assertion catches it in tests.
        let value = serde_json::to_value(&self.shape);
        debug_assert!(
            matches!(value, Ok(Value::Object(_))),
            "notice data serializes to an object"
        );
        match value {
            Ok(Value::Object(map)) => map,
            _ => Map::new(),
        }
    }
}

/// The schema of `Notice.data`: an object, with every typed shape registered in `$defs` so
/// the per-code conditions of [`data_conditions`] can name them.
pub(crate) fn data_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    generator.subschema_for::<RulesNoticeData>();
    generator.subschema_for::<UnsupportedNoticeData>();
    generator.subschema_for::<BudgetNoticeData>();
    generator.subschema_for::<PathBudgetNoticeData>();
    generator.subschema_for::<UnknownRuleNoticeData>();
    generator.subschema_for::<ModelSameNoticeData>();
    generator.subschema_for::<ModelMismatchNoticeData>();
    generator.subschema_for::<ItemLocation>();
    schemars::json_schema!({ "type": "object" })
}

/// The `allOf` entries of the notice schema: for each typed code, `if code then data` is the
/// shape. A code with no typed shape keeps the generic object. New groups go last, so the
/// entries before them keep their index.
pub(crate) fn data_conditions() -> Value {
    let groups: [(&[&str], &str); 7] = [
        (RULES_CODES, "RulesNoticeData"),
        (UNSUPPORTED_CODES, "UnsupportedNoticeData"),
        (BUDGET_CODES, "BudgetNoticeData"),
        (PATH_BUDGET_CODES, "PathBudgetNoticeData"),
        (UNKNOWN_RULE_CODES, "UnknownRuleNoticeData"),
        (MODEL_SAME_CODES, "ModelSameNoticeData"),
        (MODEL_MISMATCH_CODES, "ModelMismatchNoticeData"),
    ];
    Value::Array(
        groups
            .iter()
            .flat_map(|(codes, shape)| {
                codes.iter().map(move |code| {
                    if_then(
                        &json!({"properties": {"code": {"const": code}}, "required": ["code"]}),
                        &json!({"properties": {"data": {"$ref": format!("#/$defs/{shape}")}}}),
                    )
                })
            })
            .collect(),
    )
}
