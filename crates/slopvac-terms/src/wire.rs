//! Schema-only types for the terms file and the blocklist.
//!
//! The readers in [`crate::shape`] and [`crate::blocklist`] validate a document themselves, so
//! each error keeps its entry number and its coded reason. These types describe the same
//! documents so the generator can write `schemas/v3/terms.schema.json` and
//! `schemas/v3/blocklist.schema.json`. No code reads a value of them. The casing defaults, the
//! part-of-speech spellings, the length floor and the schema version come from constants the
//! readers also read. The kind pattern and the non-blank pattern are bound to the readers'
//! predicates by property tests in `tests/schema_patterns.rs`, and
//! `slopvac-schemas/tests/terms_agreement.rs` runs the conformance corpus through both sides.

use schemars::generate::Contract;
use schemars::transform::RecursiveTransform;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde_json::{Value, json};
use slopvac_model::{SchemaVersion, absent_not_null, schema_settings, x_extensions};

use crate::blocklist::{NON_BLANK_PATTERN, pos_names};
use crate::shape::{KIND_PATTERN, MIN_TEXT_LEN, default_casing};

/// The generator both schemas use: the shared v3 settings, for what a reader accepts. Every
/// member is absent or a real value, never `null`, so each `Option` loses the `null`
/// alternative schemars adds.
fn generator() -> SchemaGenerator {
    schema_settings(Contract::Deserialize)
        .with_transform(RecursiveTransform(absent_not_null))
        .into_generator()
}

/// The schema of a terms file, with its `$defs`. `slopvac-schemas` writes it to
/// `schemas/v3/terms.schema.json`.
#[must_use]
pub fn terms_schema() -> Value {
    generator()
        .into_root_schema_for::<TermsDocument>()
        .to_value()
}

/// The schema of a blocklist, with its `$defs`. `slopvac-schemas` writes it to
/// `schemas/v3/blocklist.schema.json`.
#[must_use]
pub fn blocklist_schema() -> Value {
    generator()
        .into_root_schema_for::<BlocklistDocument>()
        .to_value()
}

// A string of at least `MIN_TEXT_LEN` characters.
#[derive(JsonSchema)]
#[schemars(inline, with = "String", extend("minLength" = MIN_TEXT_LEN))]
pub(crate) struct Text;

#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct TermsDocument {
    schema_version: SchemaVersion,
    terms: Vec<Term>,
}

/// One term. Only `term` is required. The canonical spelling is `term`. A listed variant is a
/// wrong spelling that a check may flag and fix to the canonical one.
#[derive(JsonSchema)]
#[schemars(transform = closed_term)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
#[expect(
    clippy::struct_field_names,
    reason = "the member is named `term` in the published file"
)]
pub(crate) struct Term {
    term: Text,
    #[schemars(extend("uniqueItems" = true))]
    variants: Option<Vec<Text>>,
    /// What the term is, as a kebab-case label. Known values: product, language, protocol,
    /// acronym, proper-noun, identifier and jargon. A check may treat acronyms and identifiers
    /// differently from prose words.
    #[schemars(extend("pattern" = KIND_PATTERN))]
    kind: Option<String>,
    /// Accept any casing. The term then works as an allow-list entry and no casing finding
    /// fires for it.
    #[serde(default = "default_casing")]
    ignore_case: bool,
    /// Match only the exact spelling of `term` and its variants. Other casings are different
    /// words and never flag. For names that are also common words, such as Go, Rust, React,
    /// Windows and REST: `go` in running text is not a finding. Cannot combine with
    /// ignore_case.
    #[expect(
        clippy::doc_markdown,
        reason = "the description names the member `ignore_case` as the published text does"
    )]
    #[serde(default = "default_casing")]
    case_sensitive: bool,
}

/// A closed object, and the two casing modes exclude each other: a term never sets both to
/// true.
fn closed_term(schema: &mut Schema) {
    x_extensions(schema);
    schema.insert(
        "not".into(),
        json!({
            "required": ["ignore_case", "case_sensitive"],
            "properties": {
                "ignore_case": {"const": true},
                "case_sensitive": {"const": true},
            },
        }),
    );
}

#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct BlocklistDocument {
    schema_version: SchemaVersion,
    entries: Vec<BlocklistEntry>,
}

// A part of speech, spelled as the blocklist lists it.
#[derive(JsonSchema)]
#[schemars(inline, with = "String", extend("enum" = pos_names()))]
pub(crate) struct PosForm;

/// One refused word. The word is trimmed and lower-cased on load. A word and part of speech
/// pair appears once.
#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct BlocklistEntry {
    #[schemars(extend("pattern" = NON_BLANK_PATTERN))]
    word: String,
    /// Part of speech. The values are case-sensitive and load as their long form: the
    /// lowercase short and long forms, and the spaCy tags NOUN, VERB, ADJ, ADV, ADP, CCONJ,
    /// SCONJ, PRON and DET.
    pos: PosForm,
    /// Why the word is refused. Required: an entry with no reason cannot be reviewed.
    #[schemars(extend("pattern" = NON_BLANK_PATTERN))]
    reason: String,
    /// A note on what to write instead.
    replacement: Option<String>,
}
