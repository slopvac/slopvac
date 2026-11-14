//! Limits, escaping, credentials and construction invariants of the lock model.

use slopvac_plugins::lock::{
    LockedPack, Lockfile, MAX_LOCK_BYTES, MAX_PACKS, MAX_REQUIRES, MAX_STRING_BYTES, PackId,
    PackKind, ResolvedPack, Source, Version, diff, parse_version_req,
};

const D1: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const D2: &str = "sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn pack_text(id: &str, source: &str, extra: &str) -> String {
    format!(
        "\n[[packs]]\nid = \"{id}\"\nkind = \"data\"\nversion = \"1.0.0\"\nslopvac_api = 1\nsource = '{source}'\ndigest = \"{D1}\"\n{extra}"
    )
}

fn lock_text(source: &str, extra: &str) -> String {
    format!("schema_version = 3\n{}", pack_text("p", source, extra))
}

fn locked(id: &str) -> LockedPack {
    LockedPack {
        id: id.parse().unwrap(),
        kind: PackKind::Data,
        version: "1.0.0".parse().unwrap(),
        slopvac_api: 1,
        source: "packs/x".parse().unwrap(),
        digest: D1.parse().unwrap(),
        signature: None,
        requires: Vec::new(),
    }
}

// ---- limits ---------------------------------------------------------------

#[test]
fn input_at_the_byte_cap_is_read_and_one_byte_more_is_refused() {
    let base = "schema_version = 3\npacks = []\n";
    let pad = |total: usize| format!("{base}#{}\n", "a".repeat(total - base.len() - 2));
    assert!(Lockfile::parse(&pad(MAX_LOCK_BYTES)).is_ok());
    let err = Lockfile::parse(&pad(MAX_LOCK_BYTES + 1)).unwrap_err();
    assert_eq!(err.pointer, "");
    assert!(err.message.contains("larger than"), "{err}");
}

#[test]
fn pack_count_cap() {
    let many = |n: usize| {
        let packs: String = (0..n)
            .map(|i| pack_text(&format!("p{i}"), "packs/x", ""))
            .collect();
        format!("schema_version = 3\n{packs}")
    };
    assert_eq!(
        Lockfile::parse(&many(MAX_PACKS)).unwrap().packs().len(),
        MAX_PACKS
    );
    assert_eq!(
        Lockfile::parse(&many(MAX_PACKS + 1)).unwrap_err().pointer,
        "/packs"
    );
}

#[test]
fn requires_count_cap_holds_without_quadratic_work() {
    let requires = |n: usize| {
        let ids: Vec<String> = (0..n).map(|i| format!("\"r{i}\"")).collect();
        lock_text("packs/x", &format!("requires = [{}]\n", ids.join(", ")))
    };
    assert!(Lockfile::parse(&requires(MAX_REQUIRES)).is_ok());
    assert_eq!(
        Lockfile::parse(&requires(MAX_REQUIRES + 1))
            .unwrap_err()
            .pointer,
        "/packs/0/requires"
    );
    // 30,000 entries is about 200 KB of input: the cap answers at once.
    assert!(Lockfile::parse(&requires(30_000)).is_err());
}

#[test]
fn string_length_cap() {
    let source = |n: usize| lock_text(&"a".repeat(n), "");
    assert!(Lockfile::parse(&source(MAX_STRING_BYTES)).is_ok());
    assert_eq!(
        Lockfile::parse(&source(MAX_STRING_BYTES + 1))
            .unwrap_err()
            .pointer,
        "/packs/0/source"
    );
}

#[test]
fn string_length_cap_covers_requires_items() {
    let text = lock_text(
        "packs/x",
        &format!("requires = [\"{}\"]\n", "a".repeat(MAX_STRING_BYTES + 1)),
    );
    assert_eq!(
        Lockfile::parse(&text).unwrap_err().pointer,
        "/packs/0/requires/0"
    );
}

// ---- escaping -------------------------------------------------------------

fn assert_plain(message: &str) {
    assert!(
        !message
            .chars()
            .any(|c| c.is_control() || ('\u{202a}'..='\u{202e}').contains(&c)),
        "raw control characters in {message:?}"
    );
}

#[test]
fn unknown_key_with_escape_sequence_is_escaped_in_the_error() {
    let text = "schema_version = 3\npacks = []\n\"a\\u001b[31mred\" = 1\n";
    let err = Lockfile::parse(text).unwrap_err();
    assert_plain(&err.message);
    assert!(err.message.contains("a\\u{1b}[31mred"), "{err}");
}

#[test]
fn invalid_value_with_bidi_override_is_escaped_in_the_error() {
    let text = lock_text("packs/x", "").replace("id = \"p\"", "id = \"a\\u202Eb\"");
    let err = Lockfile::parse(&text).unwrap_err();
    assert_plain(&err.message);
    assert!(err.message.contains("a\\u{202e}b"), "{err}");
}

#[test]
fn newline_in_a_rejected_value_cannot_forge_a_second_line() {
    let text = lock_text("packs/x", "").replace("id = \"p\"", "id = \"a\\nerror: forged\"");
    let err = Lockfile::parse(&text).unwrap_err();
    assert!(!err.to_string().contains('\n'), "{err:?}");
}

#[test]
fn drift_message_escapes_control_characters_in_sources() {
    let text = lock_text("X", "").replace("'X'", "\"packs/a\\u001b[2Jb\"");
    let lock = Lockfile::parse(&text).unwrap();
    let resolved = ResolvedPack {
        id: "p".parse().unwrap(),
        version: Version::new(1, 0, 0),
        source: "packs/c\u{202e}d".parse().unwrap(),
        digest: D1.parse().unwrap(),
    };
    let message = diff(&lock, &[resolved]).to_string();
    assert_plain(message.trim_end());
    assert_eq!(
        message,
        "changed: p source packs/a\\u{1b}[2Jb -> packs/c\\u{202e}d\n"
    );
}

#[test]
fn version_requirement_error_escapes_control_characters() {
    let err = parse_version_req("^1\u{1b}[31m").unwrap_err();
    assert_plain(&err.message);
}

// ---- credentials ----------------------------------------------------------

#[test]
fn sources_with_user_info_are_rejected() {
    let git = format!("git+https://tok@github.com/a/b@{COMMIT}");
    for source in [
        "https://user:pw@example.org/p.tar.gz",
        "https://token@example.org/p.tar.gz",
        "oci://user:pw@ghcr.io/a/b:1",
        git.as_str(),
    ] {
        let err = Lockfile::parse(&lock_text(source, "")).unwrap_err();
        assert_eq!(err.pointer, "/packs/0/source", "{source}");
        assert!(!err.message.contains("pw"), "{err}");
    }
}

#[test]
fn an_at_sign_after_the_authority_is_not_user_info() {
    for source in [
        "https://example.org/dl/p@1.tar.gz",
        "https://example.org/?x=a@b",
        "oci://ghcr.io/a/b@sha256:0123",
    ] {
        assert!(source.parse::<Source>().is_ok(), "{source}");
    }
}

// ---- construction keeps the round-trip invariant -----------------------------

#[test]
fn new_rejects_build_metadata_in_a_version() {
    let mut pack = locked("p");
    pack.version = Version::parse("1.0.0+build").unwrap();
    assert_eq!(
        Lockfile::new(vec![pack]).unwrap_err().pointer,
        "/packs/0/version"
    );
}

#[test]
fn new_rejects_api_zero() {
    let mut pack = locked("p");
    pack.slopvac_api = 0;
    assert_eq!(
        Lockfile::new(vec![pack]).unwrap_err().pointer,
        "/packs/0/slopvac_api"
    );
}

#[test]
fn new_rejects_repeated_requires() {
    let mut pack = locked("p");
    let dep: PackId = "dep".parse().unwrap();
    pack.requires = vec![dep.clone(), dep];
    assert_eq!(
        Lockfile::new(vec![pack]).unwrap_err().pointer,
        "/packs/0/requires/1"
    );
}

#[test]
fn new_rejects_too_many_packs() {
    let packs = (0..=MAX_PACKS).map(|i| locked(&format!("p{i}"))).collect();
    assert_eq!(Lockfile::new(packs).unwrap_err().pointer, "/packs");
}

#[test]
fn duplicate_id_pointer_uses_the_position_in_the_input() {
    // Sorted, `z z a` would put the second `z` at index 2.
    let packs = vec![locked("z"), locked("z"), locked("a")];
    assert_eq!(Lockfile::new(packs).unwrap_err().pointer, "/packs/1/id");
}

#[test]
fn duplicate_id_in_a_file_names_the_second_entry_in_file_order() {
    let text = format!(
        "schema_version = 3\n{}{}{}",
        pack_text("z", "packs/x", ""),
        pack_text("z", "packs/x", ""),
        pack_text("a", "packs/x", "")
    );
    assert_eq!(Lockfile::parse(&text).unwrap_err().pointer, "/packs/1/id");
}

#[test]
fn a_constructed_lock_writes_text_the_reader_accepts() {
    let mut a = locked("b");
    a.requires = vec!["zeta".parse().unwrap(), "core".parse().unwrap()];
    let lock = Lockfile::new(vec![a, locked("a")]).unwrap();
    assert_eq!(Lockfile::parse(&lock.to_toml()).unwrap(), lock);
}

// ---- API version has no maximum ---------------------------------------------

#[test]
fn api_version_above_u32_is_accepted_and_round_trips() {
    let text = lock_text("packs/x", "").replace("slopvac_api = 1", "slopvac_api = 4294967296");
    let lock = Lockfile::parse(&text).unwrap();
    assert_eq!(lock.packs()[0].slopvac_api, 4_294_967_296);
    assert_eq!(Lockfile::parse(&lock.to_toml()).unwrap(), lock);
}

// ---- duplicate ids in the resolved set -----------------------------------------

fn resolved(digest: &str) -> ResolvedPack {
    ResolvedPack {
        id: "p".parse().unwrap(),
        version: Version::new(1, 0, 0),
        source: "packs/x".parse().unwrap(),
        digest: digest.parse().unwrap(),
    }
}

#[test]
fn the_last_resolved_entry_wins_when_an_id_repeats() {
    let lock = Lockfile::parse(&lock_text("packs/x", "")).unwrap(); // digest D1
    let drifted = diff(&lock, &[resolved(D1), resolved(D2)]);
    assert_eq!(drifted.changed.len(), 1);
    assert_eq!(
        drifted.changed[0]
            .digest
            .as_ref()
            .unwrap()
            .resolved
            .as_str(),
        D2
    );
    assert!(diff(&lock, &[resolved(D2), resolved(D1)]).is_empty());
}
