//! Helpers shared by the installed-binary tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde_json::Value;

static NEUTRAL: LazyLock<tempfile::TempDir> =
    LazyLock::new(|| tempfile::tempdir().expect("tempdir"));

/// A directory with no `slopvac.toml` on its path to the root. Cargo runs tests inside the
/// repository, whose own config would otherwise apply to every `--stdin` run.
pub(crate) fn neutral_dir() -> &'static Path {
    NEUTRAL.path()
}

/// A validator for the published `lint-report` schema.
#[allow(dead_code)] // not every test binary validates reports
pub(crate) fn report_validator() -> jsonschema::Validator {
    let schemas = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3");
    let read = |name: &str| -> Value {
        serde_json::from_str(&fs::read_to_string(schemas.join(name)).expect("schema file"))
            .expect("schema json")
    };
    let registry = jsonschema::Registry::new()
        .add(
            "https://srobroek.github.io/slopvac/schemas/v3/common.schema.json",
            read("common.schema.json"),
        )
        .expect("register common")
        .prepare()
        .expect("prepare registry");
    jsonschema::options()
        .with_registry(&registry)
        .build(&read("lint-report.schema.json"))
        .expect("compile the lint-report schema")
}
