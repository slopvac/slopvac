//! Limits, escaping, credentials and construction invariants of the lock model.

use slopvac_plugins::lock::{
    LockedPack, Lockfile, MAX_API_VERSION, MAX_LOCK_BYTES, MAX_PACKS, MAX_REQUIRES,
    MAX_STRING_BYTES, NotBefore, PackId, PackKind, ResolvedPack, Source, Version, diff,
    parse_version_req,
};

const D1: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const D2: &str = "sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn pack_text(id: &str, source: &str, extra: &str) -> String {
    format!(
        "\n[[plugins]]\nid = \"{id}\"\nkind = \"data\"\nversion = \"1.0.0\"\nslopvac_api = 1\nsource = '{source}'\ndigest = \"{D1}\"\n{extra}"
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
    let base = "schema_version = 3\nplugins = []\n";
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
        "/plugins"
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
        "/plugins/0/requires"
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
        "/plugins/0/source"
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
        "/plugins/0/requires/0"
    );
}

// ---- escaping -------------------------------------------------------------

/// The characters a message must never carry raw, listed apart from the
/// implementation: C0, DEL, C1, bidirectional controls, zero-width characters,
/// the BOM and the line and paragraph separators.
fn is_hostile(c: char) -> bool {
    matches!(c,
        '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}' | '\u{61c}' | '\u{200b}'..='\u{200f}'
        | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}')
}

fn assert_plain(message: &str) {
    assert!(
        !message.chars().any(is_hostile),
        "raw hostile character in {message:?}"
    );
}

/// (the character as a TOML escape, the escape a message must show)
const HOSTILE: [(&str, &str); 12] = [
    ("\\u001b", "\\u{1b}"),
    ("\\u007f", "\\u{7f}"),
    ("\\u0085", "\\u{85}"),
    ("\\u009b", "\\u{9b}"),
    ("\\u061c", "\\u{61c}"),
    ("\\u200b", "\\u{200b}"),
    ("\\u200f", "\\u{200f}"),
    ("\\u2028", "\\u{2028}"),
    ("\\u202e", "\\u{202e}"),
    ("\\u2060", "\\u{2060}"),
    ("\\u2066", "\\u{2066}"),
    ("\\ufeff", "\\u{feff}"),
];

#[test]
fn every_hostile_class_is_escaped_in_an_invalid_id_error() {
    for (toml_escape, shown) in HOSTILE {
        let text =
            lock_text("packs/x", "").replace("id = \"p\"", &format!("id = \"a{toml_escape}b\""));
        let err = Lockfile::parse(&text).unwrap_err();
        assert_plain(&err.message);
        assert!(
            err.message.contains(&format!("a{shown}b")),
            "{shown}: {err}"
        );
    }
}

#[test]
fn every_hostile_class_is_escaped_in_an_unknown_key_error() {
    for (toml_escape, shown) in HOSTILE {
        let text = format!("schema_version = 3\nplugins = []\n\"k{toml_escape}\" = 1\n");
        let err = Lockfile::parse(&text).unwrap_err();
        assert_plain(&err.message);
        assert!(err.message.contains(&format!("k{shown}")), "{shown}: {err}");
    }
}

#[test]
fn every_hostile_class_is_escaped_in_an_invalid_not_before_error() {
    for (toml_escape, shown) in HOSTILE {
        let sig = format!(
            "[plugins.signature]\nkey_id = \"A1B2C3D4E5F60718\"\nnot_before = \"2026-11-01T00:00:00{toml_escape}Z\"\n"
        );
        let err = Lockfile::parse(&lock_text("packs/x", &sig)).unwrap_err();
        assert_eq!(err.pointer, "/plugins/0/signature/not_before");
        assert_plain(&err.message);
        assert!(
            err.message.contains(&format!("00{shown}Z")),
            "{shown}: {err}"
        );
    }
}

#[test]
fn not_before_value_error_is_escaped_when_parsed_directly() {
    let err = "2026\u{202e}-11-01T00:00:00Z"
        .parse::<NotBefore>()
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "`2026\\u{202e}-11-01T00:00:00Z` is not a UTC time (`YYYY-MM-DDTHH:MM:SSZ`, calendar-valid)"
    );
}

#[test]
fn a_long_rejected_value_is_cut_in_the_error() {
    let text = lock_text("packs/x", "").replace(D1, &"f".repeat(100_000));
    let err = Lockfile::parse(&text).unwrap_err();
    assert_eq!(err.pointer, "/plugins/0/digest");
    assert!(err.message.len() < 700, "{} bytes", err.message.len());
    assert!(err.message.ends_with("..."));
}

#[test]
fn newline_in_a_rejected_value_cannot_forge_a_second_line() {
    let text = lock_text("packs/x", "").replace("id = \"p\"", "id = \"a\\nerror: forged\"");
    let err = Lockfile::parse(&text).unwrap_err();
    assert!(!err.to_string().contains('\n'), "{err:?}");
}

#[test]
fn sources_of_every_kind_reject_hostile_characters() {
    for c in [
        "\u{1b}", "\u{7f}", "\u{85}", "\u{202e}", "\u{200b}", "\u{feff}", "\u{2066}",
    ] {
        for source in [
            format!("oci://ghcr.io/a{c}b:1"),
            format!("https://example.org/a{c}b"),
            format!("git+https://example.org/a{c}b@{COMMIT}"),
            format!("packs/a{c}b"),
        ] {
            assert!(source.parse::<Source>().is_err(), "accepted {source:?}");
        }
    }
}

/// Classes the shared sanitiser covers beyond the older lock list: the soft
/// hyphen (Cf), fillers and a reserved code point that Unicode marks
/// `Default_Ignorable` (U+2065), tag characters, private use and noncharacters.
#[test]
fn sources_reject_format_ignorable_private_use_and_noncharacter_code_points() {
    for c in [
        "\u{ad}",
        "\u{34f}",
        "\u{3164}",
        "\u{fe0f}",
        "\u{2065}",
        "\u{e0041}",
        "\u{e000}",
        "\u{fdd0}",
        "\u{ffff}",
        "\u{10ffff}",
    ] {
        for source in [
            format!("oci://ghcr.io/a{c}b:1"),
            format!("https://example.org/a{c}b"),
            format!("packs/a{c}b"),
        ] {
            assert!(source.parse::<Source>().is_err(), "accepted {source:?}");
        }
    }
}

#[test]
fn a_source_displays_as_the_text_it_was_parsed_from() {
    let git = format!("git+https://example.org/a@{COMMIT}");
    for text in [
        "oci://ghcr.io/a/b:1",
        "https://example.org/a.tgz",
        git.as_str(),
        "packs/local",
    ] {
        assert_eq!(text.parse::<Source>().unwrap().to_string(), text);
    }
}

// ---- the constructor enforces the reader's limits ------------------------------

#[test]
fn pack_id_and_source_parsers_apply_the_string_cap() {
    assert!("a".repeat(MAX_STRING_BYTES).parse::<PackId>().is_ok());
    assert!("a".repeat(MAX_STRING_BYTES + 1).parse::<PackId>().is_err());
    assert!("a".repeat(MAX_STRING_BYTES).parse::<Source>().is_ok());
    assert!("a".repeat(MAX_STRING_BYTES + 1).parse::<Source>().is_err());
}

#[test]
fn new_rejects_a_version_over_the_string_cap() {
    let mut pack = locked("p");
    pack.version = Version::parse(&format!("1.0.0-{}", "a".repeat(MAX_STRING_BYTES))).unwrap();
    assert_eq!(
        Lockfile::new(vec![pack]).unwrap_err().pointer,
        "/plugins/0/version"
    );
}

#[test]
fn a_file_with_a_version_over_the_string_cap_is_refused() {
    let long = format!("1.0.0-{}", "a".repeat(MAX_STRING_BYTES));
    let text = lock_text("packs/x", "").replace("\"1.0.0\"", &format!("\"{long}\""));
    assert_eq!(
        Lockfile::parse(&text).unwrap_err().pointer,
        "/plugins/0/version"
    );
}

/// 4,094 quotes make a valid source that is escaped to twice its size on write.
fn quoted_source() -> Source {
    format!("p/{}", "\"".repeat(MAX_STRING_BYTES - 2))
        .parse()
        .unwrap()
}

fn packs_with_quoted_sources(n: usize) -> Vec<LockedPack> {
    (0..n)
        .map(|i| {
            let mut pack = locked(&format!("p{i}"));
            pack.source = quoted_source();
            pack
        })
        .collect()
}

#[test]
fn new_rejects_entries_whose_canonical_text_passes_the_byte_cap() {
    // 150 x 8 KiB of escaped quotes is about 1.2 MB on output.
    let err = Lockfile::new(packs_with_quoted_sources(150)).unwrap_err();
    assert_eq!(err.pointer, "");
    assert!(err.message.contains("canonical"), "{err}");
}

#[test]
fn a_lock_under_the_canonical_cap_is_accepted_and_re_reads() {
    let lock = Lockfile::new(packs_with_quoted_sources(100)).unwrap();
    assert!(lock.to_toml().len() <= MAX_LOCK_BYTES);
    assert_eq!(Lockfile::parse(&lock.to_toml()).unwrap(), lock);
}

#[test]
fn api_version_stops_at_the_largest_toml_integer() {
    assert_eq!(MAX_API_VERSION, u64::try_from(i64::MAX).unwrap());
    let mut pack = locked("p");
    pack.slopvac_api = MAX_API_VERSION;
    let lock = Lockfile::new(vec![pack.clone()]).unwrap();
    assert_eq!(Lockfile::parse(&lock.to_toml()).unwrap(), lock);
    pack.slopvac_api = MAX_API_VERSION + 1;
    assert_eq!(
        Lockfile::new(vec![pack]).unwrap_err().pointer,
        "/plugins/0/slopvac_api"
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
        assert_eq!(err.pointer, "/plugins/0/source", "{source}");
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
        "/plugins/0/version"
    );
}

#[test]
fn new_rejects_api_zero() {
    let mut pack = locked("p");
    pack.slopvac_api = 0;
    assert_eq!(
        Lockfile::new(vec![pack]).unwrap_err().pointer,
        "/plugins/0/slopvac_api"
    );
}

#[test]
fn new_rejects_repeated_requires() {
    let mut pack = locked("p");
    let dep: PackId = "dep".parse().unwrap();
    pack.requires = vec![dep.clone(), dep];
    assert_eq!(
        Lockfile::new(vec![pack]).unwrap_err().pointer,
        "/plugins/0/requires/1"
    );
}

#[test]
fn new_rejects_too_many_packs() {
    let packs = (0..=MAX_PACKS).map(|i| locked(&format!("p{i}"))).collect();
    assert_eq!(Lockfile::new(packs).unwrap_err().pointer, "/plugins");
}

#[test]
fn duplicate_id_pointer_uses_the_position_in_the_input() {
    // Sorted, `z z a` would put the second `z` at index 2.
    let packs = vec![locked("z"), locked("z"), locked("a")];
    assert_eq!(Lockfile::new(packs).unwrap_err().pointer, "/plugins/1/id");
}

#[test]
fn duplicate_id_in_a_file_names_the_second_entry_in_file_order() {
    let text = format!(
        "schema_version = 3\n{}{}{}",
        pack_text("z", "packs/x", ""),
        pack_text("z", "packs/x", ""),
        pack_text("a", "packs/x", "")
    );
    assert_eq!(Lockfile::parse(&text).unwrap_err().pointer, "/plugins/1/id");
}

#[test]
fn a_constructed_lock_writes_text_the_reader_accepts() {
    let mut a = locked("b");
    a.requires = vec!["zeta".parse().unwrap(), "core".parse().unwrap()];
    let lock = Lockfile::new(vec![a, locked("a")]).unwrap();
    assert_eq!(Lockfile::parse(&lock.to_toml()).unwrap(), lock);
}

// ---- the built-in core is never locked ----------------------------------------

#[test]
fn new_refuses_an_entry_with_the_id_core() {
    let err = Lockfile::new(vec![locked("a"), locked("core")]).unwrap_err();
    assert_eq!(err.pointer, "/plugins/1/id");
}

#[test]
fn parse_refuses_an_entry_with_the_id_core() {
    let err = Lockfile::parse(&format!(
        "schema_version = 3\n{}",
        pack_text("core", "packs/x", "")
    ))
    .unwrap_err();
    assert_eq!(err.pointer, "/plugins/0/id");
}

#[test]
fn an_entry_may_require_the_built_in_core_and_round_trips() {
    let mut pack = locked("p");
    pack.requires = vec!["core".parse().unwrap()];
    let lock = Lockfile::new(vec![pack]).unwrap();
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
