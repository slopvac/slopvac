//! Strict JSON parsing for plugin messages.
//!
//! Rejects what plugin-protocol.md section 2.2 calls invalid in v1: duplicate
//! object keys, nesting deeper than 64 and integers outside ±(2^53-1).
//! Whether a number must be an integer is left to the typed layer: `serde`
//! refuses `1.0` and `1e2` for an integer field.

use std::collections::HashSet;
use std::fmt;

use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

/// Largest integer that every JSON reader keeps exact.
pub const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;
/// Deepest container nesting a reader accepts.
pub const MAX_DEPTH: usize = 64;

/// Parse `bytes` as one strict JSON value.
///
/// # Errors
/// Returns the reason as text when the body is not valid JSON or breaks a v1 rule.
pub fn parse(bytes: &[u8]) -> Result<Value, String> {
    let mut de = serde_json::Deserializer::from_slice(bytes);
    let value = Strict { depth: 0 }
        .deserialize(&mut de)
        .map_err(|e| e.to_string())?;
    de.end().map_err(|e| e.to_string())?;
    Ok(value)
}

struct Strict {
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for Strict {
    type Value = Value;

    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        d.deserialize_any(StrictVisitor { depth: self.depth })
    }
}

struct StrictVisitor {
    depth: usize,
}

impl StrictVisitor {
    fn enter<E: de::Error>(&self) -> Result<usize, E> {
        let depth = self.depth + 1;
        if depth > MAX_DEPTH {
            return Err(E::custom(format!("nesting deeper than {MAX_DEPTH}")));
        }
        Ok(depth)
    }
}

impl<'de> Visitor<'de> for StrictVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
        if v.unsigned_abs() > MAX_SAFE_INTEGER {
            return Err(E::custom(format!("integer {v} outside ±(2^53-1)")));
        }
        Ok(Value::Number(v.into()))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
        if v > MAX_SAFE_INTEGER {
            return Err(E::custom(format!("integer {v} outside ±(2^53-1)")));
        }
        Ok(Value::Number(v.into()))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        Number::from_f64(v)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite number"))
    }

    fn visit_str<E>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_owned()))
    }

    fn visit_string<E>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let depth = self.enter()?;
        let mut items = Vec::new();
        while let Some(item) = seq.next_element_seed(Strict { depth })? {
            items.push(item);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let depth = self.enter()?;
        let mut seen = HashSet::new();
        let mut out = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(de::Error::custom(format!("duplicate key `{key}`")));
            }
            out.insert(key, map.next_value_seed(Strict { depth })?);
        }
        Ok(Value::Object(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_object() {
        let v = parse(br#"{"a":[1,2.5,"x",null,true]}"#).unwrap();
        assert_eq!(v["a"][0], 1);
        assert_eq!(v["a"][1], 2.5);
    }

    #[test]
    fn rejects_duplicate_keys_at_any_depth() {
        assert!(
            parse(br#"{"a":1,"a":2}"#)
                .unwrap_err()
                .contains("duplicate")
        );
        assert!(
            parse(br#"{"x":{"a":1,"a":1}}"#)
                .unwrap_err()
                .contains("duplicate")
        );
    }

    #[test]
    fn depth_64_passes_and_65_fails() {
        let ok = format!("{}{}", "[".repeat(64), "]".repeat(64));
        let bad = format!("{}{}", "[".repeat(65), "]".repeat(65));
        assert!(parse(ok.as_bytes()).is_ok());
        assert!(parse(bad.as_bytes()).unwrap_err().contains("nesting"));
    }

    #[test]
    fn integer_bounds_are_2_pow_53_minus_1() {
        assert!(parse(b"9007199254740991").is_ok());
        assert!(parse(b"-9007199254740991").is_ok());
        assert!(parse(b"9007199254740992").is_err());
        assert!(parse(b"-9007199254740992").is_err());
    }

    #[test]
    fn a_top_level_array_parses_and_is_left_to_the_envelope_check() {
        assert!(parse(b"[]").unwrap().is_array());
    }

    #[test]
    fn fractional_and_exponent_forms_are_not_integers() {
        for text in ["1.0", "1e2", "-1.0", "1E2"] {
            let v = parse(text.as_bytes()).unwrap();
            assert_eq!(v.as_u64(), None, "{text}");
            assert_eq!(v.as_i64(), None, "{text}");
            assert!(serde_json::from_value::<u64>(v).is_err(), "{text}");
        }
    }

    #[test]
    fn rejects_trailing_data_and_bad_json() {
        assert!(parse(b"{} x").is_err());
        assert!(parse(b"{").is_err());
    }
}
