use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use similar::TextDiff;
use slopvac_model::schema_id;

use crate::artifacts::{ARTIFACTS, Artifact};
use crate::canonical::to_canonical_json;
use crate::error::{Error, Result};
use crate::link::{inline_defs, link_shared_defs};

/// Where the generated files live, relative to the repository root.
const SCHEMA_DIR: &str = "schemas/v3";
const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

/// One generated file.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Generated {
    /// File name inside `schemas/v3`.
    pub file: &'static str,
    /// The canonical JSON text.
    pub text: String,
}

/// A committed file that differs from the generated one.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Drift {
    /// File name inside `schemas/v3`.
    pub file: &'static str,
    /// A unified diff from the committed text to the generated text.
    pub diff: String,
}

/// Generate every artifact in [`ARTIFACTS`] in memory.
pub fn generate() -> Result<Vec<Generated>> {
    generate_from(ARTIFACTS)
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
    // Linking drops a definition that nothing in the file reaches, so hold these back.
    let kept: Vec<(String, Value)> = artifact
        .keep
        .iter()
        .filter_map(|name| {
            let body = schema.get("$defs")?.get(*name)?.clone();
            Some(((*name).to_owned(), body))
        })
        .collect();
    link_shared_defs(artifact.file, &mut schema, &imports)?;
    if let Some(object) = schema.as_object_mut().filter(|_| !kept.is_empty()) {
        let defs = object
            .entry("$defs")
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if let Some(defs) = defs.as_object_mut() {
            defs.extend(kept);
        }
    }
    Ok(schema)
}

/// Write every generated file under `root/schemas/v3`.
pub fn write(root: &Path) -> Result<Vec<Generated>> {
    let generated = generate()?;
    for file in &generated {
        let path = schema_path(root, file.file);
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
        let path = schema_path(root, file.file);
        let committed = fs::read_to_string(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        if committed != file.text {
            let name = format!("{SCHEMA_DIR}/{}", file.file);
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

fn schema_path(root: &Path, file: &str) -> PathBuf {
    root.join(SCHEMA_DIR).join(file)
}
