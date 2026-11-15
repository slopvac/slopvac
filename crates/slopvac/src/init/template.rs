//! Generates the starter config from the config schema.
//!
//! Only tests build this module. The checked-in `templates/slopvac.toml` is what ships;
//! `template_is_current` fails when the schema moves and the file does not. Regenerate with
//! `SLOPVAC_UPDATE_TEMPLATE=1 cargo test -p slopvac template_is_current`.

use std::fmt::Write as _;

use serde_json::Value;

const WIDTH: usize = 78;

/// Plain keys, in the order the template lists them.
const SCALARS: [&str; 6] = ["mode", "extends", "styles", "exclude", "profile", "locale"];
/// Tables.
const TABLES: [&str; 8] = [
    "thresholds",
    "categories",
    "rules",
    "terms",
    "frontmatter",
    "guidance",
    "network",
    "review",
];
/// Arrays of tables.
const ARRAYS: [&str; 4] = ["packs", "plugins", "genres", "overrides"];
/// Keys the template sets for real.
const ACTIVE: [(&str, &str); 2] = [("schema_version", "3"), ("profile", "\"normal\"")];

fn resolve<'a>(root: &'a Value, schema: &'a Value) -> &'a Value {
    match schema.get("$ref").and_then(Value::as_str) {
        Some(r) => r
            .strip_prefix("#/")
            .and_then(|p| p.split('/').try_fold(root, |v, k| v.get(k)))
            .unwrap_or(schema),
        None => schema,
    }
}

/// The description, preferring the one next to the reference.
fn describe(root: &Value, schema: &Value) -> Option<String> {
    let text = schema
        .get("description")
        .or_else(|| resolve(root, schema).get("description"))?
        .as_str()?;
    let end = text.find(". ").map_or(text.len(), |i| i + 1);
    Some(text[..end].trim().to_owned())
}

fn comment(out: &mut String, text: &str) {
    let mut line = String::from("#");
    for word in text.split_whitespace() {
        if line.len() + 1 + word.len() > WIDTH && line.len() > 1 {
            out.push_str(&line);
            out.push('\n');
            line = String::from("#");
        }
        line.push(' ');
        line.push_str(word);
    }
    out.push_str(&line);
    out.push('\n');
}

fn toml_value(v: &Value) -> String {
    match v {
        Value::String(s) => format!("{s:?}"),
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(toml_value).collect::<Vec<_>>().join(", ")
        ),
        other => other.to_string(),
    }
}

/// A plausible value for a key, from the schema.
fn example(root: &Value, key: &str, schema: &Value) -> String {
    let s = resolve(root, schema);
    if let Some(d) = s.get("default").or_else(|| schema.get("default")) {
        return toml_value(d);
    }
    for branch in ["then", "else"] {
        if let Some(inner) = s.get(branch)
            && inner.get("type").is_none_or(|t| t != "object")
        {
            return example(root, key, inner);
        }
    }
    if let Some(first) = s
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|e| e.first())
    {
        return toml_value(first);
    }
    if let Some(first) = s
        .get("oneOf")
        .and_then(Value::as_array)
        .and_then(|e| e.first())
    {
        return example(root, key, first);
    }
    match s.get("type").and_then(Value::as_str) {
        Some("boolean") => "false".into(),
        Some("integer" | "number") => s
            .get("minimum")
            .map_or_else(|| "0".into(), ToString::to_string),
        Some("array") => "[]".into(),
        Some("object") => "{}".into(),
        _ => format!("\"<{key}>\""),
    }
}

fn field(out: &mut String, root: &Value, key: &str, schema: &Value) {
    if let Some(text) = describe(root, schema) {
        comment(out, &text);
    }
    let _ = writeln!(out, "# {key} = {}", example(root, key, schema));
}

/// Commented keys of a table or an array-of-tables item, required keys first.
fn fields(out: &mut String, root: &Value, schema: &Value) {
    let s = resolve(root, schema);
    let required: Vec<&str> = s
        .get("required")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if let Some(props) = s.get("properties").and_then(Value::as_object) {
        let mut keys: Vec<&String> = props.keys().collect();
        keys.sort_by_key(|k| !required.contains(&k.as_str()));
        for key in keys {
            field(out, root, key, &props[key]);
        }
    } else if let Some(values) = s.get("additionalProperties") {
        let label = if s.get("propertyNames").is_some() {
            "<name>"
        } else {
            "<key>"
        };
        let _ = writeln!(out, "# \"{label}\" = {}", example(root, label, values));
    }
}

/// Build the template text.
#[must_use]
pub(super) fn generate(schema: &Value) -> String {
    let settings = &schema["$defs"]["Settings"]["properties"];
    let lookup = |key: &str| -> &Value {
        schema["properties"]
            .get(key)
            .or_else(|| settings.get(key))
            .unwrap_or_else(|| panic!("the config schema has no `{key}`"))
    };
    let mut out = String::new();
    for line in [
        "slopvac configuration.",
        "Generated from schemas/v3/config.schema.json. Edit your copy, not the template.",
        "Unknown keys are errors. Keys that start with `x-` are ignored.",
        "Each commented line shows a setting with its default or an example.",
    ] {
        comment(&mut out, line);
    }
    out.push('\n');
    let _ = writeln!(out, "{} = {}", ACTIVE[0].0, ACTIVE[0].1);
    for key in SCALARS {
        out.push('\n');
        if let Some((_, value)) = ACTIVE.iter().find(|(k, _)| *k == key) {
            if let Some(text) = describe(schema, lookup(key)) {
                comment(&mut out, &text);
            }
            let _ = writeln!(out, "{key} = {value}");
        } else {
            field(&mut out, schema, key, lookup(key));
        }
    }
    for key in TABLES {
        out.push('\n');
        if let Some(text) = describe(schema, lookup(key)) {
            comment(&mut out, &text);
        }
        let _ = writeln!(out, "# [{key}]");
        fields(&mut out, schema, lookup(key));
    }
    for key in ARRAYS {
        out.push('\n');
        let items = &resolve(schema, lookup(key))["items"];
        if let Some(text) = describe(schema, items) {
            comment(&mut out, &text);
        }
        let _ = writeln!(out, "# [[{key}]]");
        fields(&mut out, schema, items);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn schema() -> Value {
        let text = std::fs::read_to_string(root().join("schemas/v3/config.schema.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn template_is_current() {
        let generated = generate(&schema());
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates/slopvac.toml");
        if std::env::var_os("SLOPVAC_UPDATE_TEMPLATE").is_some() {
            std::fs::write(&path, &generated).unwrap();
        }
        let checked_in = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            checked_in == generated,
            "templates/slopvac.toml is stale; run `SLOPVAC_UPDATE_TEMPLATE=1 cargo test -p slopvac template_is_current`"
        );
    }

    #[test]
    fn every_schema_key_has_a_place_in_the_template() {
        let schema = schema();
        let mut known: Vec<&str> = SCALARS
            .iter()
            .chain(&TABLES)
            .chain(&ARRAYS)
            .copied()
            .chain(["schema_version"])
            .collect();
        known.sort_unstable();
        let mut keys: Vec<&str> = schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .chain(
                schema["$defs"]["Settings"]["properties"]
                    .as_object()
                    .unwrap()
                    .keys(),
            )
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(
            keys, known,
            "a schema key is missing from the template lists"
        );
    }
}
