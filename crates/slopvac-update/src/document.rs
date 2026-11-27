//! The `slopvac --version --json` document, as one type.
//!
//! The binary writes it with [`Document::to_line`], and the update lock reads another binary's
//! document back with the same type (`probe`). The schema in
//! `schemas/v3/version.schema.json` is generated from this type, so the reader and the writer
//! cannot drift from it or from each other.
//!
//! A reader must cope with a binary of another age. The members are therefore read with a
//! default, and an unknown member is ignored. The schema still lists every member as written.
//! Fields are declared in alphabetical order, which is the order of the documents that earlier
//! builds printed: the bytes of a document for the same inputs did not change.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use slopvac_model::x_extensions;

use crate::command::Supported;
use crate::notice::Notice;

/// The document that `slopvac --version --json` prints: the binary version and how it was
/// installed, each supported `schema_version` and `slopvac_api`, and any update notice. Unknown
/// keys are errors, and keys that start with `x-` are ignored. Semantics are specified in
/// docs/contract/policies.md, section 3.2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = x_extensions)]
#[non_exhaustive]
pub struct Document {
    /// How this binary was installed, such as `installer`, `homebrew` or `unknown`.
    #[serde(default)]
    pub install_method: String,
    /// Always `slopvac`.
    #[schemars(extend("const" = "slopvac"))]
    pub name: String,
    /// The update notice, when a newer release is known. Empty otherwise.
    #[serde(default)]
    pub notices: Vec<Notice>,
    /// The `schema_version` numbers the binary reads or writes for each versioned artifact. A
    /// key is the schema file name without `.schema.json`.
    #[serde(default)]
    pub schema_versions: BTreeMap<String, Vec<u64>>,
    /// The extension API numbers the binary supports.
    #[serde(default)]
    pub slopvac_api: Vec<u64>,
    /// The binary's semantic version.
    pub version: String,
}

impl Document {
    /// The document for a binary at `version`, installed as `install_method`, that supports
    /// `supported` and has `notices` to show.
    #[must_use]
    pub fn new(
        version: &str,
        install_method: &str,
        supported: &Supported,
        notices: Vec<Notice>,
    ) -> Self {
        Self {
            install_method: install_method.to_owned(),
            name: "slopvac".to_owned(),
            notices,
            schema_versions: supported
                .schema_versions
                .iter()
                .map(|(artifact, versions)| ((*artifact).to_owned(), versions.clone()))
                .collect(),
            slopvac_api: supported.slopvac_api.clone(),
            version: version.to_owned(),
        }
    }

    /// The document as one line of compact JSON that ends with a newline.
    #[must_use]
    pub fn to_line(&self) -> String {
        #[expect(
            clippy::expect_used,
            reason = "the document holds strings, integers and string-keyed maps, which always serialize"
        )]
        let mut text = serde_json::to_string(self).expect("a version document serializes");
        text.push('\n');
        text
    }
}
