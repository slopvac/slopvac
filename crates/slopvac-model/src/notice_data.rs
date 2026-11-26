//! The typed `data` of the notices that name their keys.
//!
//! A notice carries its details as a JSON object. This module gives each shape that the
//! engine writes a type, so the keys and their value types are fixed in one place and the
//! published `lint-report.schema.json` declares them per notice code.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::report::Count;
use crate::schema::if_then;

const RULE_KIND_NOT_IMPLEMENTED: &str = "rule-kind-not-implemented";
const RULE_STYLE_NOT_IMPLEMENTED: &str = "rule-style-not-implemented";
const COMMENT_RULES_NOT_RUN: &str = "comment-rules-not-run";
const COMMENT_RULE_NAMED_NOT_RUN: &str = "comment-rule-named-not-run";
const CASES_RULE_UNCHECKED: &str = "cases-rule-unchecked";
const EXTRACT_UNSUPPORTED: &str = "extract-unsupported";
const EXTRACT_BUDGET: &str = "extract-budget";

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
    schemars::json_schema!({ "type": "object" })
}

/// The `allOf` entries of the notice schema: for each typed code, `if code then data` is the
/// shape. A code with no typed shape keeps the generic object.
pub(crate) fn data_conditions() -> Value {
    let groups: [(&[&str], &str); 3] = [
        (RULES_CODES, "RulesNoticeData"),
        (UNSUPPORTED_CODES, "UnsupportedNoticeData"),
        (BUDGET_CODES, "BudgetNoticeData"),
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
