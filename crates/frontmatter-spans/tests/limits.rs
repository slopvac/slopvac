//! Resource limits: hostile input returns an error, never an abort or a runaway allocation.

use std::time::{Duration, Instant};

use frontmatter_spans::{FrontMatter, Limits, Value};

fn yaml(body: &str) -> String {
    format!("---\n{body}\n---\n")
}

fn message(doc: &str, limits: &Limits) -> String {
    FrontMatter::parse_with(doc, limits)
        .expect_err("should exceed a limit")
        .message
}

fn limits_with_depth(max_depth: usize) -> Limits {
    Limits::default().with_max_depth(max_depth)
}

#[test]
fn defaults_are_documented_values() {
    let d = Limits::default();
    assert_eq!(d.max_depth, 128);
    assert_eq!(d.max_alias_bytes, 1 << 20);
    assert_eq!(d.max_input_bytes, 16 << 20);
}

// ---- depth ----------------------------------------------------------------------------

#[test]
fn deep_block_sequences_are_an_error_not_an_abort() {
    let doc = yaml(&"- ".repeat(200_000));
    assert!(message(&doc, &Limits::default()).contains("too deep"));
}

#[test]
fn deep_indented_block_mappings_are_an_error() {
    let body: Vec<String> = (0..300).map(|i| format!("{}k:", " ".repeat(i))).collect();
    let doc = yaml(&body.join("\n"));
    assert!(message(&doc, &Limits::default()).contains("too deep"));
}

#[test]
fn deep_flow_sequences_are_an_error() {
    let doc = yaml(&format!("a: {}", "[".repeat(100_000)));
    assert!(FrontMatter::parse(&doc).is_err());
    let balanced = yaml(&format!("a: {}1{}", "[".repeat(1_000), "]".repeat(1_000)));
    assert!(FrontMatter::parse(&balanced).is_err());
}

#[test]
fn deep_json_is_an_error() {
    let doc = format!("{{\"a\":{}1{}}}", "[".repeat(5_000), "]".repeat(5_000));
    assert!(message(&doc, &Limits::default()).contains("too deep"));
}

#[test]
fn depth_limit_is_configurable_and_counts_collections_from_the_root() {
    // Root table = depth 1; `{b: ..}` = 2; `{c: 1}` = 3.
    let y = yaml("a: {b: {c: 1}}");
    let j = "{\"a\": {\"b\": {\"c\": 1}}}";
    for (doc, name) in [(y.as_str(), "yaml"), (j, "json")] {
        assert!(
            FrontMatter::parse_with(doc, &limits_with_depth(3)).is_ok(),
            "{name} at the limit"
        );
        assert!(
            message(doc, &limits_with_depth(2)).contains("too deep"),
            "{name} past the limit"
        );
    }
}

// ---- aliases --------------------------------------------------------------------------

fn alias_doc(string_len: usize, refs: usize) -> String {
    let uses: Vec<String> = (0..refs).map(|i| format!("r{i}: *big")).collect();
    yaml(&format!(
        "big: &big {}\n{}",
        "x".repeat(string_len),
        uses.join("\n")
    ))
}

#[test]
fn alias_expansion_counts_bytes_not_nodes() {
    // 64 KiB anchor referenced 100 times is 6.4 MB of clones, far past the 1 MiB default.
    let doc = alias_doc(64 * 1024, 100);
    assert!(message(&doc, &Limits::default()).contains("alias"));
}

#[test]
fn alias_limit_is_configurable() {
    let doc = alias_doc(1_000, 5);
    let tight = Limits::default().with_max_alias_bytes(4_000);
    assert!(message(&doc, &tight).contains("alias"));
    let roomy = Limits::default().with_max_alias_bytes(6_000);
    assert!(FrontMatter::parse_with(&doc, &roomy).is_ok());
}

#[test]
fn nested_alias_bomb_is_an_error() {
    let mut lines = vec!["a0: &a0 [x, x, x, x, x, x, x, x, x, x]".to_owned()];
    for i in 1..12 {
        let refs = vec![format!("*a{}", i - 1); 10].join(", ");
        lines.push(format!("a{i}: &a{i} [{refs}]"));
    }
    assert!(FrontMatter::parse(&yaml(&lines.join("\n"))).is_err());
}

#[test]
fn small_aliases_expand_with_the_span_of_the_use() {
    let doc = yaml("a: &x [1, 2]\nb: *x");
    let fm = FrontMatter::parse(&doc).unwrap().unwrap();
    assert_eq!(fm.get("a").unwrap().value, fm.get("b").unwrap().value);
    let b = fm.get("b").unwrap();
    assert_eq!(&doc[b.span.range.clone()], "*x");
}

#[test]
fn scalar_alias_as_a_key_uses_the_anchored_text() {
    let doc = yaml("a: &k name\n*k : 1");
    let fm = FrontMatter::parse(&doc).unwrap().unwrap();
    assert_eq!(fm.get("name").unwrap().value, Value::Integer(1));
}

#[test]
fn duplicate_yaml_keys_are_an_error_at_the_second_key() {
    let doc = yaml("a: 1\nb: 2\na: 3");
    let err = FrontMatter::parse(&doc).unwrap_err();
    let span = err.span.clone().unwrap();
    assert_eq!(span.start.line, 4);
    assert!(err.message.contains("duplicate"), "{err}");
}

// ---- input size -----------------------------------------------------------------------

#[test]
fn oversized_blocks_are_rejected_before_parsing() {
    let limits = Limits::default().with_max_input_bytes(16);
    assert!(FrontMatter::parse_with("---\na: 1\n---\n", &limits).is_ok());
    for doc in [
        "---\nkey: a-long-value-here\n---\n",
        "+++\nkey = \"a-long-value\"\n+++\n",
        "{\"key\": \"a-long-value\"}",
    ] {
        let err = FrontMatter::parse_with(doc, &limits).unwrap_err();
        assert!(err.message.contains("larger than"), "{doc:?}: {err}");
        assert_eq!(err.span, None);
    }
}

// ---- linear cost ----------------------------------------------------------------------

/// A quadratic span computation takes minutes at this size; linear takes well under a second.
const LINEAR_BOUND: Duration = Duration::from_secs(20);
const ONE_MIB: usize = 1 << 20;

#[test]
fn flat_single_line_input_parses_in_linear_time() {
    let n = ONE_MIB / 2;
    let json = format!("{{\"a\": [{}1]}}", "1,".repeat(n));
    let toml = format!("+++\na = [{}1]\n+++\n", "1,".repeat(n));
    let yml = format!("---\na: [{}1]\n---\n", "1,".repeat(n));
    let utf8 = format!("{{\"a\": [{}\"é\"]}}", "\"é\",".repeat(n / 4));
    for (name, doc) in [
        ("json", json),
        ("toml", toml),
        ("yaml", yml),
        ("json-utf8", utf8),
    ] {
        let start = Instant::now();
        let fm = FrontMatter::parse(&doc).unwrap().unwrap();
        assert!(
            matches!(fm.get("a").unwrap().value, Value::Array(_)),
            "{name}"
        );
        assert!(
            start.elapsed() < LINEAR_BOUND,
            "{name} took {:?}",
            start.elapsed()
        );
    }
}
