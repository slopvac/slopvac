//! Behavior of the pack index through its public interface.
#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use semver::Version;
use slopvac_plugins::index::{
    Index, IndexEntry, Listing, MAX_DESCRIPTION_CHARS, MAX_ENTRIES, MAX_INDEX_BYTES, PackKind,
    PublisherKey, build_index, search,
};

fn key() -> PublisherKey {
    PublisherKey {
        key_id: "0123456789ABCDEF".into(),
        fingerprint: "a".repeat(64),
    }
}

fn listing(namespace: &str, id: &str, name: &str, description: &str) -> Listing {
    Listing {
        id: id.into(),
        namespace: namespace.into(),
        name: Some(name.into()),
        description: description.into(),
        version: Version::new(1, 0, 0),
        kind: PackKind::Data,
        slopvac_api: 1,
        source: format!("oci://ghcr.io/{namespace}/{id}"),
        publisher_key: key(),
    }
}

fn entry(namespace: &str, id: &str) -> IndexEntry {
    listing(namespace, id, id, "A pack.").into()
}

// ---- conformance corpus (shared with the JSON Schema harness) ----

fn corpus(kind: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas/v3/conformance/pack-index")
        .join(kind)
}

fn files(dir: &PathBuf) -> Vec<PathBuf> {
    let mut found: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    found.sort();
    assert!(!found.is_empty(), "no corpus files in {}", dir.display());
    found
}

#[test]
fn every_valid_conformance_document_parses() {
    for path in files(&corpus("valid")) {
        let result = Index::parse(&fs::read(&path).unwrap());
        assert!(result.is_ok(), "{}: {result:?}", path.display());
    }
}

#[test]
fn every_invalid_conformance_document_fails_at_or_below_its_pointer() {
    let dir = corpus("invalid");
    let cases: toml::Table =
        toml::from_str(&fs::read_to_string(dir.join("cases.toml")).unwrap()).unwrap();
    for path in files(&dir) {
        let name = path.file_name().unwrap().to_str().unwrap();
        let want = cases[name].as_str().unwrap();
        let err = Index::parse(&fs::read(&path).unwrap()).expect_err(name);
        // The schema reports a missing key at the object; the reader names the key.
        assert!(
            err.pointer.starts_with(want),
            "{name}: reader pointer `{}` is not under `{want}`",
            err.pointer
        );
    }
}

// ---- format ----

#[test]
fn x_keys_are_ignored_and_other_unknown_keys_are_errors() {
    let ok = r#"{"schema_version":3,"x-job":1,"packs":[]}"#;
    assert_eq!(Index::parse(ok.as_bytes()).unwrap().entries().len(), 0);
    let bad = r#"{"schema_version":3,"packs":[],"extra":1}"#;
    assert_eq!(Index::parse(bad.as_bytes()).unwrap_err().pointer, "");
}

#[test]
fn a_repeated_namespace_and_id_pair_is_an_error_but_the_same_id_in_two_namespaces_is_not() {
    let twice = Index::new(vec![entry("acme", "a"), entry("acme", "a")]).unwrap_err();
    assert_eq!(twice.pointer, "/packs/1/id");
    let two_namespaces = Index::new(vec![entry("acme", "a"), entry("beta", "a")]);
    assert_eq!(two_namespaces.unwrap().entries().len(), 2);
}

// ---- build ----

#[test]
fn build_sorts_by_namespace_then_id_and_ignores_input_order() {
    let forward = vec![
        listing("beta", "a", "A", "d"),
        listing("acme", "z", "Z", "d"),
        listing("acme", "b", "B", "d"),
    ];
    let mut backward = forward.clone();
    backward.reverse();
    let one = build_index(forward).unwrap();
    let two = build_index(backward).unwrap();
    let order: Vec<_> = one
        .entries()
        .iter()
        .map(|e| format!("{}/{}", e.namespace, e.id))
        .collect();
    assert_eq!(order, ["acme/b", "acme/z", "beta/a"]);
    assert_eq!(one.to_json(), two.to_json());
}

#[test]
fn build_uses_the_id_as_name_when_the_manifest_gives_none() {
    let mut l = listing("acme", "plain", "ignored", "d");
    l.name = None;
    assert_eq!(build_index([l]).unwrap().entries()[0].name, "plain");
}

#[test]
fn output_round_trips_and_has_a_fixed_shape() {
    let mut plugin = listing("beta", "bridge", "Bridge", "Runs linters.");
    plugin.kind = PackKind::Plugin;
    plugin.version = Version::parse("0.3.1-rc.1").unwrap();
    plugin.slopvac_api = 2;
    plugin.source = format!(
        "git+https://github.com/beta/bridge@{}",
        "0123456789".repeat(4)
    );
    let index = build_index([plugin]).unwrap();
    let json = index.to_json();
    assert_eq!(Index::parse(json.as_bytes()).unwrap(), index);
    let want = r#"{
  "schema_version": 3,
  "packs": [
    {
      "id": "bridge",
      "namespace": "beta",
      "name": "Bridge",
      "description": "Runs linters.",
      "version": "0.3.1-rc.1",
      "kind": "plugin",
      "slopvac_api": 2,
      "source": "git+https://github.com/beta/bridge@0123456789012345678901234567890123456789",
      "publisher_key": {
        "key_id": "0123456789ABCDEF",
        "fingerprint": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
      }
    }
  ]
}
"#;
    assert_eq!(json, want);
}

#[test]
fn build_rejects_an_invalid_listing_with_its_position() {
    let mut bad = listing("acme", "b", "B", "d");
    bad.source = "http://example.com/b".into();
    let err = build_index([listing("acme", "a", "A", "d"), bad]).unwrap_err();
    assert_eq!(err.pointer, "/packs/1/source");
}

// ---- caps (N11) ----

#[test]
fn an_oversized_document_is_refused_before_parsing() {
    // Not JSON at all: the size error must win over the syntax error.
    let err = Index::parse(&vec![b'x'; MAX_INDEX_BYTES + 1]).unwrap_err();
    assert!(err.message.contains("limit"), "{err}");
}

#[test]
fn a_document_at_the_size_limit_is_parsed() {
    let mut doc = br#"{"schema_version":3,"packs":[]}"#.to_vec();
    doc.resize(MAX_INDEX_BYTES, b' ');
    assert!(Index::parse(&doc).is_ok());
}

#[test]
fn too_many_entries_are_refused() {
    let many: Vec<_> = (0..=MAX_ENTRIES)
        .map(|i| {
            entry(
                "acme",
                &format!("p{}", i.to_string().replace(|_| true, "a")),
            )
        })
        .collect();
    assert_eq!(many.len(), MAX_ENTRIES + 1);
    assert_eq!(Index::new(many).unwrap_err().pointer, "/packs");
}

#[test]
fn string_caps_count_characters() {
    // 1024 two-byte characters fit; one more does not.
    let at_cap = "é".repeat(MAX_DESCRIPTION_CHARS);
    assert!(build_index([listing("acme", "a", "A", &at_cap)]).is_ok());
    let over = "é".repeat(MAX_DESCRIPTION_CHARS + 1);
    let err = build_index([listing("acme", "a", "A", &over)]).unwrap_err();
    assert_eq!(err.pointer, "/packs/0/description");
}

// ---- search ----

fn corpus_index() -> Index {
    build_index([
        listing("acme", "style-core", "Core style", "Rules for tone"),
        listing("acme", "style", "Style guide", "Plain style rules"),
        listing(
            "beta",
            "lint-bridge",
            "Bridge",
            "Runs linters; style checks",
        ),
        listing("beta", "terms", "Term lists", "Vocabulary"),
        listing("acme", "zeta", "Zeta", "Style helper"),
    ])
    .unwrap()
}

fn ranked(index: &Index, query: &str, limit: usize) -> Vec<(String, u32)> {
    search(index, query, limit)
        .iter()
        .map(|h| (format!("{}/{}", h.entry.namespace, h.entry.id), h.score))
        .collect()
}

#[test]
fn exact_id_beats_prefix_beats_description_and_ties_sort_by_namespace_and_id() {
    // style: exact id 100 + whole-query bonus 50; style-core: prefix 60;
    // zeta and lint-bridge: description only, 10 each, acme before beta.
    let got = ranked(&corpus_index(), "style", 10);
    let want = [
        ("acme/style", 150),
        ("acme/style-core", 60),
        ("acme/zeta", 10),
        ("beta/lint-bridge", 10),
    ];
    assert_eq!(got, want.map(|(i, s)| (i.to_owned(), s)));
}

#[test]
fn every_token_must_match_and_scores_add_up() {
    // style: id 100 + rules in description 10 = 110 (joined `style-rules` is not an id).
    // style-core: prefix 60 + rules in description 10 = 70.
    // lint-bridge has no "rules" and drops out.
    let got = ranked(&corpus_index(), "style rules", 10);
    let want = [("acme/style", 110), ("acme/style-core", 70)];
    assert_eq!(got, want.map(|(i, s)| (i.to_owned(), s)));
}

#[test]
fn tokens_joined_by_a_dash_that_spell_the_id_earn_the_bonus() {
    // lint: prefix 60; bridge: inside the id 40; bonus 50.
    let got = ranked(&corpus_index(), "Lint bridge", 10);
    assert_eq!(got, [("beta/lint-bridge".to_owned(), 150)]);
}

#[test]
fn a_name_match_scores_thirty() {
    let got = ranked(&corpus_index(), "lists", 10);
    assert_eq!(got, [("beta/terms".to_owned(), 30)]);
}

#[test]
fn matching_ignores_case() {
    assert_eq!(
        ranked(&corpus_index(), "STYLE", 10),
        ranked(&corpus_index(), "style", 10)
    );
}

#[test]
fn the_limit_keeps_the_best_hits() {
    assert_eq!(
        ranked(&corpus_index(), "style", 2),
        [
            ("acme/style".to_owned(), 150),
            ("acme/style-core".to_owned(), 60)
        ]
    );
}

#[test]
fn a_query_without_tokens_matches_nothing() {
    for query in ["", "   ", "---", "!?"] {
        assert!(search(&corpus_index(), query, 10).is_empty(), "{query:?}");
    }
}

// ---- terminal safety (N11, TM-41) ----

#[test]
fn rendered_hits_carry_no_control_characters_from_the_index() {
    let hostile = "ok\u{1b}]8;;http://evil\u{7}x\nfake line\u{202e}rtl\u{200b}\u{9b}31m\u{7f}";
    let mut l = listing("acme", "a", "Name\u{1b}[2J", hostile);
    l.source = "https://example.com/a\u{1b}[0m".into();
    let index = build_index([l]).unwrap();
    let hit = search(&index, "ok", 1).remove(0);
    let text = hit.render();
    // Six lines the renderer wrote, and nothing else breaks a line.
    assert_eq!(text.lines().count(), 6, "{text:?}");
    let unsafe_left = text
        .chars()
        .filter(|&c| c != '\n')
        .any(|c| c.is_control() || "\u{202e}\u{200b}\u{2028}\u{feff}".contains(c));
    assert!(!unsafe_left, "{text:?}");
    assert!(text.contains(
        "ok\\u{1b}]8;;http://evil\\u{7}x\\u{a}fake line\\u{202e}rtl\\u{200b}\\u{9b}31m\\u{7f}"
    ));
}

#[test]
fn rendered_key_is_labelled_unverified() {
    let index = Index::new(vec![entry("acme", "a")]).unwrap();
    let text = search(&index, "a", 1).remove(0).render();
    assert!(
        text.contains("publisher key: 0123456789ABCDEF (fingerprint "),
        "{text}"
    );
}
// ---- fix round 1 ----

fn with_source_len(len: usize) -> String {
    let prefix = "oci://ghcr.io/acme/";
    format!("{prefix}{}", "s".repeat(len - prefix.len()))
}

#[test]
fn build_refuses_output_the_reader_would_refuse() {
    // 32768 entries of 1024 characters is 32 MiB of description text alone.
    let listings = (0..32_768).map(|i| {
        listing(
            "acme",
            &format!("p{i}"),
            "N",
            &"d".repeat(MAX_DESCRIPTION_CHARS),
        )
    });
    let err = build_index(listings).unwrap_err();
    assert_eq!(err.pointer, "");
    assert!(err.message.contains("limit"), "{err}");
}

#[test]
fn every_string_field_accepts_its_cap_and_refuses_one_more() {
    type Edit = fn(&mut Listing, String);
    let fields: [(&str, usize, Edit); 4] = [
        ("id", 64, |l, v| l.id = v),
        ("namespace", 64, |l, v| l.namespace = v),
        ("name", 128, |l, v| l.name = Some(v)),
        ("description", 1024, |l, v| l.description = v),
    ];
    for (field, cap, edit) in fields {
        let mut at_cap = listing("acme", "a", "A", "d");
        edit(&mut at_cap, "a".repeat(cap));
        assert!(build_index([at_cap]).is_ok(), "{field} at {cap}");
        let mut over = listing("acme", "a", "A", "d");
        edit(&mut over, "a".repeat(cap + 1));
        let err = build_index([over]).unwrap_err();
        assert_eq!(err.pointer, format!("/packs/0/{field}"));
    }
    let mut at_cap = listing("acme", "a", "A", "d");
    at_cap.source = with_source_len(2048);
    assert!(build_index([at_cap]).is_ok());
    let mut over = listing("acme", "a", "A", "d");
    over.source = with_source_len(2049);
    assert_eq!(build_index([over]).unwrap_err().pointer, "/packs/0/source");
}

#[test]
fn numeric_bounds_match_the_schema() {
    let mut l = listing("acme", "a", "A", "d");
    l.slopvac_api = u64::from(u32::MAX);
    l.version = Version::new(999_999_999, 999_999_999, 999_999_999);
    assert!(build_index([l.clone()]).is_ok());
    l.slopvac_api += 1;
    assert_eq!(
        build_index([l.clone()]).unwrap_err().pointer,
        "/packs/0/slopvac_api"
    );
    l.slopvac_api = 1;
    l.version = Version::new(1_000_000_000, 0, 0);
    assert_eq!(build_index([l]).unwrap_err().pointer, "/packs/0/version");
}

const ONE_ENTRY: &str = r#"{"schema_version":3,"packs":[{"id":"a","namespace":"b","name":"N","description":"d","version":"1.0.0","kind":"data","slopvac_api":2,"source":"oci://h/x","publisher_key":{"key_id":"0123456789ABCDEF","fingerprint":"0000000000000000000000000000000000000000000000000000000000000000"}}]}"#;

#[test]
fn whole_numbers_with_a_fraction_are_integers_as_in_the_schema() {
    let doc = ONE_ENTRY.replace("\"slopvac_api\":2", "\"slopvac_api\":2.0");
    assert_eq!(
        Index::parse(doc.as_bytes()).unwrap().entries()[0].slopvac_api,
        2
    );
    let fractional = ONE_ENTRY.replace("\"slopvac_api\":2", "\"slopvac_api\":2.5");
    assert_eq!(
        Index::parse(fractional.as_bytes()).unwrap_err().pointer,
        "/packs/0/slopvac_api"
    );
}

#[test]
fn a_repeated_key_is_an_error() {
    let doc = r#"{"schema_version":3,"packs":[],"packs":[]}"#;
    assert_eq!(Index::parse(doc.as_bytes()).unwrap_err().pointer, "");
}

#[test]
fn a_rejected_source_is_not_echoed_with_its_credentials() {
    let mut l = listing("acme", "a", "A", "d");
    l.source = "https://user:hunter2@example.com/a.tar.gz".into();
    let err = build_index([l]).unwrap_err();
    assert_eq!(err.pointer, "/packs/0/source");
    let shown = err.to_string();
    assert!(!shown.contains("hunter2"), "{shown}");
    let doc = ONE_ENTRY.replace("oci://h/x", "https://user:hunter2@example.com/a");
    let shown = Index::parse(doc.as_bytes()).unwrap_err().to_string();
    assert!(!shown.contains("hunter2"), "{shown}");
}

#[test]
fn invisible_and_format_characters_are_escaped_in_output() {
    // One code point per default-ignorable or format (Cf) range of Unicode 15,
    // taken from DerivedCoreProperties.txt and UnicodeData.txt, not from the code.
    let cases = [
        '\u{ad}',
        '\u{34f}',
        '\u{600}',
        '\u{115f}',
        '\u{17b4}',
        '\u{180e}',
        '\u{2060}',
        '\u{206f}',
        '\u{3164}',
        '\u{fe0f}',
        '\u{feff}',
        '\u{ffa0}',
        '\u{fff9}',
        '\u{110bd}',
        '\u{1bca0}',
        '\u{1d173}',
        '\u{e0001}',
        '\u{e0100}',
        '\u{2029}',
        '\u{85}',
    ];
    for c in cases {
        let want = format!("a\\u{{{:x}}}b", u32::from(c));
        assert_eq!(slopvac_sanitize::line(&format!("a{c}b")), want, "{c:?}");
    }
}

#[test]
fn rendered_hits_say_that_every_identity_claim_is_unverified() {
    let index = build_index([listing("acme", "a", "A", "d")]).unwrap();
    let text = search(&index, "a", 1).remove(0).render();
    assert!(
        text.contains("unverified: the namespace, source and key come from the index; pin a key with `slopvac plugins trust --key <fingerprint>`"),
        "{text}"
    );
    assert!(!text.contains("first-party"), "{text}");
}

#[test]
fn a_reserved_namespace_is_flagged_as_a_claim_not_shown_as_an_identity() {
    for reserved in ["core", "slopvac"] {
        let index = build_index([listing(reserved, "evil", "E", "d")]).unwrap();
        let text = search(&index, "evil", 1).remove(0).render();
        assert!(
            text.contains(&format!(
                "claims first-party namespace `{reserved}`; not verified"
            )),
            "{text}"
        );
    }
}

#[test]
fn a_bad_entry_is_refused_while_reading_not_after_the_whole_document_is_built() {
    // The tail is not JSON. A reader that builds the full tree first reports the
    // syntax error at the root; a streaming reader stops at the first bad entry.
    let doc = r#"{"schema_version":3,"packs":[0, this is not json"#;
    assert_eq!(
        Index::parse(doc.as_bytes()).unwrap_err().pointer,
        "/packs/0"
    );
}

#[test]
fn a_huge_ignored_x_value_and_a_huge_scalar_array_are_handled() {
    let scalars = "0,".repeat(2_000_000);
    let skipped = format!(r#"{{"schema_version":3,"x-big":[{scalars}0],"packs":[]}}"#);
    assert_eq!(Index::parse(skipped.as_bytes()).unwrap().entries().len(), 0);
    let refused = format!(r#"{{"schema_version":3,"packs":[{scalars}0]}}"#);
    assert_eq!(
        Index::parse(refused.as_bytes()).unwrap_err().pointer,
        "/packs/0"
    );
}
