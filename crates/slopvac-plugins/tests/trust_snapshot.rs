//! `parse_store`: the trust store rules applied to bytes the caller read itself.

use std::path::Path;

use slopvac_plugins::trust::{MAX_STORE_FILE_BYTES, TrustError, TrustStore, parse_store};

const PATH: &str = "trust.json";

fn parse(raw: &[u8]) -> Result<usize, TrustError> {
    parse_store(raw, Path::new(PATH)).map(|s| s.records.len() + s.pins.len())
}

#[test]
fn an_empty_versioned_store_parses() {
    assert_eq!(parse(br#"{"schema_version":1}"#).unwrap(), 0);
}

#[test]
fn bytes_written_by_the_store_parse_back_to_the_same_counts() {
    let dir = tempfile::tempdir().unwrap();
    let store = TrustStore::open(dir.path());
    let key = minisign::KeyPair::generate_unencrypted_keypair()
        .unwrap()
        .pk
        .to_base64();
    store
        .pin_key("registry.example", "acme", &key, "2026-11-15T00:00:00Z")
        .unwrap();
    let raw = std::fs::read(dir.path().join("trust.json")).unwrap();
    let snapshot = parse_store(&raw, Path::new(PATH)).unwrap();
    assert_eq!(snapshot.pins.len(), 1);
    assert_eq!(snapshot.pins[0].namespace, "acme");
    assert_eq!(snapshot.records.len(), 0);
}

#[test]
fn extension_keys_are_ignored_and_other_unknown_keys_refused() {
    assert!(parse(br#"{"schema_version":1,"x-note":true}"#).is_ok());
    assert!(matches!(
        parse(br#"{"schema_version":1,"surprise":1}"#),
        Err(TrustError::Corrupt { .. })
    ));
}

#[test]
fn another_schema_version_and_bad_json_are_corrupt() {
    for raw in [&br#"{"schema_version":2}"#[..], b"{broken", b"", b"[]"] {
        assert!(
            matches!(parse(raw), Err(TrustError::Corrupt { .. })),
            "{raw:?}"
        );
    }
}

/// Valid store JSON of exactly `len` bytes, padded inside an ignored `x-` key.
fn store_of_len(len: usize) -> Vec<u8> {
    let head = br#"{"schema_version":1,"x-pad":""#;
    let tail = br#""}"#;
    let pad = len - head.len() - tail.len();
    let mut raw = head.to_vec();
    raw.extend(std::iter::repeat_n(b'a', pad));
    raw.extend_from_slice(tail);
    assert_eq!(raw.len(), len);
    raw
}

#[test]
fn a_valid_store_exactly_at_the_limit_is_accepted() {
    let len = usize::try_from(MAX_STORE_FILE_BYTES).unwrap();
    assert_eq!(parse(&store_of_len(len)).unwrap(), 0);
}

#[test]
fn a_valid_store_one_byte_over_the_limit_is_refused_for_its_size() {
    let len = usize::try_from(MAX_STORE_FILE_BYTES).unwrap() + 1;
    match parse(&store_of_len(len)) {
        Err(TrustError::Corrupt { why, .. }) => assert!(why.contains("larger than"), "{why}"),
        other => panic!("expected a size refusal, got {other:?}"),
    }
}
