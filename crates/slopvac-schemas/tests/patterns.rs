//! Every `pattern` in `schemas/v3` stays inside a regex subset that Python `re`, the Rust
//! jsonschema crate (fancy-regex, with ECMA translation) and ECMA-262 read the same way.
//!
//! Allowed: literals, `^`, explicit character classes, groups, lookahead, quantifiers,
//! alternation, escaped punctuation, `\n`-style control escapes, `\xHH` and `\uHHHH`, and the
//! end anchor `(?![\s\S])` as the last token of the pattern or of a group. Forbidden: a bare
//! `$` (Python lets it match before a final newline), the shorthand classes `\d`, `\w`, `\s` and their negations,
//! `\p{..}`, inline flags, lookbehind, named groups, backreferences, `\b`, `\A`, `\z`, `\Z`
//! `.` (the engines disagree on which line terminators it excludes) and `]` first in a class.
//!
//! `schemas/v3/conformance/patterns/vectors.json` holds inputs with both validators'
//! verdicts; the Python half of the check is `tests/contract/test_pattern_vectors.py`.

#![allow(missing_docs, reason = "an integration test crate has no public API")]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;
use serde_json::{Value, json};

/// The portable end-of-string anchor.
const END: &str = r"(?![\s\S])";

/// Patterns in files that are not generated yet, each fixed in the step that generates its
/// file. A pending entry that no longer breaks the subset fails the test, so the list only
/// shrinks.
const PENDING: &[(&str, &str, &str)] = &[
    (
        "blocklist.schema.json",
        "/$defs/BlocklistEntry/properties/reason",
        "slopvac-8l4.2.55",
    ),
    (
        "blocklist.schema.json",
        "/$defs/BlocklistEntry/properties/word",
        "slopvac-8l4.2.55",
    ),
    (
        "plugin-index.schema.json",
        "/$defs/PackIndexSource",
        "slopvac-8l4.2.56",
    ),
    (
        "plugin-manifest.schema.json",
        "/$defs/PackPath",
        "slopvac-8l4.2.52",
    ),
    (
        "plugin-manifest.schema.json",
        "/$defs/Plugin/properties/python",
        "slopvac-8l4.2.52",
    ),
    (
        "plugins-lock.schema.json",
        "/$defs/Locked/properties/source",
        "slopvac-8l4.2.56",
    ),
    (
        "plugin-protocol.schema.json",
        "/$defs/FeatureName",
        "slopvac-8l4.2.53",
    ),
    (
        "plugin-protocol.schema.json",
        "/$defs/PluginDocument/properties/path",
        "slopvac-8l4.2.53",
    ),
    (
        "plugin-protocol.schema.json",
        "/$defs/RecipeFile/properties/path",
        "slopvac-8l4.2.53",
    ),
    ("style.schema.json", "/$defs/RelPath", "slopvac-8l4.2.54"),
];

/// A construct outside the portable subset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Violation {
    BareDollar,
    Shorthand(char),
    UnicodeProperty,
    InlineFlag,
    Lookbehind,
    NamedGroup,
    Backreference,
    Boundary(char),
    Dot,
    BracketFirst,
    UnknownEscape(char),
}

/// The constructs in `pattern` outside the portable subset.
fn violations(pattern: &str) -> Vec<Violation> {
    let body = pattern.replace(&format!("{END})"), ")");
    let body = body.strip_suffix(END).unwrap_or(&body);
    let chars: Vec<char> = body.chars().collect();
    let at = |i: usize| chars.get(i).copied();
    let hex = |from: usize, n: usize| {
        (from..from + n).all(|i| at(i).is_some_and(|c| c.is_ascii_hexdigit()))
    };
    let mut out = Vec::new();
    let mut in_class = false;
    let mut i = 0;
    while let Some(c) = at(i) {
        match c {
            '\\' => {
                let Some(escaped) = at(i + 1) else {
                    out.push(Violation::UnknownEscape('\\'));
                    break;
                };
                match escaped {
                    'd' | 'D' | 'w' | 'W' | 's' | 'S' => out.push(Violation::Shorthand(escaped)),
                    'p' | 'P' => out.push(Violation::UnicodeProperty),
                    'b' | 'B' | 'A' | 'z' | 'Z' | 'G' => out.push(Violation::Boundary(escaped)),
                    '1'..='9' | 'k' => out.push(Violation::Backreference),
                    'u' if hex(i + 2, 4) => i += 4,
                    'x' if hex(i + 2, 2) => i += 2,
                    'n' | 'r' | 't' | 'f' | 'v' => {}
                    p if p.is_ascii_punctuation() => {}
                    other => out.push(Violation::UnknownEscape(other)),
                }
                i += 2;
                continue;
            }
            '[' if !in_class => {
                in_class = true;
                if at(i + 1) == Some('^') {
                    i += 1;
                }
                // Python reads `[]` as the start of a class holding `]`; ECMA as an empty
                // class.
                if at(i + 1) == Some(']') {
                    out.push(Violation::BracketFirst);
                    i += 1;
                }
            }
            ']' if in_class => in_class = false,
            '(' if !in_class && at(i + 1) == Some('?') => match at(i + 2) {
                Some(':' | '=' | '!') => {}
                Some('<') if matches!(at(i + 3), Some('=' | '!')) => {
                    out.push(Violation::Lookbehind);
                }
                Some('<' | 'P') => out.push(Violation::NamedGroup),
                _ => out.push(Violation::InlineFlag),
            },
            '$' if !in_class => out.push(Violation::BareDollar),
            '.' if !in_class => out.push(Violation::Dot),
            _ => {}
        }
        i += 1;
    }
    out
}

fn schema_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v3")
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// One `pattern` keyword: the file, the pointer to the schema that holds it, the pattern.
type Found = (String, String, String);

/// Every `pattern` keyword in `schemas/v3`. `patternProperties` keys are patterns too.
fn schema_patterns() -> Vec<Found> {
    let mut found = Vec::new();
    let mut paths: Vec<PathBuf> = fs::read_dir(schema_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".schema.json"))
        .collect();
    paths.sort();
    for path in paths {
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        walk(&read_json(&path), "", &mut |pointer, pattern| {
            found.push((file.clone(), pointer.to_owned(), pattern.to_owned()));
        });
    }
    found
}

fn escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

fn walk(schema: &Value, pointer: &str, found: &mut dyn FnMut(&str, &str)) {
    match schema {
        Value::Object(map) => {
            for (key, member) in map {
                let here = format!("{pointer}/{}", escape(key));
                match key.as_str() {
                    "const" | "enum" | "default" | "examples" => {}
                    "pattern" => {
                        if let Value::String(pattern) = member {
                            found(pointer, pattern);
                        }
                    }
                    "properties" | "patternProperties" | "$defs" | "dependentSchemas" => {
                        if let Value::Object(named) = member {
                            for (name, sub) in named {
                                if key == "patternProperties" {
                                    found(&here, name);
                                }
                                walk(sub, &format!("{here}/{}", escape(name)), found);
                            }
                        }
                    }
                    _ => walk(member, &here, found),
                }
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                walk(item, &format!("{pointer}/{i}"), found);
            }
        }
        _ => {}
    }
}

struct Vector {
    pattern: String,
    input: String,
    python: bool,
    rust: bool,
}

fn vectors() -> Vec<Vector> {
    let doc = read_json(&schema_dir().join("conformance/patterns/vectors.json"));
    doc["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| Vector {
            pattern: v["pattern"].as_str().unwrap().to_owned(),
            input: v["input"].as_str().unwrap().to_owned(),
            python: v["python"].as_bool().unwrap(),
            rust: v["rust"].as_bool().unwrap(),
        })
        .collect()
}

fn rust_accepts(pattern: &str, input: &str) -> bool {
    jsonschema::draft202012::new(&json!({"type": "string", "pattern": pattern}))
        .unwrap()
        .is_valid(&json!(input))
}

#[test]
fn every_schema_pattern_is_in_the_subset_or_pending_its_generation_step() {
    let pending: BTreeSet<(&str, &str)> = PENDING.iter().map(|(f, p, _)| (*f, *p)).collect();
    let mut outside = Vec::new();
    let mut still_pending = BTreeSet::new();
    for (file, pointer, pattern) in schema_patterns() {
        let found = violations(&pattern);
        if found.is_empty() {
            continue;
        }
        if pending.contains(&(file.as_str(), pointer.as_str())) {
            still_pending.insert((file, pointer));
        } else {
            outside.push(format!("{file}:{pointer}: {pattern} {found:?}"));
        }
    }
    assert!(
        outside.is_empty(),
        "outside the subset:\n{}",
        outside.join("\n")
    );
    let fixed: Vec<_> = pending
        .iter()
        .filter(|(f, p)| !still_pending.contains(&((*f).to_owned(), (*p).to_owned())))
        .collect();
    assert!(fixed.is_empty(), "drop these from PENDING: {fixed:?}");
}

#[test]
fn the_rust_validator_gives_each_vector_its_recorded_verdict() {
    for v in vectors() {
        assert_eq!(
            rust_accepts(&v.pattern, &v.input),
            v.rust,
            "{:?} against {:?}",
            v.input,
            v.pattern
        );
    }
}

#[test]
fn the_engines_disagree_only_on_patterns_outside_the_subset() {
    for v in vectors().iter().filter(|v| v.python != v.rust) {
        assert!(
            !violations(&v.pattern).is_empty(),
            "{:?} splits the engines on {:?} but passes the subset",
            v.pattern,
            v.input
        );
    }
}

#[test]
fn every_end_anchored_schema_pattern_has_a_trailing_newline_vector() {
    let covered: BTreeSet<String> = vectors()
        .into_iter()
        .filter(|v| v.input.ends_with('\n'))
        .map(|v| v.pattern)
        .collect();
    let missing: BTreeSet<String> = schema_patterns()
        .into_iter()
        .map(|(_, _, pattern)| pattern)
        .filter(|p| (p.ends_with('$') || p.ends_with(END)) && !covered.contains(p))
        .collect();
    assert!(
        missing.is_empty(),
        "no trailing-newline vector: {missing:?}"
    );
}

#[rstest]
#[case::slug(r"^[a-z][a-z0-9-]*(?![\s\S])")]
#[case::alternation_and_escaped_dot(r"^(?:[0-9]+:[0-9]+\.[0-9]+|[0-9]+:GR-[0-9]+)(?![\s\S])")]
#[case::unanchored_prefix("^x-")]
#[case::end_anchor_inside_a_lookahead(r"^(?!core(?![\s\S]))[a-z]+(?![\s\S])")]
#[case::escaped_backslashes(r"^(?:[^\\]|\\[^uU])*\\[uU]")]
#[case::lookahead_and_unicode_escapes(r"^(?!/)[^\u0000-\u001F]{1,64}(?![\s\S])")]
fn a_portable_pattern_passes(#[case] pattern: &str) {
    assert_eq!(violations(pattern), []);
}

#[rstest]
#[case::bare_dollar(r"^[a-z]+$", Violation::BareDollar)]
#[case::dollar_inside_a_group(r"^(?!(.*/)?\.\.(/|$))", Violation::BareDollar)]
#[case::digit(r"^\d+(?![\s\S])", Violation::Shorthand('d'))]
#[case::word(r"^\w+(?![\s\S])", Violation::Shorthand('w'))]
#[case::space(r"\S", Violation::Shorthand('S'))]
#[case::end_anchor_not_last(r"^a(?![\s\S])b", Violation::Shorthand('s'))]
#[case::unicode_property(r"^\p{L}+(?![\s\S])", Violation::UnicodeProperty)]
#[case::inline_flag(r"(?i)^a(?![\s\S])", Violation::InlineFlag)]
#[case::lookbehind(r"(?<!\\)\\u", Violation::Lookbehind)]
#[case::named_group(r"^(?<n>a)(?![\s\S])", Violation::NamedGroup)]
#[case::end_of_text(r"^a\z", Violation::Boundary('z'))]
#[case::dot(r"^.+(?![\s\S])", Violation::Dot)]
#[case::backreference(r"^(a)\1(?![\s\S])", Violation::Backreference)]
#[case::bracket_first(r"^[]a]+(?![\s\S])", Violation::BracketFirst)]
fn a_construct_outside_the_subset_is_flagged(#[case] pattern: &str, #[case] want: Violation) {
    assert!(
        violations(pattern).contains(&want),
        "{:?}",
        violations(pattern)
    );
}
