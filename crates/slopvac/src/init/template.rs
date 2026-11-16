//! Generates the starter config from the config schema.
//!
//! Only tests build this module. The checked-in `templates/slopvac.toml` is what ships;
//! `template_is_current` fails when the schema moves and the file does not. Regenerate with
//! `SLOPVAC_UPDATE_TEMPLATE=1 cargo test -p slopvac template_is_current`.
//!
//! [`generate`] with `live = true` writes the same text with every example uncommented. A
//! test validates that config against the schema, so each example meets its constraints.
#![expect(
    clippy::let_underscore_must_use,
    reason = "fmt::Write for String never returns an error"
)]

use std::fmt::Write as _;

use serde_json::{Value, json};

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
/// Names tried, in order, for a string with a `pattern`. The first one that matches wins.
const NAME_CANDIDATES: [&str; 2] = ["example", "example-name"];

/// The two schemas and whether examples are live.
struct Ctx<'a> {
    root: &'a Value,
    common: &'a Value,
    live: bool,
}

impl Ctx<'_> {
    /// `# ` before a commented example, nothing before a live one.
    fn mark(&self) -> &'static str {
        if self.live { "" } else { "# " }
    }

    fn resolve<'s>(&'s self, schema: &'s Value) -> &'s Value {
        let Some(r) = schema.get("$ref").and_then(Value::as_str) else {
            return schema;
        };
        let (base, path) = match r.split_once('#') {
            Some(("", path)) => (self.root, path),
            Some(("common.schema.json", path)) => (self.common, path),
            _ => return schema,
        };
        path.strip_prefix('/')
            .and_then(|p| p.split('/').try_fold(base, |v, k| v.get(k)))
            .unwrap_or(schema)
    }

    /// The first sentence of the description, preferring the one next to the reference.
    fn describe(&self, schema: &Value) -> Option<String> {
        let text = schema
            .get("description")
            .or_else(|| self.resolve(schema).get("description"))?
            .as_str()?;
        let end = text.find(". ").map_or(text.len(), |i| i + 1);
        Some(text[..end].trim().to_owned())
    }

    /// A value for a key that meets the schema's constraints.
    fn example(&self, key: &str, schema: &Value) -> String {
        let s = self.resolve(schema);
        if let Some(d) = s.get("default").or_else(|| schema.get("default")) {
            return toml_value(d);
        }
        for branch in ["then", "else"] {
            if let Some(inner) = s.get(branch)
                && inner.get("type").is_none_or(|t| t != "object")
            {
                return self.example(key, inner);
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
            return self.example(key, first);
        }
        match s.get("type").and_then(Value::as_str) {
            Some("boolean") => "false".into(),
            Some("integer" | "number") => s
                .get("minimum")
                .map_or_else(|| "0".into(), ToString::to_string),
            Some("array") => {
                let needed = s.get("minItems").and_then(Value::as_u64).unwrap_or(0);
                match s.get("items") {
                    Some(items) if needed > 0 => format!("[{}]", self.example(key, items)),
                    _ => "[]".into(),
                }
            }
            Some("object") => "{}".into(),
            _ => string_example(key, s),
        }
    }

    fn field(&self, out: &mut String, key: &str, schema: &Value) {
        if let Some(text) = self.describe(schema) {
            comment(out, &text);
        }
        let _ = writeln!(out, "{}{key} = {}", self.mark(), self.example(key, schema));
    }

    /// Keys of a table or an array-of-tables item, required keys first.
    fn fields(&self, out: &mut String, schema: &Value) {
        let s = self.resolve(schema);
        let required: Vec<&str> = s
            .get("required")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        if let Some(props) = s.get("properties").and_then(Value::as_object) {
            let mut keys: Vec<&String> = props.keys().collect();
            keys.sort_by_key(|k| !required.contains(&k.as_str()));
            for key in keys {
                self.field(out, key, &props[key]);
            }
        } else if let Some(values) = s.get("additionalProperties") {
            let name = s.get("propertyNames").map_or_else(
                || "\"<key>\"".into(),
                |n| string_example("name", self.resolve(n)),
            );
            let _ = writeln!(
                out,
                "{}{name} = {}",
                self.mark(),
                self.example("value", values)
            );
        }
    }
}

/// A string that satisfies the schema's `pattern`, or a `<key>` placeholder without one.
fn string_example(key: &str, schema: &Value) -> String {
    let Some(pattern) = schema.get("pattern") else {
        return format!("\"<{key}>\"");
    };
    let checker = jsonschema::validator_for(&json!({"pattern": pattern}))
        .expect("the schema pattern compiles");
    let name = NAME_CANDIDATES
        .iter()
        .find(|c| checker.is_valid(&json!(c)))
        .unwrap_or_else(|| panic!("no example name fits the pattern of `{key}`"));
    format!("\"{name}\"")
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
        other @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_)) => {
            other.to_string()
        }
    }
}

/// Build the template text. With `live`, examples are not commented out.
#[must_use]
pub(super) fn generate(schema: &Value, common: &Value, live: bool) -> String {
    let ctx = Ctx {
        root: schema,
        common,
        live,
    };
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
            if let Some(text) = ctx.describe(lookup(key)) {
                comment(&mut out, &text);
            }
            let _ = writeln!(out, "{key} = {value}");
        } else {
            ctx.field(&mut out, key, lookup(key));
        }
    }
    for key in TABLES {
        out.push('\n');
        if let Some(text) = ctx.describe(lookup(key)) {
            comment(&mut out, &text);
        }
        let _ = writeln!(out, "{}[{key}]", ctx.mark());
        ctx.fields(&mut out, lookup(key));
    }
    for key in ARRAYS {
        out.push('\n');
        let items = &ctx.resolve(lookup(key))["items"];
        if let Some(text) = ctx.describe(items) {
            comment(&mut out, &text);
        }
        let _ = writeln!(out, "{}[[{key}]]", ctx.mark());
        ctx.fields(&mut out, items);
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

    fn read(name: &str) -> Value {
        let text = std::fs::read_to_string(root().join("schemas/v3").join(name)).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn template(live: bool) -> String {
        generate(
            &read("config.schema.json"),
            &read("common.schema.json"),
            live,
        )
    }

    #[test]
    fn template_is_current() {
        let generated = template(false);
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates/slopvac.toml");
        if std::env::var_os("SLOPVAC_UPDATE_TEMPLATE").is_some() {
            std::fs::write(&path, &generated).unwrap();
        }
        // A Windows checkout may convert the committed file to CRLF.
        let checked_in = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            checked_in == generated,
            "templates/slopvac.toml is stale; run `SLOPVAC_UPDATE_TEMPLATE=1 cargo test -p slopvac template_is_current`"
        );
    }

    #[test]
    fn every_schema_key_has_a_place_in_the_template() {
        let schema = read("config.schema.json");
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

    /// Uncommenting every example gives a config the schema accepts: each example meets
    /// its constraints (minimum list lengths, identifier patterns).
    #[test]
    fn every_example_validates_against_the_schema_when_uncommented() {
        let registry = jsonschema::Registry::new()
            .add(
                "https://srobroek.github.io/slopvac/schemas/v3/common.schema.json",
                read("common.schema.json"),
            )
            .unwrap()
            .prepare()
            .unwrap();
        let validator = jsonschema::options()
            .with_registry(&registry)
            .build(&read("config.schema.json"))
            .unwrap();
        let live = template(true);
        let config: toml::Value = toml::from_str(&live)
            .unwrap_or_else(|e| panic!("the live template is TOML: {e}\n{live}"));
        let problems: Vec<String> = validator
            .iter_errors(&serde_json::to_value(&config).unwrap())
            .map(|e| format!("{} at {}", e, e.instance_path()))
            .collect();
        assert!(problems.is_empty(), "{problems:#?}\n{live}");
    }
}
