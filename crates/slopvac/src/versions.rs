//! The schema and extension-API versions this build supports, for `slopvac --version --json`.
//!
//! This table is the one source of the `schema_versions` object. Each key is the name of a
//! schema file under `schemas/v3`, without the `.schema.json` suffix, and each value comes from
//! the crate that owns the version. `slopvac-schemas` reads the table and checks every key
//! against the generated artifact list (`docs/contract/policies.md`, section 3.2).

use std::collections::BTreeMap;

use slopvac_update::command::Supported;

/// The versions this build supports, taken from the crates that own them.
#[must_use]
pub fn supported() -> Supported {
    // A config-style file is checked by `slopvac-config`, which owns its version; styles share it.
    // Both owners declare an `i64`. The assertion below makes `unsigned_abs` exact for them.
    const _: () = assert!(
        slopvac_config::version::SUPPORTED > 0 && slopvac_plugins::lock::LOCK_SCHEMA_VERSION > 0,
        "schema versions are positive"
    );
    let config = slopvac_config::version::SUPPORTED.unsigned_abs();
    let lock = slopvac_plugins::lock::LOCK_SCHEMA_VERSION.unsigned_abs();
    // The artifacts that the binary writes at the shared `schema_version`: the lint report, the
    // rules output, the claims report, the review plan, the resolved-config document and
    // this one. Categories are read at the same version.
    let report = u64::from(slopvac_model::SchemaVersion::VALUE);
    Supported {
        schema_versions: BTreeMap::from([
            (
                "blocklist",
                vec![u64::from(slopvac_terms::BLOCKLIST_SCHEMA_VERSION)],
            ),
            ("category", vec![report]),
            ("claims", vec![report]),
            ("config", vec![config]),
            (
                "doctor-report",
                vec![u64::from(crate::doctor::SCHEMA_VERSION)],
            ),
            ("explain-config", vec![report]),
            (
                "genre",
                vec![u64::from(slopvac_context::GENRE_SCHEMA_VERSION)],
            ),
            ("lint-report", vec![report]),
            ("plugins-lock", vec![lock]),
            ("review-plan", vec![report]),
            ("rules-output", vec![report]),
            ("style", vec![config]),
            (
                "tagger",
                vec![u64::from(slopvac_context::TAGGER_SCHEMA_VERSION)],
            ),
            (
                "terms",
                vec![u64::from(slopvac_terms::TERMS_SCHEMA_VERSION)],
            ),
            ("version", vec![report]),
        ]),
        slopvac_api: slopvac_model::ApiVersion::SUPPORTED
            .iter()
            .map(|v| u64::from(*v))
            .collect(),
    }
}
