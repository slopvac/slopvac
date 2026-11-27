//! The requests and notifications the host sends, as typed values.
//!
//! Shapes follow `schemas/v3/plugin-protocol.schema.json`, and each params type has the name of
//! its `$def`. Every message is built from these types and serialized through [`encode`], so no
//! site builds JSON by hand.
//!
//! Object keys are written in alphabetical order, which is the order the host has always sent
//! them in. A plugin must not depend on it, but the bytes stay the same, so a field is declared
//! in that order and `tests/host_wire.rs` compares whole frames with committed goldens.
//! Optional members are absent, never null.

use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

use crate::error::{EncodeCause, PluginError};
use crate::hooks::CompleteItem;
use crate::host::MAX_REQUEST_ID;

/// The JSON-RPC version member: always the string `"2.0"`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct JsonRpc;

impl Serialize for JsonRpc {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str("2.0")
    }
}

impl JsonSchema for JsonRpc {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "JsonRpcVersion".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "const": "2.0" })
    }
}

/// A request id: an integer from 1 to 2^53-1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct RequestId(u64);

impl RequestId {
    /// The largest id, which the batch size probe uses.
    pub(crate) const MAX: Self = Self(MAX_REQUEST_ID);

    /// The id for `value`, or `None` outside 1 to 2^53-1.
    pub(crate) fn new(value: u64) -> Option<Self> {
        (1..=MAX_REQUEST_ID).contains(&value).then_some(Self(value))
    }

    /// The number.
    pub(crate) fn get(self) -> u64 {
        self.0
    }
}

impl JsonSchema for RequestId {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "RequestId".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "integer", "minimum": 1, "maximum": MAX_REQUEST_ID })
    }
}

/// A request: it has an id and gets one reply.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct Request<'a, P> {
    /// The request id.
    pub(crate) id: RequestId,
    /// Always `"2.0"`.
    pub(crate) jsonrpc: JsonRpc,
    /// The method name.
    pub(crate) method: &'a str,
    /// The parameters, absent for a method that takes none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) params: Option<P>,
}

impl<'a, P> Request<'a, P> {
    /// A request for `method`.
    pub(crate) fn new(id: RequestId, method: &'a str, params: Option<P>) -> Self {
        Self {
            id,
            jsonrpc: JsonRpc,
            method,
            params,
        }
    }
}

/// A notification: it has no id and gets no reply.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct Notification<'a, P> {
    /// Always `"2.0"`.
    pub(crate) jsonrpc: JsonRpc,
    /// The method name.
    pub(crate) method: &'a str,
    /// The parameters, absent for a method that takes none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) params: Option<P>,
}

impl<'a, P> Notification<'a, P> {
    /// A notification for `method`.
    pub(crate) fn new(method: &'a str, params: Option<P>) -> Self {
        Self {
            jsonrpc: JsonRpc,
            method,
            params,
        }
    }
}

/// The name and version the host gives for itself.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct HostInfo<'a> {
    /// Program name.
    pub(crate) name: &'a str,
    /// Program version.
    pub(crate) version: &'a str,
}

/// What the host offers in `initialize`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct HostCapabilities<'a> {
    /// Features the host grants.
    pub(crate) features: &'a [String],
    /// Hook kinds the host can call.
    pub(crate) hooks: Vec<&'static str>,
}

/// The host's ceilings in `initialize`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct HostLimits {
    /// Documents per request.
    #[serde(rename = "maxBatchDocuments")]
    pub(crate) max_batch_documents: usize,
    /// Body size limit, both directions.
    #[serde(rename = "maxMessageBytes")]
    pub(crate) max_message_bytes: usize,
}

/// The first request, from host to plugin.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct InitializeParams<'a> {
    /// Hook kinds and features the host offers.
    pub(crate) capabilities: HostCapabilities<'a>,
    /// Who the host is.
    #[serde(rename = "hostInfo")]
    pub(crate) host_info: HostInfo<'a>,
    /// The host's ceilings.
    pub(crate) limits: HostLimits,
    /// Lowest protocol version the host speaks.
    #[serde(rename = "minProtocolVersion")]
    pub(crate) min_protocol_version: u32,
    /// Whether offline mode is on.
    pub(crate) offline: bool,
    /// Highest protocol version the host speaks.
    #[serde(rename = "protocolVersion")]
    pub(crate) protocol_version: u32,
}

/// The `initialized` notification carries an empty object.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct InitializedParams {}

/// `$/cancelRequest`: the request the host no longer wants answered.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct CancelParams {
    /// The request to cancel.
    pub(crate) id: RequestId,
}

/// Define the params of one documents hook: the batch, and the hook id under the key that
/// names the kind.
macro_rules! documents_params {
    ($(#[$meta:meta])* $name:ident, $key:literal) => {
        $(#[$meta])*
        #[derive(Debug, Serialize, JsonSchema)]
        pub(crate) struct $name<'a, D> {
            /// The batch.
            pub(crate) documents: &'a [D],
            /// The hook id.
            #[serde(rename = $key)]
            pub(crate) hook: &'a str,
        }
    };
}

documents_params!(
    /// `tagger/tag`.
    TagParams,
    "tagger"
);
documents_params!(
    /// `extractor/extract`.
    ExtractParams,
    "extractor"
);
documents_params!(
    /// `fixer/fix`.
    FixParams,
    "fixer"
);
documents_params!(
    /// `scorer/score`.
    ScoreParams,
    "scorer"
);

/// `reporter/render`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct RenderParams<'a> {
    /// Reporter settings from the config.
    pub(crate) options: &'a Value,
    /// The lint report.
    pub(crate) report: &'a Value,
    /// The hook id.
    pub(crate) reporter: &'a str,
}

/// `provider/complete`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct CompleteParams<'a> {
    /// What to ask about.
    pub(crate) items: &'a [CompleteItem],
    /// The model name from the config.
    pub(crate) model: &'a str,
    /// The hook id.
    pub(crate) provider: &'a str,
    /// The review type.
    pub(crate) task: &'a str,
}

/// `recipe/plan`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct RecipeParams<'a> {
    /// The agent harness to set up.
    pub(crate) harness: &'a str,
    /// The flags of the setup command.
    pub(crate) options: &'a Value,
    /// The hook id.
    pub(crate) recipe: &'a str,
}

/// Serialize one message to the bytes of a frame body.
///
/// # Errors
/// [`PluginError::Encode`] when the message cannot be encoded. Its source keeps the category and
/// the position of the failure, never the encoder's text.
pub(crate) fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, PluginError> {
    serde_json::to_vec(msg).map_err(|e| PluginError::Encode(EncodeCause::from(&e)))
}

/// The size of the request for `method` with the largest id, which no real request exceeds.
///
/// Pass params with an empty batch: the result is the room that a request needs besides its
/// documents.
///
/// # Errors
/// [`PluginError::Encode`] when the request cannot be encoded.
pub(crate) fn request_len<P: Serialize>(method: &str, params: &P) -> Result<usize, PluginError> {
    encode(&Request::new(RequestId::MAX, method, Some(params))).map(|bytes| bytes.len())
}
#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::panic,
        reason = "test code: a panic is a failed test"
    )]

    use std::fs;
    use std::path::PathBuf;

    use rstest::rstest;
    use serde::Serializer;
    use serde::ser::Error as _;
    use serde_json::json;

    use super::*;
    use crate::hooks::{Document, Finding, FindingsDocument, Span};
    use crate::protocol::{HookKind, method};

    #[rstest]
    #[case::zero(0, false)]
    #[case::first(1, true)]
    #[case::largest(9_007_199_254_740_991, true)]
    #[case::past_largest(9_007_199_254_740_992, false)]
    fn a_request_id_is_an_integer_from_1_to_2_to_the_53_minus_1(
        #[case] n: u64,
        #[case] valid: bool,
    ) {
        assert_eq!(RequestId::new(n).map(RequestId::get), valid.then_some(n));
    }

    #[test]
    fn a_request_without_params_has_no_params_member() {
        let id = RequestId::new(7).unwrap();
        let bytes = encode(&Request::<()>::new(id, "shutdown", None)).unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            r#"{"id":7,"jsonrpc":"2.0","method":"shutdown"}"#
        );
    }

    /// Params that cannot be encoded.
    struct Unencodable;

    impl Serialize for Unencodable {
        fn serialize<S: Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(S::Error::custom("no way"))
        }
    }

    #[test]
    fn a_failed_encode_is_an_error_not_a_zero_size() {
        let err = request_len("m", &Unencodable).unwrap_err();
        let PluginError::Encode(cause) = &err else {
            panic!("expected an encode error, got {err:?}");
        };
        assert_eq!(cause.category, serde_json::error::Category::Data);
        assert_eq!(err.slopvac_code(), "plugin-usage");
        // The chain names the kind and the position, not the encoder's own text.
        let source = std::error::Error::source(&err).unwrap();
        let chain = format!("{err}|{source}");
        assert!(!chain.contains("no way"), "{chain}");
        assert!(chain.contains("Data error at line 0, column 0"), "{chain}");
    }

    #[test]
    fn the_size_probe_counts_the_largest_id() {
        let params = RecipeParams {
            harness: "h",
            options: &json!({}),
            recipe: "r",
        };
        let want = r#"{"id":9007199254740991,"jsonrpc":"2.0","method":"recipe/plan","params":{"harness":"h","options":{},"recipe":"r"}}"#;
        assert_eq!(request_len("recipe/plan", &params), Ok(want.len()));
    }

    const BASE: &str = "https://slopvac.github.io/slopvac/schemas/v3";

    fn schemas() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3")
    }

    /// A validator for a part of the published protocol schema: `reference` is `""` for the root
    /// or `"#/$defs/<Def>"`. Every published schema is registered, so the `$ref`s between them
    /// resolve.
    fn published(reference: &str) -> jsonschema::Validator {
        let mut registry = jsonschema::Registry::new();
        for entry in fs::read_dir(schemas()).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "json") {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                let schema: Value =
                    serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
                registry = registry.add(format!("{BASE}/{name}"), schema).unwrap();
            }
        }
        let registry = registry.prepare().unwrap();
        jsonschema::options()
            .with_registry(&registry)
            .build(&json!({"$ref": format!("{BASE}/plugin-protocol.schema.json{reference}")}))
            .unwrap()
    }

    /// Bridge until the published schema is generated from these types: each type's own schema
    /// accepts an instance of it, and so does the matching `$def` of the hand-written file.
    /// The generator replaces this test.
    fn agree<T: JsonSchema + Serialize>(def: &str, instance: &T) {
        let value = serde_json::to_value(instance).unwrap();
        let generated = serde_json::to_value(schemars::schema_for!(T)).unwrap();
        let own = jsonschema::validator_for(&generated).unwrap();
        assert!(own.is_valid(&value), "{def}: own schema rejects {value}");
        let file = published(&format!("#/$defs/{def}"));
        assert!(
            file.is_valid(&value),
            "{def}: published $def rejects {value}"
        );
    }

    /// Like [`agree`], for a whole message: the type's own schema, and the root of the
    /// published schema, which dispatches on the method.
    fn agree_message<T: JsonSchema + Serialize>(what: &str, message: &T) {
        let value = serde_json::to_value(message).unwrap();
        let generated = serde_json::to_value(schemars::schema_for!(T)).unwrap();
        let own = jsonschema::validator_for(&generated).unwrap();
        assert!(own.is_valid(&value), "{what}: own schema rejects {value}");
        assert!(
            published("").is_valid(&value),
            "{what}: the published root rejects {value}"
        );
    }

    #[test]
    fn whole_messages_satisfy_the_published_root_schema() {
        let features = ["document-path".to_owned()];
        let id = RequestId::new(1).unwrap();
        let initialize = InitializeParams {
            capabilities: HostCapabilities {
                features: &features,
                hooks: vec!["tagger"],
            },
            host_info: HostInfo {
                name: "slopvac",
                version: "3.0.0",
            },
            limits: HostLimits {
                max_batch_documents: 64,
                max_message_bytes: 1 << 20,
            },
            min_protocol_version: 1,
            offline: false,
            protocol_version: 1,
        };
        agree_message(
            "initialize",
            &Request::new(id, "initialize", Some(initialize)),
        );
        agree_message(
            "initialized",
            &Notification::new("initialized", Some(InitializedParams {})),
        );
        agree_message(
            "cancel",
            &Notification::new("$/cancelRequest", Some(CancelParams { id })),
        );
        let docs = [doc()];
        agree_message(
            "tagger/tag",
            &Request::new(
                id,
                "tagger/tag",
                Some(TagParams {
                    documents: &docs,
                    hook: "main",
                }),
            ),
        );
    }

    fn to<T: Serialize>(message: &T) -> Value {
        serde_json::to_value(message).unwrap()
    }

    /// The four message shapes of the bridge, as values, with the method each must carry.
    fn bridge_messages() -> Vec<(&'static str, Value, &'static str)> {
        let features = ["document-path".to_owned()];
        let id = RequestId::new(1).unwrap();
        let initialize = InitializeParams {
            capabilities: HostCapabilities {
                features: &features,
                hooks: vec!["tagger"],
            },
            host_info: HostInfo {
                name: "slopvac",
                version: "3.0.0",
            },
            limits: HostLimits {
                max_batch_documents: 64,
                max_message_bytes: 1 << 20,
            },
            min_protocol_version: 1,
            offline: false,
            protocol_version: 1,
        };
        let docs = [doc()];
        vec![
            (
                "initialize",
                to(&Request::new(id, method::INITIALIZE, Some(initialize))),
                method::INITIALIZE,
            ),
            (
                "initialized",
                to(&Notification::new(
                    method::INITIALIZED,
                    Some(InitializedParams {}),
                )),
                method::INITIALIZED,
            ),
            (
                "cancel",
                to(&Notification::new(
                    method::CANCEL_REQUEST,
                    Some(CancelParams { id }),
                )),
                method::CANCEL_REQUEST,
            ),
            (
                "tagger/tag",
                to(&Request::new(
                    id,
                    HookKind::Tagger.method(),
                    Some(TagParams {
                        documents: &docs,
                        hook: "main",
                    }),
                )),
                HookKind::Tagger.method(),
            ),
        ]
    }

    #[test]
    fn each_message_carries_the_method_name_of_the_protocol() {
        // The expected names are written out here, apart from the constants under test.
        let want = ["initialize", "initialized", "$/cancelRequest", "tagger/tag"];
        for ((what, value, name), want) in bridge_messages().into_iter().zip(want) {
            assert_eq!(value["method"], want, "{what}");
            assert_eq!(name, want, "{what}");
        }
    }

    /// For each branch of the published root, a message that breaks the branch's own rule is
    /// rejected. The same message under a mistyped method is accepted, because an unknown
    /// method is not a schema error. So the rejection proves that the exact method name
    /// selected the branch.
    #[test]
    fn a_mistyped_method_leaves_the_branch_of_the_published_root() {
        let root = published("");
        for (what, value, _) in bridge_messages() {
            let mut broken = value.clone();
            let object = broken.as_object_mut().unwrap();
            match what {
                "initialize" | "tagger/tag" => {
                    object.remove("params");
                }
                _ => {
                    object.insert("id".into(), json!(1));
                }
            }
            assert!(root.is_valid(&value), "{what}: the valid message");
            assert!(
                !root.is_valid(&broken),
                "{what}: the branch rejects {broken}"
            );
            let method = broken["method"].as_str().unwrap().to_owned();
            broken["method"] = json!(format!("{method}x"));
            assert!(
                root.is_valid(&broken),
                "{what}: a mistyped method is outside the branch"
            );
        }
    }

    fn doc() -> Document {
        Document {
            id: "d0".into(),
            media_type: Some("text/markdown".into()),
            path: Some("docs/a.md".into()),
            text: "text".into(),
        }
    }

    fn findings_doc() -> FindingsDocument {
        FindingsDocument {
            findings: vec![Finding {
                message: "m".into(),
                rule: "core:x".into(),
                severity: "warning".into(),
                span: Span { end: 2, start: 0 },
            }],
            document: doc(),
        }
    }

    #[test]
    fn the_initialization_and_cancel_params_satisfy_the_published_schema() {
        let features = ["document-path".to_owned()];
        agree(
            "InitializeParams",
            &InitializeParams {
                capabilities: HostCapabilities {
                    features: &features,
                    hooks: vec!["tagger", "recipe"],
                },
                host_info: HostInfo {
                    name: "slopvac",
                    version: "3.0.0",
                },
                limits: HostLimits {
                    max_batch_documents: 64,
                    max_message_bytes: 1 << 20,
                },
                min_protocol_version: 1,
                offline: false,
                protocol_version: 1,
            },
        );
        agree(
            "CancelParams",
            &CancelParams {
                id: RequestId::new(3).unwrap(),
            },
        );
        agree("RequestId", &RequestId::new(3).unwrap());
    }

    #[test]
    fn the_hook_params_satisfy_the_published_schema() {
        let docs = [doc()];
        let findings = [findings_doc()];
        agree(
            "TagParams",
            &TagParams {
                documents: &docs,
                hook: "main",
            },
        );
        agree(
            "ExtractParams",
            &ExtractParams {
                documents: &docs,
                hook: "main",
            },
        );
        agree(
            "FixParams",
            &FixParams {
                documents: &findings,
                hook: "main",
            },
        );
        agree(
            "ScoreParams",
            &ScoreParams {
                documents: &findings,
                hook: "main",
            },
        );
        let report = json!({"schema_version": 3});
        let options = json!({"width": 80});
        agree(
            "RenderParams",
            &RenderParams {
                options: &options,
                report: &report,
                reporter: "main",
            },
        );
        let items = [CompleteItem {
            id: "i1".into(),
            instruction: Some("shorter".into()),
            text: "abc".into(),
        }];
        agree(
            "CompleteParams",
            &CompleteParams {
                items: &items,
                model: "m1",
                provider: "main",
                task: "rewrite",
            },
        );
        agree(
            "RecipeParams",
            &RecipeParams {
                harness: "claude",
                options: &options,
                recipe: "main",
            },
        );
    }
}
