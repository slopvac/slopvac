//! Wire types for the lifecycle messages and the declaration.
//!
//! Shapes follow `schemas/v3/plugin-protocol.schema.json`. Unknown fields are
//! ignored in every message (plugin-protocol.md section 9).

use serde::{Deserialize, Serialize};

/// Highest protocol version the host speaks.
pub const PROTOCOL_VERSION: u32 = 1;
/// Lowest protocol version the host speaks.
pub const MIN_PROTOCOL_VERSION: u32 = 1;
/// The one registered v1 feature.
pub const FEATURE_DOCUMENT_PATH: &str = "document-path";

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
            Self::Tagger => "tagger/tag",
            Self::Extractor => "extractor/extract",
            Self::Reporter => "reporter/render",
            Self::Fixer => "fixer/fix",
            Self::Scorer => "scorer/score",
            Self::Provider => "provider/complete",
            Self::Recipe => "recipe/plan",
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

/// Features a plugin requires or would like.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureSet {
    /// Features that must be granted.
    pub required: Vec<String>,
    /// Features used when granted.
    pub optional: Vec<String>,
}

/// The pre-spawn declaration in the trusted manifest (section 4.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginDeclaration {
    /// Whether the plugin may open connections.
    pub network: bool,
    /// Every hook the plugin may offer.
    pub hooks: Vec<HookDecl>,
    /// Features it may require or request.
    pub features: FeatureSet,
}

/// Name and version pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NameVersion {
    /// Program name.
    pub name: String,
    /// Program version.
    pub version: String,
}

/// Capabilities in the plugin's `initialize` reply.
#[derive(Debug, Clone, Deserialize)]
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
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ReplyLimits {
    /// Body size limit, both directions.
    #[serde(rename = "maxMessageBytes")]
    pub max_message_bytes: u64,
    /// Documents per request.
    #[serde(rename = "maxBatchDocuments")]
    pub max_batch_documents: u64,
    /// Requests the plugin handles at once.
    #[serde(rename = "maxInFlight")]
    pub max_in_flight: u64,
}

/// The plugin's `initialize` result.
#[derive(Debug, Clone, Deserialize)]
pub struct InitializeResult {
    /// Picked version.
    #[serde(rename = "protocolVersion")]
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
