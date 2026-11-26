use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use similar::TextDiff;
use slopvac_model::schema_id;

use crate::artifacts::{ARTIFACTS, Artifact};
use crate::canonical::to_canonical_json;
use crate::error::{Error, Result};
use crate::link::{inline_defs, link_shared_defs_keeping};

/// Where the generated files live, relative to the repository root.
const SCHEMA_DIR: &str = "schemas/v3";
/// Where the generated data files live, relative to the repository root.
const REFERENCE_DIR: &str = "docs/reference";
/// The section 10 code tables of the plugin protocol, generated from the host's code table.
const PROTOCOL_CODES_FILE: &str = "plugin-protocol-codes.json";
const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

/// One generated file.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Generated {
    /// The directory of the file, relative to the repository root.
    pub dir: &'static str,
    /// File name inside that directory.
    pub file: &'static str,
    /// The canonical JSON text.
    pub text: String,
}

/// A committed file that differs from the generated one.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Drift {
    /// File name. The diff header names its directory.
    pub file: &'static str,
    /// A unified diff from the committed text to the generated text.
    pub diff: String,
}

/// Generate every artifact in [`ARTIFACTS`] in memory, then the data file under
/// `docs/reference` that the docs site reads.
pub fn generate() -> Result<Vec<Generated>> {
    let mut files = generate_from(ARTIFACTS)?;
    files.push(Generated {
        dir: REFERENCE_DIR,
        file: PROTOCOL_CODES_FILE,
        text: protocol_codes_text()?,
    });
    Ok(files)
}

/// The host's section 10 tables as canonical JSON.
fn protocol_codes_text() -> Result<String> {
    let value = serde_json::to_value(slopvac_plugins::protocol_codes()).map_err(|source| {
        Error::Serialize {
            file: PROTOCOL_CODES_FILE,
            source,
        }
    })?;
    Ok(to_canonical_json(&value))
}

/// Generate the given artifacts in memory, in order. Each artifact may import only the files
/// listed before it.
pub fn generate_from(artifacts: &[Artifact]) -> Result<Vec<Generated>> {
    let mut done: Vec<(&'static str, Value)> = Vec::new();
    let mut publisher: BTreeMap<String, &'static str> = BTreeMap::new();
    for artifact in artifacts {
        let schema = build(artifact, artifacts, &done)?;
        for def in schema
            .get("$defs")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            if let Some(first) = publisher.insert(def.0.clone(), artifact.file) {
                return Err(Error::DuplicateDef {
                    def: def.0.clone(),
                    first,
                    second: artifact.file,
                });
            }
        }
        done.push((artifact.file, schema));
    }
    Ok(done
        .into_iter()
        .map(|(file, schema)| Generated {
            dir: SCHEMA_DIR,
            file,
            text: to_canonical_json(&schema),
        })
        .collect())
}

fn build(
    artifact: &Artifact,
    artifacts: &[Artifact],
    done: &[(&'static str, Value)],
) -> Result<Value> {
    let mut schema = (artifact.build)();
    let Some(object) = schema.as_object_mut() else {
        return Err(Error::NotAnObject {
            file: artifact.file,
        });
    };
    object.insert("$schema".into(), DIALECT.into());
    object.insert("$id".into(), schema_id(artifact.file).into());
    object.insert("title".into(), artifact.title.into());
    if let Some(description) = artifact.description {
        object.insert("description".into(), description.into());
    }
    let mut imports = Vec::new();
    for import in artifact.imports {
        let Some((file, schema)) = done.iter().find(|(file, _)| file == import) else {
            return Err(Error::UnknownImport {
                file: artifact.file,
                import,
            });
        };
        imports.push((*file, schema));
    }
    // Inline before linking: a shared definition is compared with the importer's copy, and
    // the published copy already has its owner's inlined definitions written in place.
    let mut inline: Vec<&str> = artifact.inline.to_vec();
    let local = |def: &str| schema.get("$defs").and_then(|defs| defs.get(def)).is_some();
    for import in artifact.imports {
        if let Some(owner) = artifacts.iter().find(|a| a.file == *import) {
            inline.extend(owner.inline.iter().filter(|def| local(def)));
        }
    }
    inline_defs(artifact.file, &mut schema, &inline)?;
    link_shared_defs_keeping(artifact.file, &mut schema, &imports, artifact.keep)?;
    Ok(schema)
}

/// Write every generated file under `root/schemas/v3`.
pub fn write(root: &Path) -> Result<Vec<Generated>> {
    let generated = generate()?;
    for file in &generated {
        let path = generated_path(root, file);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::write(&path, &file.text).map_err(|source| Error::Io { path, source })?;
    }
    Ok(generated)
}

/// Compare every generated file with the one committed under `root/schemas/v3`; the result
/// lists the files that differ. A committed file that is missing or cannot be read is an
/// error, not a difference: the comparison did not run.
pub fn check(root: &Path) -> Result<Vec<Drift>> {
    let mut drift = Vec::new();
    for file in generate()? {
        let path = generated_path(root, &file);
        let committed = fs::read_to_string(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        if committed != file.text {
            let name = format!("{}/{}", file.dir, file.file);
            let diff = TextDiff::from_lines(&committed, &file.text)
                .unified_diff()
                .context_radius(3)
                .header(&format!("a/{name}"), &format!("b/{name}"))
                .to_string();
            drift.push(Drift {
                file: file.file,
                diff,
            });
        }
    }
    Ok(drift)
}

fn generated_path(root: &Path, file: &Generated) -> PathBuf {
    root.join(file.dir).join(file.file)
}
