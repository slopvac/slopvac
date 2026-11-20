use std::fmt::Write as _;

use serde_json::Value;

const INDENT: &str = "  ";

/// Append formatted text to `out`.
#[expect(
    clippy::let_underscore_must_use,
    reason = "writing to a `String` cannot fail"
)]
fn push_fmt(out: &mut String, args: std::fmt::Arguments<'_>) {
    let _ = out.write_fmt(args);
}

/// Write `value` as canonical JSON: object keys sorted by their UTF-8 bytes, two-space
/// indent, one member or element per line and a trailing newline.
///
/// The key order does not depend on how `serde_json::Map` stores keys, so the output stays
/// the same whether or not another crate in the build turns on `serde_json/preserve_order`.
#[must_use]
pub fn to_canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value, 0);
    out.push('\n');
    out
}

fn write_value(out: &mut String, value: &Value, depth: usize) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            out.push('{');
            for (i, (key, member)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, depth + 1);
                write_string(out, key);
                out.push_str(": ");
                write_value(out, member, depth + 1);
            }
            newline(out, depth);
            out.push('}');
        }
        Value::Array(items) if !items.is_empty() => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, depth + 1);
                write_value(out, item, depth + 1);
            }
            newline(out, depth);
            out.push(']');
        }
        Value::Object(_) => out.push_str("{}"),
        Value::Array(_) => out.push_str("[]"),
        Value::String(text) => write_string(out, text),
        Value::Number(number) => push_fmt(out, format_args!("{number}")),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Null => out.push_str("null"),
    }
}

fn newline(out: &mut String, depth: usize) {
    out.push('\n');
    for _ in 0..depth {
        out.push_str(INDENT);
    }
}

/// A JSON string literal: `"`, `\` and control characters escaped, everything else as is.
fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if u32::from(c) < 0x20 => push_fmt(out, format_args!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn keys_sort_at_every_depth_and_the_text_ends_in_a_newline() {
        let value = json!({"b": {"z": 1, "a": [true, null]}, "$id": "x", "a": {}});
        let want = "{\n  \"$id\": \"x\",\n  \"a\": {},\n  \"b\": {\n    \"a\": [\n      true,\n      null\n    ],\n    \"z\": 1\n  }\n}\n";
        assert_eq!(to_canonical_json(&value), want);
    }

    #[test]
    fn the_same_members_in_any_insertion_order_write_the_same_text() {
        let keys = ["properties", "$defs", "type", "additionalProperties", "$id"];
        let build = |order: &mut dyn Iterator<Item = &&str>| {
            let mut map = serde_json::Map::new();
            for key in order {
                map.insert((*key).to_owned(), json!(key.len()));
            }
            Value::Object(map)
        };
        let forward = to_canonical_json(&build(&mut keys.iter()));
        let backward = to_canonical_json(&build(&mut keys.iter().rev()));
        assert_eq!(forward, backward);
    }

    #[test]
    fn strings_escape_only_what_json_requires() {
        let value = json!(["a\"b\\c\n\u{1}", "é→"]);
        assert_eq!(
            to_canonical_json(&value),
            "[\n  \"a\\\"b\\\\c\\n\\u0001\",\n  \"é→\"\n]\n"
        );
    }

    #[test]
    fn the_text_parses_back_to_the_same_value() {
        let value = json!({"x": [1.5, -2, {"k": "v"}], "y": []});
        let text = to_canonical_json(&value);
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), value);
    }
}
