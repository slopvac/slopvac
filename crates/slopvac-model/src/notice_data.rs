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

/// Notice codes whose `data` is a [`RulesNoticeData`].
const RULES_CODES: &[&str] = &[
    "rule-kind-not-implemented",
    "rule-style-not-implemented",
    "comment-rules-not-run",
    "comment-rule-named-not-run",
    "cases-rule-unchecked",
];

/// Notice codes whose `data` is an [`UnsupportedNoticeData`].
const UNSUPPORTED_CODES: &[&str] = &["extract-unsupported"];

/// Notice codes whose `data` is a [`BudgetNoticeData`].
const BUDGET_CODES: &[&str] = &["extract-budget"];

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

/// The typed `data` of a notice. Attach it with [`crate::Notice::with`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum NoticeData {
    /// The rules a notice names.
    Rules(RulesNoticeData),
    /// An input that no extractor reads.
    Unsupported(UnsupportedNoticeData),
    /// A cap that stopped an extraction.
    Budget(BudgetNoticeData),
}

impl NoticeData {
    /// Data that lists rule ids.
    #[must_use]
    pub fn rules<I, S>(rules: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::Rules(RulesNoticeData {
            rules: rules.into_iter().map(Into::into).collect(),
        })
    }

    /// Data for content that no extractor reads.
    #[must_use]
    pub fn unsupported(document: impl Into<String>, kind: impl Into<String>) -> Self {
        Self::Unsupported(UnsupportedNoticeData {
            document: document.into(),
            kind: kind.into(),
        })
    }

    /// Data for an extraction that stopped at a cap.
    #[must_use]
    pub fn budget(budget: impl Into<String>, limit: u64) -> Self {
        Self::Budget(BudgetNoticeData {
            budget: budget.into(),
            limit,
        })
    }

    /// The notice codes that carry this shape. A debug build checks them in
    /// [`crate::Notice::with`].
    #[must_use]
    pub const fn codes(&self) -> &'static [&'static str] {
        match self {
            Self::Rules(_) => RULES_CODES,
            Self::Unsupported(_) => UNSUPPORTED_CODES,
            Self::Budget(_) => BUDGET_CODES,
        }
    }

    /// Whether `code` is a code that carries this shape.
    #[must_use]
    pub fn serves(&self, code: &str) -> bool {
        self.codes().contains(&code)
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
        let value = serde_json::to_value(self);
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
