//! The published schema of the plugin protocol, `schemas/v3/plugin-protocol.schema.json`.
//!
//! The wire types of the host derive their definitions: the lifecycle types in
//! [`protocol`](super), the hook types in [`hooks`](crate::hooks) and the request types in
//! `requests`. This module holds what no host type describes yet: the shapes of replies that
//! the host reads field by field, the binary extractor shapes, and the small helper schemas that
//! combine them (a batch, a per-document entry, the response dispatch). [`METHODS`] is the one
//! table of methods. The `Call` definition and the result definitions come from it.
//!
//! No object here sets `additionalProperties` to false: a reader ignores unknown fields, and keys
//! with the prefix `x-` carry no meaning.

use std::borrow::Cow;
use std::marker::PhantomData;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde_json::{Map, Value, json};
use slopvac_model::{
    ApiVersion, ContextTag, KEBAB, SLUG, SchemaVersion, absent_not_null as absent,
};

use super::requests::{
    CancelParams, CompleteParams, ExtractParams, FixParams, InitializeParams, JsonRpc,
    RecipeParams, RenderParams, RequestId, ScoreParams, TagParams,
};
use super::{InitializeResult, PluginDeclaration, method, text};
use crate::hooks::{Document, Finding, FixEdit, RecipeFile, Rendered, Segment, Tag};

/// The base64 form of a binary document: standard alphabet, padding, no line breaks.
const BASE64: &str = r"^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?(?![\s\S])";
/// A container part or locator: 1 to 1024 characters with no control or bidirectional-control
/// character.
const PART: &str = r"^[^\u0000-\u001F\u007F-\u009F\u202A-\u202E\u2066-\u2069]{1,1024}(?![\s\S])";
/// A feature name: a slug, or a private extension with the prefix `x-`. The text is the
/// published string, byte for byte. Its two alternatives are not in the pattern subset, so
/// `patterns.rs` lists it as pending.
const FEATURE_NAME: &str = r"^[a-z][a-z0-9-]*(?![\s\S])|^x-[a-z0-9-]+(?![\s\S])";
/// The largest integer a JSON number carries without loss.
const MAX_SAFE: u64 = crate::host::MAX_REQUEST_ID;

// ---- transforms

/// Drop the description of each member of an object schema. The doc comments of the fields are
/// for readers of the code. The published schema describes each definition and not its members.
pub(crate) fn bare_fields(schema: &mut Schema) {
    if let Some(Value::Object(properties)) = schema.get_mut("properties") {
        for member in properties.values_mut() {
            if let Value::Object(member) = member {
                member.remove("description");
            }
        }
    }
}

/// [`bare_fields`], and drop the description of the object itself, for an inline object that the
/// published schema does not describe.
pub(crate) fn bare_entry(schema: &mut Schema) {
    bare_fields(schema);
    schema.remove("description");
}

/// Remove `name` from `required`.
fn optional(schema: &mut Schema, name: &str) {
    if let Some(Value::Array(required)) = schema.get_mut("required") {
        required.retain(|member| member != name);
    }
    if schema
        .get("required")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        schema.remove("required");
    }
}

/// `minProtocolVersion` defaults to 1, so a message may leave it out.
pub(crate) fn min_version_is_optional(schema: &mut Schema) {
    optional(schema, "minProtocolVersion");
}

/// `options` defaults to an empty object, so a message may leave it out.
pub(crate) fn options_are_optional(schema: &mut Schema) {
    optional(schema, "options");
}

/// Describe `requiresNetwork`, the one member of the `initialize` reply that the schema
/// describes on its own.
pub(crate) fn network_note(schema: &mut Schema) {
    if let Some(Value::Object(properties)) = schema.get_mut("properties")
        && let Some(Value::Object(member)) = properties.get_mut("requiresNetwork")
    {
        member.insert("description".into(), text::REQUIRES_NETWORK.into());
    }
}

// ---- names and values

/// Hook point name. The kind is open, so the schema fixes only its form.
#[derive(JsonSchema)]
#[schemars(
    rename = "HookKind",
    description = text::HOOK_KIND,
    with = "String",
    extend("pattern" = SLUG)
)]
pub(crate) struct HookKindSchema;

/// Plugin-local hook identifier.
#[derive(JsonSchema)]
#[schemars(
    rename = "HookId",
    description = text::HOOK_ID,
    with = "String",
    extend("pattern" = SLUG)
)]
pub(crate) struct HookIdSchema;

/// Optional capability name.
#[derive(JsonSchema)]
#[schemars(
    rename = "FeatureName",
    description = text::FEATURE_NAME,
    with = "String",
    extend("pattern" = FEATURE_NAME)
)]
pub(crate) struct FeatureNameSchema;

/// Protocol version: the value space of the plugin API version.
#[derive(JsonSchema)]
#[schemars(description = text::PROTOCOL_VERSION)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the field and no code reads a value"
)]
pub(crate) struct ProtocolVersion(ApiVersion);

/// The review types of `provider/complete`.
#[derive(JsonSchema)]
#[schemars(inline, rename_all = "kebab-case")]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the variants and no code builds one"
)]
pub(crate) enum TaskKind {
    FactCheck,
    Update,
    Rewrite,
}

/// Any JSON value: the schema of `result`, whose shape depends on the method.
struct Anything;

impl JsonSchema for Anything {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "Anything".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({})
    }
}

/// A JSON object with no further rule.
pub(crate) struct ObjectSchema;

impl JsonSchema for ObjectSchema {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "ObjectSchema".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type": "object"})
    }
}

/// The part of a lint report that the protocol checks: its schema version. The lint report
/// schema owns the rest.
#[derive(JsonSchema)]
#[schemars(inline, transform = bare_entry)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct ReportStub {
    schema_version: SchemaVersion,
}

/// A request id, or `null` when the sender could not read the id of the request.
struct NullableId;

impl JsonSchema for NullableId {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "NullableId".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let id = generator.subschema_for::<RequestId>();
        json_schema!({"oneOf": [id, {"type": "null"}]})
    }
}

// ---- arrays

/// An array of unique `T`.
pub(crate) struct UniqueItems<T>(PhantomData<T>);

impl<T: JsonSchema> JsonSchema for UniqueItems<T> {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        format!("UniqueItems_{}", T::schema_name()).into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let items = generator.subschema_for::<T>();
        json_schema!({"type": "array", "items": items, "uniqueItems": true})
    }
}

/// An array of at least one `T`.
pub(crate) struct MinOne<T>(PhantomData<T>);

impl<T: JsonSchema> JsonSchema for MinOne<T> {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        format!("MinOne_{}", T::schema_name()).into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let items = generator.subschema_for::<T>();
        json_schema!({"type": "array", "items": items, "minItems": 1})
    }
}

/// An array of at least one unique `T`: a batch.
struct Batch<T>(PhantomData<T>);

impl<T: JsonSchema> JsonSchema for Batch<T> {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        format!("Batch_{}", T::schema_name()).into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let items = generator.subschema_for::<T>();
        json_schema!({"type": "array", "items": items, "minItems": 1, "uniqueItems": true})
    }
}

/// The batch of documents of one request.
pub(crate) struct Documents;

impl JsonSchema for Documents {
    fn schema_name() -> Cow<'static, str> {
        "Documents".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let items = generator.subschema_for::<Document>();
        json_schema!({
            "description": text::DOCUMENTS,
            "type": "array",
            "items": items,
            "minItems": 1,
            "uniqueItems": true
        })
    }
}

/// An object that must satisfy `B` and the members of `E`, as one entry of a `results` or
/// `documents` array.
pub(crate) struct Over<B, E>(PhantomData<(B, E)>);

impl<B: JsonSchema, E: JsonSchema> JsonSchema for Over<B, E> {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        format!("Over_{}_{}", B::schema_name(), E::schema_name()).into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let base = generator.subschema_for::<B>();
        let extra = E::json_schema(generator);
        json_schema!({"allOf": [base, extra]})
    }
}

// ---- errors and responses

/// An error object.
#[derive(JsonSchema)]
#[schemars(rename = "ErrorObject", description = text::ERROR_OBJECT, transform = bare_fields)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct ErrorObjectShape {
    /// The JSON-RPC code.
    #[schemars(range(min = -9_007_199_254_740_991_i64, max = 9_007_199_254_740_991_i64))]
    code: i64,
    /// A short message.
    message: String,
    /// The stable code.
    data: ErrorData,
}

/// The `data` of an error: the stable code. The other members belong to single codes.
#[derive(JsonSchema)]
#[schemars(inline, transform = bare_entry)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct ErrorData {
    /// The kebab-case code. A reader accepts a code it does not know.
    #[schemars(extend("pattern" = KEBAB))]
    slopvac_code: String,
}

/// Fields shared by every per-document result.
#[derive(JsonSchema)]
#[schemars(rename = "DocumentResult", description = text::DOCUMENT_RESULT, transform = bare_fields)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct DocumentResultShape {
    /// Echo of a document id.
    #[schemars(length(min = 1))]
    document: String,
    /// Set when only this document failed.
    #[schemars(transform = absent)]
    error: Option<ErrorObjectShape>,
}

/// A response that carries an error.
#[derive(JsonSchema)]
#[schemars(
    rename = "ErrorResponse",
    description = text::ERROR_RESPONSE,
    transform = bare_fields,
    extend("not" = {"required": ["result"]})
)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct ErrorResponseShape {
    jsonrpc: JsonRpc,
    id: NullableId,
    error: ErrorObjectShape,
}

/// A response that carries a result.
#[derive(JsonSchema)]
#[schemars(
    rename = "SuccessResponse",
    description = text::SUCCESS_RESPONSE,
    transform = bare_fields,
    extend("not" = {"required": ["error"]})
)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct SuccessResponseShape {
    jsonrpc: JsonRpc,
    id: RequestId,
    result: Anything,
}

/// A success or an error response.
struct Response;

impl JsonSchema for Response {
    fn schema_name() -> Cow<'static, str> {
        "Response".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let error = generator.subschema_for::<ErrorResponseShape>();
        let success = generator.subschema_for::<SuccessResponseShape>();
        json_schema!({
            "description": text::RESPONSE,
            "if": {"required": ["error"]},
            "then": error,
            "else": success
        })
    }
}

// ---- hook results

/// The tags of one document.
#[derive(JsonSchema)]
#[schemars(inline, transform = bare_entry)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct TagEntry {
    #[schemars(transform = absent)]
    tags: Option<Vec<Tag>>,
}

/// The segments of one document.
#[derive(JsonSchema)]
#[schemars(inline, transform = bare_entry)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct SegmentEntry {
    #[schemars(transform = absent)]
    segments: Option<Vec<Segment>>,
}

/// The edits for one document.
#[derive(JsonSchema)]
#[schemars(inline, transform = bare_entry)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct EditEntry {
    #[schemars(transform = absent)]
    edits: Option<Vec<FixEdit>>,
}

/// The score of one document.
#[derive(JsonSchema)]
#[schemars(inline, transform = bare_entry)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct ScoreEntry {
    #[schemars(transform = absent, extend("minimum" = 0))]
    score: Option<f64>,
}

/// The findings that come with a document sent to a fixer or a scorer.
#[derive(JsonSchema)]
#[schemars(inline, transform = bare_entry)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct FindingsEntry {
    findings: Vec<Finding>,
}

/// The reply to `tagger/tag`.
#[derive(JsonSchema)]
#[schemars(rename = "TagResult", description = text::TAG_RESULT, transform = bare_fields)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct TagResultShape {
    results: Vec<Over<DocumentResultShape, TagEntry>>,
}

/// The reply to `extractor/extract`.
#[derive(JsonSchema)]
#[schemars(rename = "ExtractResult", description = text::EXTRACT_RESULT, transform = bare_fields)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct ExtractResultShape {
    results: Vec<Over<DocumentResultShape, SegmentEntry>>,
}

/// The reply to `fixer/fix`.
#[derive(JsonSchema)]
#[schemars(rename = "FixResult", description = text::FIX_RESULT, transform = bare_fields)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct FixResultShape {
    results: Vec<Over<DocumentResultShape, EditEntry>>,
}

/// The reply to `scorer/score`.
#[derive(JsonSchema)]
#[schemars(rename = "ScoreResult", description = text::SCORE_RESULT, transform = bare_fields)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct ScoreResultShape {
    results: Vec<Over<DocumentResultShape, ScoreEntry>>,
}

/// One answer of a provider: a text and a model, or an error.
#[derive(JsonSchema)]
#[schemars(
    inline,
    transform = bare_entry,
    extend("if" = {"not": {"required": ["error"]}}, "then" = {"required": ["text", "model"]})
)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct CompleteEntry {
    #[schemars(length(min = 1))]
    item: String,
    #[schemars(transform = absent)]
    text: Option<String>,
    #[schemars(length(min = 1), transform = absent)]
    model: Option<String>,
    #[schemars(transform = absent)]
    error: Option<ErrorObjectShape>,
}

/// The reply to `provider/complete`.
#[derive(JsonSchema)]
#[schemars(rename = "CompleteResult", description = text::COMPLETE_RESULT, transform = bare_fields)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct CompleteResultShape {
    results: Vec<CompleteEntry>,
}

/// The reply to `recipe/plan`.
#[derive(JsonSchema)]
#[schemars(rename = "RecipeResult", description = text::RECIPE_RESULT, transform = bare_fields)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct RecipeResultShape {
    files: Vec<RecipeFile>,
}

// ---- binary extractors

/// Format ids of binary documents.
#[derive(JsonSchema)]
#[schemars(
    rename = "BinaryFormat",
    description = text::BINARY_FORMAT,
    rename_all = "kebab-case",
    extend("x-slopvac-extensible" = true)
)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the variants and no code builds one"
)]
enum BinaryFormat {
    Docx,
    Docm,
    Dotx,
    Dotm,
    Pptx,
    Pptm,
    Ppsx,
    Ppsm,
    Potx,
    Potm,
    Xlsx,
    Xlsm,
    Xltx,
    Xltm,
    Odt,
    Odp,
    Ods,
    DrawioPng,
    ExcalidrawPng,
    PlantumlPng,
}

/// Budgets that an `extract-budget` error names.
#[derive(JsonSchema)]
#[schemars(
    rename = "BinaryBudget",
    description = text::BINARY_BUDGET,
    rename_all = "kebab-case",
    extend("x-slopvac-extensible" = true)
)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the variants and no code builds one"
)]
enum BinaryBudget {
    Segments,
    Part,
    Locator,
    Text,
    InFlight,
}

/// Why a binary extractor declines a document. The set is closed.
#[derive(JsonSchema)]
#[schemars(
    rename = "BinaryDeclineReason",
    description = text::BINARY_DECLINE_REASON,
    rename_all = "kebab-case"
)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the variants and no code builds one"
)]
enum BinaryDeclineReason {
    UnknownFormat,
    VariantUnsupported,
}

/// One binary document of a batch.
#[derive(JsonSchema)]
#[schemars(rename = "BinaryDocument", description = text::BINARY_DOCUMENT, transform = bare_fields)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct BinaryDocument {
    #[schemars(length(min = 1))]
    id: String,
    format: BinaryFormat,
    #[schemars(transform = absent)]
    media_type: Option<String>,
    #[schemars(range(max = MAX_SAFE))]
    byte_length: u64,
    #[schemars(extend("pattern" = BASE64))]
    bytes: String,
    #[schemars(
        transform = absent,
        extend("pattern" = crate::hooks::RELATIVE_PATH_PATTERN)
    )]
    path: Option<String>,
}

/// A byte range inside a binary document.
#[derive(JsonSchema)]
#[schemars(rename = "BinarySpan", description = text::BINARY_SPAN, transform = bare_fields)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct BinarySpan {
    #[schemars(range(max = MAX_SAFE))]
    start: u64,
    #[schemars(range(max = MAX_SAFE))]
    end: u64,
}

/// A region of prose that a binary extractor returns.
#[derive(JsonSchema)]
#[schemars(rename = "BinarySegment", description = text::BINARY_SEGMENT, transform = bare_fields)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct BinarySegment {
    #[schemars(extend("pattern" = PART))]
    part: String,
    #[schemars(extend("pattern" = PART))]
    locator: String,
    span: BinarySpan,
    #[schemars(length(min = 1))]
    text: String,
    context: ContextTag,
    #[schemars(extend("pattern" = SLUG))]
    role: String,
}

/// Content that a binary extractor skipped.
#[derive(JsonSchema)]
#[schemars(
    rename = "BinaryUnsupported",
    description = text::BINARY_UNSUPPORTED,
    transform = bare_fields
)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct BinaryUnsupported {
    #[schemars(extend("pattern" = PART))]
    part: String,
    #[schemars(transform = absent, extend("pattern" = PART))]
    locator: Option<String>,
    #[schemars(length(min = 1), transform = absent)]
    media_type: Option<String>,
}

/// The params of `binary-extractor/extract`.
#[derive(JsonSchema)]
#[schemars(
    rename = "BinaryExtractParams",
    description = text::BINARY_EXTRACT_PARAMS,
    transform = bare_fields
)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct BinaryExtractParams {
    extractor: HookIdSchema,
    documents: Batch<BinaryDocument>,
}

/// The result of one binary document: an outcome with segments, or an error. The rules that tie
/// the outcome, the segments and the error codes together are conditionals that a derive cannot
/// say.
struct BinaryEntry;

impl JsonSchema for BinaryEntry {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "BinaryEntry".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let base = generator.subschema_for::<DocumentResultShape>();
        let segment = generator.subschema_for::<BinarySegment>();
        let unsupported = generator.subschema_for::<BinaryUnsupported>();
        let budget = generator.subschema_for::<BinaryBudget>();
        let reason = generator.subschema_for::<BinaryDeclineReason>();
        let code = |name: &str| {
            json!({
                "properties": {"error": {
                    "properties": {"data": {
                        "properties": {"slopvac_code": {"const": name}},
                        "required": ["slopvac_code"]
                    }},
                    "required": ["data"]
                }},
                "required": ["error"]
            })
        };
        let outcome = |condition: Value| json!({"properties": {"outcome": condition}, "required": ["outcome"]});
        let detail = |members: Value, required: &[&str]| {
            json!({"properties": {"error": {"properties": {"data": {
                "properties": members,
                "required": required
            }}}}})
        };
        json_schema!({"allOf": [base, {
            "type": "object",
            "properties": {
                "outcome": {"enum": ["extracted", "empty", "no-source"]},
                "segments": {"type": "array", "items": segment},
                "unsupported": {"type": "array", "items": unsupported, "minItems": 1}
            },
            "oneOf": [
                {"required": ["outcome"], "not": {"required": ["error"]}},
                {
                    "required": ["error"],
                    "not": {"anyOf": [
                        {"required": ["outcome"]},
                        {"required": ["segments"]},
                        {"required": ["unsupported"]}
                    ]}
                }
            ],
            "allOf": [
                {
                    "if": outcome(json!({"const": "extracted"})),
                    "then": {"properties": {"segments": {"minItems": 1}}, "required": ["segments"]}
                },
                {
                    "if": outcome(json!({"enum": ["empty", "no-source"]})),
                    "then": {"properties": {"segments": {"maxItems": 0}}}
                },
                {
                    "if": outcome(json!({"const": "no-source"})),
                    "then": {"properties": {"unsupported": {"maxItems": 0}}}
                },
                {
                    "if": code(crate::error_codes::EXTRACT_BUDGET),
                    "then": detail(
                        json!({
                            "budget": budget,
                            "limit": {"type": "integer", "minimum": 0, "maximum": MAX_SAFE}
                        }),
                        &["budget", "limit"],
                    )
                },
                {
                    "if": code(crate::error_codes::BINARY_INPUT_DECLINED),
                    "then": detail(json!({"reason": reason}), &["reason"])
                },
                {
                    "properties": {"error": {"properties": {"data": {"properties": {
                        "slopvac_code": {"not": {"const": crate::error_codes::EXTRACT_UNSUPPORTED}}
                    }}}}}
                }
            ]
        }]})
    }
}

/// The reply to `binary-extractor/extract`.
#[derive(JsonSchema)]
#[schemars(
    rename = "BinaryExtractResult",
    description = text::BINARY_EXTRACT_RESULT,
    transform = bare_fields
)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct BinaryExtractResultShape {
    results: Vec<BinaryEntry>,
}

// ---- methods

/// Whether a method has an id and gets a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// It has an id and gets one reply.
    Request,
    /// It has no id and gets no reply.
    Notification,
}

/// A type that a method names, by definition name and by the call that defines it.
#[derive(Clone, Copy)]
pub(crate) struct TypeRef {
    /// The definition name.
    pub(crate) name: fn() -> Cow<'static, str>,
    /// Define the type in a generator and return the reference to it.
    pub(crate) define: fn(&mut SchemaGenerator) -> Schema,
}

impl TypeRef {
    /// A reference to the definition of `T`.
    pub(crate) const fn of<T: JsonSchema>() -> Self {
        Self {
            name: T::schema_name,
            define: define::<T>,
        }
    }
}

fn define<T: JsonSchema>(generator: &mut SchemaGenerator) -> Schema {
    generator.subschema_for::<T>()
}

/// One method of the protocol.
#[derive(Clone, Copy)]
pub(crate) struct Method {
    /// The method name on the wire.
    pub(crate) name: &'static str,
    /// Request or notification.
    pub(crate) kind: Kind,
    /// The params, when the method names any.
    pub(crate) params: Option<TypeRef>,
    /// The result of a request, when the schema describes it.
    pub(crate) result: Option<TypeRef>,
}

impl Method {
    const fn request(name: &'static str, params: Option<TypeRef>, result: Option<TypeRef>) -> Self {
        Self {
            name,
            kind: Kind::Request,
            params,
            result,
        }
    }

    const fn notification(name: &'static str, params: Option<TypeRef>) -> Self {
        Self {
            name,
            kind: Kind::Notification,
            params,
            result: None,
        }
    }

    /// The rule that a message with this method name must meet.
    fn case(&self, generator: &mut SchemaGenerator) -> Value {
        let mut properties = Map::new();
        let mut required = Vec::new();
        if self.kind == Kind::Request {
            properties.insert(
                "id".into(),
                generator.subschema_for::<RequestId>().to_value(),
            );
            required.push("id");
        }
        if let Some(params) = self.params {
            properties.insert("params".into(), (params.define)(generator).to_value());
            required.push("params");
        }
        let mut then = Map::new();
        if !properties.is_empty() {
            then.insert("properties".into(), Value::Object(properties));
            then.insert("required".into(), json!(required));
        }
        if self.kind == Kind::Notification {
            then.insert("not".into(), json!({"required": ["id"]}));
        }
        json!({
            "if": {"properties": {"method": {"const": self.name}}},
            "then": then
        })
    }
}

/// Every method of protocol v1, in the order of the document. The `Call` definition, the
/// definitions of the params and the definitions of the results all come from this table.
pub(crate) const METHODS: &[Method] = &[
    Method::request(
        method::INITIALIZE,
        Some(TypeRef::of::<InitializeParams<'static>>()),
        Some(TypeRef::of::<InitializeResult>()),
    ),
    Method::notification(method::INITIALIZED, None),
    Method::request(method::SHUTDOWN, None, None),
    Method::notification(method::EXIT, None),
    Method::notification(method::CANCEL_REQUEST, Some(TypeRef::of::<CancelParams>())),
    Method::request(
        method::TAGGER_TAG,
        Some(TypeRef::of::<TagParams<'static, Document>>()),
        Some(TypeRef::of::<TagResultShape>()),
    ),
    Method::request(
        method::EXTRACTOR_EXTRACT,
        Some(TypeRef::of::<ExtractParams<'static, Document>>()),
        Some(TypeRef::of::<ExtractResultShape>()),
    ),
    Method::request(
        method::REPORTER_RENDER,
        Some(TypeRef::of::<RenderParams<'static>>()),
        Some(TypeRef::of::<Rendered>()),
    ),
    Method::request(
        method::FIXER_FIX,
        Some(TypeRef::of::<FixParams<'static, Document>>()),
        Some(TypeRef::of::<FixResultShape>()),
    ),
    Method::request(
        method::SCORER_SCORE,
        Some(TypeRef::of::<ScoreParams<'static, Document>>()),
        Some(TypeRef::of::<ScoreResultShape>()),
    ),
    Method::request(
        method::PROVIDER_COMPLETE,
        Some(TypeRef::of::<CompleteParams<'static>>()),
        Some(TypeRef::of::<CompleteResultShape>()),
    ),
    Method::request(
        method::RECIPE_PLAN,
        Some(TypeRef::of::<RecipeParams<'static>>()),
        Some(TypeRef::of::<RecipeResultShape>()),
    ),
    Method::request(
        method::BINARY_EXTRACTOR_EXTRACT,
        Some(TypeRef::of::<BinaryExtractParams>()),
        Some(TypeRef::of::<BinaryExtractResultShape>()),
    ),
];

/// The names of the definitions that hold the results of the methods in the table. Nothing in a
/// message refers to them, because a response names no method, so the generated file keeps each
/// one on purpose.
#[must_use]
pub fn result_defs() -> Vec<String> {
    METHODS
        .iter()
        .filter_map(|method| method.result)
        .map(|result| (result.name)().into_owned())
        .collect()
}

/// A request or notification, dispatched on its method.
struct Call;

impl JsonSchema for Call {
    fn schema_name() -> Cow<'static, str> {
        "Call".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let cases: Vec<Value> = METHODS
            .iter()
            .map(|method| method.case(generator))
            .collect();
        json_schema!({
            "description": text::CALL,
            "type": "object",
            "properties": {"method": {"type": "string", "minLength": 1}},
            "required": ["method"],
            "allOf": cases
        })
    }
}

/// One message of plugin protocol v1: the root of the published schema.
///
/// The root checks the `jsonrpc` member, then dispatches a message with a `method` to its rule
/// and checks any other message as a response.
pub struct ProtocolMessage;

impl JsonSchema for ProtocolMessage {
    fn schema_name() -> Cow<'static, str> {
        "ProtocolMessage".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let call = generator.subschema_for::<Call>();
        let response = generator.subschema_for::<Response>();
        let version = generator.subschema_for::<JsonRpc>();
        for result in METHODS.iter().filter_map(|method| method.result) {
            (result.define)(generator);
        }
        generator.subschema_for::<PluginDeclaration>();
        json_schema!({
            "type": "object",
            "properties": {"jsonrpc": version},
            "required": ["jsonrpc"],
            "allOf": [{"if": {"required": ["method"]}, "then": call, "else": response}]
        })
    }
}
