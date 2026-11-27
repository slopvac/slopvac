//! Wire types for the lifecycle messages and the declaration.
//!
//! Shapes follow `schemas/v3/plugin-protocol.schema.json`. Unknown fields are
//! ignored in every message (plugin-protocol.md section 9).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod requests;
pub(crate) mod schema;
pub(crate) mod text;

use schema::{
    FeatureNameSchema, ProtocolVersion, UniqueItems, bare_entry, bare_fields, network_note,
};
pub use schema::ProtocolMessage;

pub(crate) use requests::{
    CancelParams, CompleteParams, ExtractParams, FixParams, HostCapabilities, HostInfo, HostLimits,
    InitializeParams, InitializedParams, Notification, RecipeParams, RenderParams, Request,
    RequestId, ScoreParams, TagParams, encode, request_len,
};

/// Highest protocol version the host speaks.
pub const PROTOCOL_VERSION: u32 = 1;
/// Lowest protocol version the host speaks.
pub const MIN_PROTOCOL_VERSION: u32 = 1;
/// The one registered v1 feature.
pub const FEATURE_DOCUMENT_PATH: &str = "document-path";

/// The lifecycle method names the host sends. Hook methods come from [`HookKind::method`].
pub(crate) mod method {
    /// The first request.
    pub(crate) const INITIALIZE: &str = "initialize";
    /// The notification that ends the handshake.
    pub(crate) const INITIALIZED: &str = "initialized";
    /// The request that starts the shutdown.
    pub(crate) const SHUTDOWN: &str = "shutdown";
    /// The notification that tells the plugin to leave.
    pub(crate) const EXIT: &str = "exit";
    /// The notification that withdraws a request.
    pub(crate) const CANCEL_REQUEST: &str = "$/cancelRequest";
    /// The tagger hook.
    pub(crate) const TAGGER_TAG: &str = "tagger/tag";
    /// The extractor hook.
    pub(crate) const EXTRACTOR_EXTRACT: &str = "extractor/extract";
    /// The reporter hook.
    pub(crate) const REPORTER_RENDER: &str = "reporter/render";
    /// The fixer hook.
    pub(crate) const FIXER_FIX: &str = "fixer/fix";
    /// The scorer hook.
    pub(crate) const SCORER_SCORE: &str = "scorer/score";
    /// The provider hook.
    pub(crate) const PROVIDER_COMPLETE: &str = "provider/complete";
    /// The recipe hook.
    pub(crate) const RECIPE_PLAN: &str = "recipe/plan";
    /// The binary extractor hook. It has no [`HookKind`](super::HookKind) yet.
    pub(crate) const BINARY_EXTRACTOR_EXTRACT: &str = "binary-extractor/extract";
}

/// A hook kind the host knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookKind {
    /// Context tags.
    Tagger,
    /// Split a format into prose segments.
    Extractor,
    /// Render a report as text.
    Reporter,
    /// Propose edits.
    Fixer,
    /// Compute scores.
    Scorer,
    /// Ask a model.
    Provider,
    /// Plan agent-harness files.
    Recipe,
}

impl HookKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 7] = [
        Self::Tagger,
        Self::Extractor,
        Self::Reporter,
        Self::Fixer,
        Self::Scorer,
        Self::Provider,
        Self::Recipe,
    ];

    /// The kind name on the wire.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tagger => "tagger",
            Self::Extractor => "extractor",
            Self::Reporter => "reporter",
            Self::Fixer => "fixer",
            Self::Scorer => "scorer",
            Self::Provider => "provider",
            Self::Recipe => "recipe",
        }
    }

    /// The JSON-RPC method for this kind.
    #[must_use]
    pub fn method(self) -> &'static str {
        match self {
            Self::Tagger => method::TAGGER_TAG,
            Self::Extractor => method::EXTRACTOR_EXTRACT,
            Self::Reporter => method::REPORTER_RENDER,
            Self::Fixer => method::FIXER_FIX,
            Self::Scorer => method::SCORER_SCORE,
            Self::Provider => method::PROVIDER_COMPLETE,
            Self::Recipe => method::RECIPE_PLAN,
        }
    }

    /// Parse a wire name. Unknown kinds give `None` and are ignored by callers.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// One hook a plugin offers or may offer.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HookDecl {
    /// Hook kind name.
    pub kind: String,
    /// Plugin-local hook id.
    pub id: String,
}

impl JsonSchema for HookDecl {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "HookDecl".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schema::hook_decl(generator)
    }
}

/// Features a plugin requires or would like.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline, transform = bare_entry)]
pub struct FeatureSet {
    /// Features that must be granted.
    #[schemars(with = "UniqueItems<FeatureNameSchema>")]
    pub required: Vec<String>,
    /// Features used when granted.
    #[schemars(with = "UniqueItems<FeatureNameSchema>")]
    pub optional: Vec<String>,
}

/// The pre-spawn declaration in the trusted manifest (section 4.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(description = text::PLUGIN_DECLARATION, transform = bare_fields)]
pub struct PluginDeclaration {
    /// Whether the plugin may open connections.
    pub network: bool,
    /// Every hook the plugin may offer.
    pub hooks: Vec<HookDecl>,
    /// Features it may require or request.
    pub features: FeatureSet,
}

/// Name and version pair.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "Name", description = text::NAME, transform = bare_fields)]
pub struct NameVersion {
    /// Program name.
    #[schemars(length(min = 1))]
    pub name: String,
    /// Program version.
    #[schemars(length(min = 1))]
    pub version: String,
}

/// Capabilities in the plugin's `initialize` reply.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(inline, transform = bare_entry, transform = network_note)]
pub struct ReplyCapabilities {
    /// Hooks offered.
    pub hooks: Vec<HookDecl>,
    /// Features wanted.
    pub features: FeatureSet,
    /// Copy of the manifest network flag.
    #[serde(rename = "requiresNetwork")]
    pub requires_network: bool,
}

/// Limits in the plugin's `initialize` reply.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[schemars(inline, transform = bare_entry)]
pub struct ReplyLimits {
    /// Body size limit, both directions.
    #[serde(rename = "maxMessageBytes")]
    #[schemars(range(min = 65536))]
    pub max_message_bytes: u64,
    /// Documents per request.
    #[serde(rename = "maxBatchDocuments")]
    #[schemars(range(min = 1))]
    pub max_batch_documents: u64,
    /// Requests the plugin handles at once.
    #[serde(rename = "maxInFlight")]
    #[schemars(range(min = 1))]
    pub max_in_flight: u64,
}

/// The plugin's `initialize` result.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(description = text::INITIALIZE_RESULT, transform = bare_fields)]
pub struct InitializeResult {
    /// Picked version.
    #[serde(rename = "protocolVersion")]
    #[schemars(with = "ProtocolVersion")]
    pub protocol_version: u32,
    /// Who the plugin says it is.
    #[serde(rename = "pluginInfo")]
    pub plugin_info: NameVersion,
    /// Offered hooks and features.
    pub capabilities: ReplyCapabilities,
    /// Negotiated limits.
    pub limits: ReplyLimits,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_kind_round_trips_through_its_name() {
        for k in HookKind::ALL {
            assert_eq!(HookKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(HookKind::parse("telepath"), None);
    }

    #[test]
    fn initialize_result_ignores_unknown_fields_and_requires_known_ones() {
        let ok = serde_json::json!({
            "protocolVersion": 1, "pluginInfo": {"name": "p", "version": "1", "x": 1},
            "capabilities": {"hooks": [], "features": {"required": [], "optional": []},
                             "requiresNetwork": false, "x-extra": true},
            "limits": {"maxMessageBytes": 65536, "maxBatchDocuments": 1, "maxInFlight": 1},
            "future": 1
        });
        assert!(serde_json::from_value::<InitializeResult>(ok.clone()).is_ok());
        let mut missing = ok;
        missing["capabilities"]
            .as_object_mut()
            .unwrap()
            .remove("requiresNetwork");
        assert!(serde_json::from_value::<InitializeResult>(missing).is_err());
    }
}
