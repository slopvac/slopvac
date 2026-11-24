//! Schema-only helpers for the doctor report.
//!
//! The report's closed string sets are `&'static str` fields, and its paths and messages are
//! plain strings, so the derive cannot say what the published `doctor-report.schema.json` says
//! about them. Each helper here writes that shape and changes nothing about the JSON the report
//! serializes.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};

/// A path made safe to print. Shared as `$defs/Path`.
pub(super) struct PathText;

impl JsonSchema for PathText {
    fn schema_name() -> Cow<'static, str> {
        "Path".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A path made safe to print: relative to the working directory (`.` for the directory itself), or `<external>/<file name>` outside it.",
            "type": "string",
            "minLength": 1
        })
    }
}

/// A human message with no absolute path. Shared as `$defs/Message`.
pub(super) struct MessageText;

impl JsonSchema for MessageText {
    fn schema_name() -> Cow<'static, str> {
        "Message".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A human message with no absolute path. Not an interface: branch on the status fields.",
            "type": "string",
            "minLength": 1
        })
    }
}

/// A message, or `null` when there is nothing to say.
pub(super) fn nullable_message(generator: &mut SchemaGenerator) -> Schema {
    let message = generator.subschema_for::<MessageText>();
    json_schema!({ "oneOf": [message, { "type": "null" }] })
}

/// `schema_version`: the shared definition, so the report and every other artifact agree.
pub(super) fn schema_version(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "$ref": "common.schema.json#/$defs/SchemaVersion" })
}

/// How the binary was installed. The set grows, so a consumer must tolerate a new value.
pub(super) fn install_method(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "enum": ["installer", "uv-tool", "uvx", "pip", "uv-pip", "homebrew", "mise", "cargo", "unknown"],
        "x-slopvac-extensible": true
    })
}

/// Whether update checks run.
pub(super) fn update_checks(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "enum": ["enabled", "disabled"] })
}

/// Why update checks are off, or `null` when they run.
pub(super) fn update_disabled_by(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "enum": [null, "offline", "ci", "env", "config"],
        "x-slopvac-extensible": true
    })
}

/// What the cached update check says about the running version.
pub(super) fn update_state(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "enum": ["available", "current", "unknown"] })
}

/// The sources that turned offline mode on, each listed once.
pub(super) fn offline_sources(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "array",
        "uniqueItems": true,
        "items": { "enum": ["flag", "env", "config"], "x-slopvac-extensible": true }
    })
}

/// The layer a config file came from.
pub(super) fn config_layer(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "enum": ["explicit", "project"] })
}

/// What reading the lockfile showed.
pub(super) fn lockfile_status(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "enum": ["absent", "ok", "invalid", "unreadable"] })
}

/// What a locked plugin is.
pub(super) fn plugin_kind(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "enum": ["data", "code"] })
}

/// How a locked plugin stands against the store. The set grows.
pub(super) fn plugin_state(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "enum": ["recorded", "in-place", "changed", "tampered", "not-installed"],
        "x-slopvac-extensible": true
    })
}

/// Mark a closed set as open to new values, so a consumer tolerates a value it does not know.
pub(super) fn extensible(schema: &mut Schema) {
    schema.insert("x-slopvac-extensible".into(), true.into());
}

/// What reading the trust store showed.
pub(super) fn trust_status(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "enum": ["absent", "ok", "unreadable"] })
}
