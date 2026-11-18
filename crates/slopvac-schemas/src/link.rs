use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use crate::error::{Error, Result};

const LOCAL: &str = "#/$defs/";

/// Point every `$defs` entry that an imported file publishes at that file.
///
/// `root` is the schema generated for `file`; `imports` are the finished schemas of the files
/// it may reference, by file name. For each local definition an import also has, the local
/// copy must equal the import's (else two Rust types share a schema name). Each
/// `#/$defs/X...` reference to such a definition becomes `<owner>#/$defs/X...`, and the local
/// copy is removed. When the file has a root shape, local definitions it no longer reaches are
/// removed too.
pub fn link_shared_defs(
    file: &'static str,
    root: &mut Value,
    imports: &[(&'static str, &Value)],
) -> Result<()> {
    let owners = owners(file, imports)?;
    let Some(object) = root.as_object_mut() else {
        return Err(Error::NotAnObject { file });
    };
    let mut defs = match object.remove("$defs") {
        Some(Value::Object(defs)) => defs,
        _ => Map::new(),
    };
    for (name, local) in &defs {
        if let Some((owner, published)) = owners.get(name.as_str()) {
            let mut local = local.clone();
            qualify(&mut local, &|def| owners.get(def).map(|(f, _)| *f));
            let mut published = (*published).clone();
            qualify(&mut published, &|_| Some(*owner));
            if local != published {
                return Err(Error::SharedDefDiffers {
                    file,
                    def: name.clone(),
                    owner,
                });
            }
        }
    }
    defs.retain(|name, _| !owners.contains_key(name.as_str()));
    let owner_of = |def: &str| owners.get(def).map(|(f, _)| *f);
    for def in defs.values_mut() {
        qualify(def, &owner_of);
    }
    let mut shape = Value::Object(std::mem::take(object));
    qualify(&mut shape, &owner_of);
    let Value::Object(shape) = shape else {
        return Err(Error::NotAnObject { file });
    };
    *object = shape;
    if has_root_shape(object) {
        let reached = reachable(object, &defs);
        defs.retain(|name, _| reached.contains(name));
    }
    if !defs.is_empty() {
        object.insert("$defs".into(), Value::Object(defs));
    }
    Ok(())
}

/// Write each named local definition in place of every reference to it, and drop it from
/// `$defs`, so it is published only where its owner publishes it. Keywords beside a `$ref`
/// (a description, say) are kept and win over the definition's own.
pub(crate) fn inline_defs(file: &'static str, root: &mut Value, names: &[&str]) -> Result<()> {
    let Some(object) = root.as_object_mut() else {
        return Err(Error::NotAnObject { file });
    };
    let mut defs = match object.remove("$defs") {
        Some(Value::Object(defs)) => defs,
        _ => Map::new(),
    };
    let mut bodies = Map::new();
    for name in names {
        let Some(body) = defs.remove(*name) else {
            return Err(Error::UnknownInline {
                file,
                def: (*name).to_owned(),
            });
        };
        bodies.insert((*name).to_owned(), body);
    }
    let mut failure = None;
    let mut replace = |schema: &mut Value| {
        replace_refs(schema, &bodies, &mut |def| {
            failure.get_or_insert(Error::InlinePointer {
                file,
                def: def.to_owned(),
            });
        });
    };
    for def in defs.values_mut() {
        replace(def);
    }
    let mut shape = Value::Object(std::mem::take(object));
    replace(&mut shape);
    if let Some(err) = failure {
        return Err(err);
    }
    let Value::Object(shape) = shape else {
        return Err(Error::NotAnObject { file });
    };
    *object = shape;
    if !defs.is_empty() {
        object.insert("$defs".into(), Value::Object(defs));
    }
    Ok(())
}

/// Replace each object whose `$ref` names a definition in `bodies` with that definition plus
/// the object's other keywords. A reference into such a definition calls `into_def`.
fn replace_refs(schema: &mut Value, bodies: &Map<String, Value>, into_def: &mut dyn FnMut(&str)) {
    match schema {
        Value::Object(map) => {
            let target = map
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|r| r.strip_prefix(LOCAL))
                .map(str::to_owned);
            if let Some(rest) = target {
                if let Some(body) = bodies.get(rest.as_str()) {
                    map.remove("$ref");
                    let siblings = std::mem::take(map);
                    if let Value::Object(body) = body.clone() {
                        *map = body;
                    }
                    map.extend(siblings);
                } else if bodies.contains_key(def_name(&rest)) {
                    into_def(def_name(&rest));
                }
            }
            for (key, member) in map.iter_mut() {
                match key.as_str() {
                    keyword if DATA.contains(&keyword) => {}
                    keyword if NAMED.contains(&keyword) => {
                        if let Value::Object(named) = member {
                            named
                                .values_mut()
                                .for_each(|sub| replace_refs(sub, bodies, into_def));
                        }
                    }
                    _ => replace_refs(member, bodies, into_def),
                }
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| replace_refs(item, bodies, into_def)),
        _ => {}
    }
}

/// Definition name to owning file and its published definition.
fn owners<'a>(
    file: &'static str,
    imports: &[(&'static str, &'a Value)],
) -> Result<BTreeMap<&'a str, (&'static str, &'a Value)>> {
    let mut owners: BTreeMap<&str, (&'static str, &Value)> = BTreeMap::new();
    for (import, schema) in imports {
        let Some(defs) = schema.get("$defs").and_then(Value::as_object) else {
            continue;
        };
        for (name, def) in defs {
            if let Some((first, _)) = owners.insert(name.as_str(), (import, def)) {
                return Err(Error::AmbiguousOwner {
                    file,
                    def: name.clone(),
                    first,
                    second: import,
                });
            }
        }
    }
    Ok(owners)
}

/// Keywords whose value is instance data, not a schema.
const DATA: [&str; 4] = ["const", "enum", "default", "examples"];
/// Keywords whose value maps names to schemas; the names are not keywords.
const NAMED: [&str; 4] = [
    "properties",
    "patternProperties",
    "$defs",
    "dependentSchemas",
];

/// Call `f` on the target of every `$ref` in `schema` and its subschemas.
fn each_ref(schema: &mut Value, f: &mut dyn FnMut(&mut String)) {
    match schema {
        Value::Object(map) => {
            for (key, member) in map.iter_mut() {
                match key.as_str() {
                    "$ref" => {
                        if let Value::String(target) = member {
                            f(target);
                        }
                    }
                    keyword if DATA.contains(&keyword) => {}
                    keyword if NAMED.contains(&keyword) => {
                        if let Value::Object(named) = member {
                            named.values_mut().for_each(|sub| each_ref(sub, f));
                        }
                    }
                    _ => each_ref(member, f),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| each_ref(item, f)),
        _ => {}
    }
}

/// Rewrite each local reference `#/$defs/X...` to `<file>#/$defs/X...` where `owner_of(X)`
/// names a file.
fn qualify(value: &mut Value, owner_of: &dyn Fn(&str) -> Option<&'static str>) {
    each_ref(value, &mut |target| {
        if let Some(rest) = target.strip_prefix(LOCAL)
            && let Some(owner) = owner_of(def_name(rest))
        {
            *target = format!("{owner}{target}");
        }
    });
}

/// The definition name at the start of a pointer tail such as `LineCol/properties/line`.
fn def_name(rest: &str) -> &str {
    rest.split('/').next().unwrap_or(rest)
}

/// Whether the schema validates something itself, rather than only holding `$defs`.
fn has_root_shape(object: &Map<String, Value>) -> bool {
    object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "$schema" | "$id" | "$comment" | "title" | "description" | "$defs"
        )
    })
}

/// Names of the local definitions reached from the root shape, directly or through other
/// local definitions.
fn reachable(object: &Map<String, Value>, defs: &Map<String, Value>) -> BTreeSet<String> {
    let shape: Map<String, Value> = object
        .iter()
        .filter(|(key, _)| *key != "$defs")
        .map(|(key, member)| (key.clone(), member.clone()))
        .collect();
    let mut queue = local_refs(Value::Object(shape));
    let mut seen = BTreeSet::new();
    while let Some(name) = queue.pop() {
        if seen.insert(name.clone())
            && let Some(def) = defs.get(&name)
        {
            queue.extend(local_refs(def.clone()));
        }
    }
    seen
}

/// The definition names of the local references in `schema`.
fn local_refs(mut schema: Value) -> Vec<String> {
    let mut names = Vec::new();
    each_ref(&mut schema, &mut |target| {
        if let Some(rest) = target.strip_prefix(LOCAL) {
            names.push(def_name(rest).to_owned());
        }
    });
    names
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn common() -> Value {
        json!({"$defs": {
            "Span": {"type": "object", "properties": {"start": {"type": "integer"}}},
            "Edit": {"type": "object", "properties": {"span": {"$ref": "#/$defs/Span"}}},
        }})
    }

    #[test]
    fn a_def_another_file_owns_is_referenced_there_and_dropped_here() {
        let mut report = json!({
            "type": "object",
            "properties": {
                "fix": {"$ref": "#/$defs/Fix"},
                "start": {"$ref": "#/$defs/Span/properties/start"},
            },
            "$defs": {
                "Fix": {"type": "array", "items": {"$ref": "#/$defs/Edit"}},
                "Edit": {"type": "object", "properties": {"span": {"$ref": "#/$defs/Span"}}},
                "Span": {"type": "object", "properties": {"start": {"type": "integer"}}},
            },
        });
        link_shared_defs(
            "lint-report.schema.json",
            &mut report,
            &[("common.schema.json", &common())],
        )
        .unwrap();
        assert_eq!(
            report,
            json!({
                "type": "object",
                "properties": {
                    "fix": {"$ref": "#/$defs/Fix"},
                    "start": {"$ref": "common.schema.json#/$defs/Span/properties/start"},
                },
                "$defs": {
                    "Fix": {"type": "array", "items": {"$ref": "common.schema.json#/$defs/Edit"}},
                },
            })
        );
    }

    #[test]
    fn a_local_def_the_root_no_longer_reaches_is_dropped() {
        let mut schema = json!({
            "type": "object",
            "$defs": {"Unused": {"type": "string"}, "Kept": {"type": "integer"}},
            "properties": {"n": {"$ref": "#/$defs/Kept"}},
        });
        link_shared_defs("x.schema.json", &mut schema, &[]).unwrap();
        assert_eq!(schema["$defs"], json!({"Kept": {"type": "integer"}}));
    }

    #[test]
    fn a_defs_only_file_keeps_every_def() {
        let mut schema = common();
        link_shared_defs("common.schema.json", &mut schema, &[]).unwrap();
        assert_eq!(schema, common());
    }

    #[test]
    fn a_local_def_that_differs_from_the_owners_is_an_error() {
        let mut report = json!({
            "$ref": "#/$defs/Span",
            "$defs": {"Span": {"type": "string"}},
        });
        let err = link_shared_defs(
            "lint-report.schema.json",
            &mut report,
            &[("common.schema.json", &common())],
        )
        .unwrap_err();
        assert!(matches!(err, Error::SharedDefDiffers { ref def, .. } if def == "Span"));
    }

    #[test]
    fn two_imports_that_define_one_name_are_an_error() {
        let mut schema = json!({"$ref": "#/$defs/Span"});
        let other = json!({"$defs": {"Span": {"type": "string"}}});
        let err = link_shared_defs(
            "x.schema.json",
            &mut schema,
            &[
                ("common.schema.json", &common()),
                ("rule.schema.json", &other),
            ],
        )
        .unwrap_err();
        assert!(matches!(err, Error::AmbiguousOwner { ref def, .. } if def == "Span"));
    }

    #[test]
    fn a_property_named_like_a_data_keyword_is_still_a_schema() {
        let mut schema = json!({
            "type": "object",
            "properties": {"examples": {"items": {"$ref": "#/$defs/Span"}}},
        });
        link_shared_defs(
            "x.schema.json",
            &mut schema,
            &[("common.schema.json", &common())],
        )
        .unwrap();
        assert_eq!(
            schema["properties"]["examples"]["items"]["$ref"],
            "common.schema.json#/$defs/Span"
        );
    }

    #[test]
    fn instance_values_that_look_like_references_are_left_alone() {
        let mut schema = json!({"const": {"$ref": "#/$defs/Span"}});
        link_shared_defs(
            "x.schema.json",
            &mut schema,
            &[("common.schema.json", &common())],
        )
        .unwrap();
        assert_eq!(schema, json!({"const": {"$ref": "#/$defs/Span"}}));
    }
}
