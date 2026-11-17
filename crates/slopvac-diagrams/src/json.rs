//! A small JSON reader that keeps the byte range of every string value.
//!
//! `serde_json` gives no offsets, and the extractors need them to report spans.

use crate::sub;
use std::ops::Range;

const MAX_DEPTH: usize = 128;

#[derive(Debug)]
pub(crate) enum Json {
    Bool(bool),
    Other,
    Str { span: Range<usize>, value: String },
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub(crate) fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            Json::Bool(_) | Json::Other | Json::Str { .. } | Json::Arr(_) => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str { value, .. } => Some(value),
            Json::Bool(_) | Json::Other | Json::Arr(_) | Json::Obj(_) => None,
        }
    }
}

pub(crate) fn parse(src: &str) -> Option<Json> {
    let mut p = Parser { src, pos: 0 };
    let v = p.value(0)?;
    p.ws();
    (p.pos == src.len()).then_some(v)
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.as_bytes().get(self.pos).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        let hit = self.peek() == Some(c);
        if hit {
            self.pos += 1;
        }
        hit
    }

    fn value(&mut self, depth: usize) -> Option<Json> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.ws();
        match self.peek()? {
            b'{' => {
                self.pos += 1;
                let mut members = Vec::new();
                if self.eat(b'}') {
                    return Some(Json::Obj(members));
                }
                loop {
                    self.ws();
                    let (_, key) = self.string()?;
                    if !self.eat(b':') {
                        return None;
                    }
                    members.push((key, self.value(depth + 1)?));
                    if self.eat(b',') {
                        continue;
                    }
                    return self.eat(b'}').then_some(Json::Obj(members));
                }
            }
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                if self.eat(b']') {
                    return Some(Json::Arr(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    if self.eat(b',') {
                        continue;
                    }
                    return self.eat(b']').then_some(Json::Arr(items));
                }
            }
            b'"' => {
                let (span, value) = self.string()?;
                Some(Json::Str { span, value })
            }
            _ => {
                let start = self.pos;
                while self
                    .peek()
                    .is_some_and(|c| !matches!(c, b',' | b']' | b'}') && !c.is_ascii_whitespace())
                {
                    self.pos += 1;
                }
                match sub(self.src, start..self.pos) {
                    "" => None,
                    "true" => Some(Json::Bool(true)),
                    "false" => Some(Json::Bool(false)),
                    _ => Some(Json::Other),
                }
            }
        }
    }

    /// Read a string. Return the range inside the quotes and the decoded value.
    fn string(&mut self) -> Option<(Range<usize>, String)> {
        if self.peek()? != b'"' {
            return None;
        }
        self.pos += 1;
        let start = self.pos;
        let mut out = String::new();
        loop {
            let rest = sub(self.src, self.pos..);
            let c = rest.chars().next()?;
            self.pos += c.len_utf8();
            match c {
                '"' => return Some((start..self.pos - 1, out)),
                '\\' => out.push(self.escape()?),
                c if c < '\u{20}' => return None,
                c => out.push(c),
            }
        }
    }

    fn escape(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        Some(match c {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => {
                let hi = self.hex4()?;
                if (0xD800..0xDC00).contains(&hi) {
                    if sub(self.src, self.pos..).starts_with("\\u") {
                        self.pos += 2;
                        let lo = self.hex4()?;
                        char::from_u32(0x10000 + ((hi - 0xD800) << 10) + lo.checked_sub(0xDC00)?)?
                    } else {
                        return None;
                    }
                } else {
                    char::from_u32(hi)?
                }
            }
            _ => return None,
        })
    }

    fn hex4(&mut self) -> Option<u32> {
        let digits = self.src.get(self.pos..self.pos + 4)?;
        self.pos += 4;
        u32::from_str_radix(digits, 16).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_cover_the_raw_bytes_inside_the_quotes() {
        let src = r#"{"a": "x\ny", "b": ["é"]}"#;
        let v = parse(src).unwrap();
        let Json::Str { span, value } = v.get("a").unwrap() else {
            panic!()
        };
        assert_eq!(sub(src, span.clone()), r"x\ny");
        assert_eq!(value, "x\ny");
        let Json::Arr(items) = v.get("b").unwrap() else {
            panic!()
        };
        let Json::Str { span, .. } = &items[0] else {
            panic!()
        };
        assert_eq!(sub(src, span.clone()), "é");
    }

    #[test]
    fn decodes_surrogate_pairs() {
        let v = parse(r#"{"k":"\ud83d\ude00"}"#).unwrap();
        assert_eq!(v.get("k").unwrap().as_str(), Some("😀"));
    }

    #[test]
    fn rejects_malformed_and_overdeep_input() {
        assert!(parse("{\"a\": ").is_none());
        assert!(parse(&"[".repeat(500)).is_none());
        assert!(parse("{} x").is_none());
    }
}
