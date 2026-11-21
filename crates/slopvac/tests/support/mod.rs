//! Helpers shared by the installed-binary tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde_json::Value;

static NEUTRAL: LazyLock<PathBuf> = LazyLock::new(|| {
    // A static is never dropped, so a `TempDir` here would stay behind after every test
    // process. One fixed empty directory, shared by all of them, leaves a single entry.
    let dir = std::env::temp_dir().join("slopvac-test-neutral-cwd");
    fs::create_dir_all(&dir).expect("create the neutral directory");
    dir
});

/// A directory with no `slopvac.toml` on its path to the root. Cargo runs tests inside the
/// repository, whose own config would otherwise apply to every `--stdin` run.
pub(crate) fn neutral_dir() -> &'static Path {
    NEUTRAL.as_path()
}

/// A validator for the published `lint-report` schema.
#[allow(dead_code, reason = "not every test binary validates reports")]
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
