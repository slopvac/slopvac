//! Properties of `Manifest::parse` over generated input.

use std::fmt::Write as _;

use proptest::prelude::*;
use slopvac_packs::{Manifest, ManifestError, PackKind};

const VALID: &str = r#"schema_version = 3
id = "acme-docs"
version = "1.2.0"
slopvac_api = 1
kind = "plugin"
description = "House style."

[provides]
styles = ["styles/a.toml"]

[[requires]]
id = "core"
version = ">=3.0, <4"

[plugin]
command = "bin/acme"
network = false
hooks = [{ kind = "tagger", id = "acme" }]
features = { required = [], optional = ["document-path"] }
"#;

/// A JSON Pointer as RFC 6901 writes it: empty, or `/` segments with `~` only before `0` or `1`.
fn is_json_pointer(p: &str) -> bool {
    if p.is_empty() {
        return true;
    }
    p.starts_with('/')
        && p.split('/').skip(1).all(|seg| {
            seg.match_indices('~')
                .all(|(i, _)| matches!(seg.as_bytes().get(i + 1), Some(b'0' | b'1')))
        })
}

fn check_outcome(result: &Result<Manifest, ManifestError>) {
    if let Err(e) = result {
        if let Some(p) = e.pointer() {
            assert!(is_json_pointer(p), "{p:?}");
        }
        // Display never panics and stays on one line.
        assert!(!e.to_string().contains('\n'));
    }
}

/// Mutate a valid manifest: cut a range, then insert arbitrary text.
fn mutated() -> impl Strategy<Value = String> {
    (0..VALID.len(), 0..40usize, ".{0,12}").prop_map(|(at, cut, insert)| {
        let start = (0..=at)
            .rev()
            .find(|i| VALID.is_char_boundary(*i))
            .unwrap_or(0);
        let end = (start + cut..=VALID.len())
            .find(|i| VALID.is_char_boundary(*i))
            .unwrap_or(VALID.len());
        format!("{}{insert}{}", &VALID[..start], &VALID[end..])
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn arbitrary_text_never_panics_and_errors_carry_a_pointer(text in ".{0,300}") {
        check_outcome(&Manifest::parse(&text));
    }

    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..300)) {
        check_outcome(&Manifest::from_reader(bytes.as_slice()));
    }

    #[test]
    fn a_mutated_manifest_never_panics_and_errors_carry_a_pointer(text in mutated()) {
        check_outcome(&Manifest::parse(&text));
    }

    #[test]
    fn a_generated_data_manifest_round_trips(
        id in "[a-z][a-z0-9-]{0,20}",
        (major, minor, patch) in (0u32..1000, 0u32..1000, 0u32..1000),
        api in 1u32..1000,
        styles in proptest::collection::btree_set("[a-z][a-z0-9/_.-]{0,15}\\.toml", 0..4),
        required in proptest::collection::btree_set("[a-z][a-z0-9-]{0,10}", 0..3),
    ) {
        let mut text = format!(
            "schema_version = 3\nid = \"{id}\"\nversion = \"{major}.{minor}.{patch}\"\nslopvac_api = {api}\nkind = \"data\"\n"
        );
        if !styles.is_empty() {
            let list: Vec<String> = styles.iter().map(|s| format!("\"{s}\"")).collect();
            write!(text, "[provides]\nstyles = [{}]\n", list.join(", ")).unwrap();
        }
        for r in &required {
            write!(text, "[[requires]]\nid = \"{r}\"\nversion = \"^{major}.{minor}\"\n").unwrap();
        }
        let m = Manifest::parse(&text).expect("a generated manifest is valid");
        prop_assert_eq!(m.id.as_str(), id.as_str());
        prop_assert_eq!(m.version.to_string(), format!("{major}.{minor}.{patch}"));
        prop_assert_eq!(m.slopvac_api.get(), api);
        prop_assert_eq!(m.kind, PackKind::Data);
        let got: Vec<&str> = m.provides.styles.iter().map(slopvac_packs::PackPath::as_str).collect();
        let want: Vec<&str> = styles.iter().map(String::as_str).collect();
        prop_assert_eq!(got, want);
        let got: Vec<&str> = m.requires.iter().map(|r| r.id.as_str()).collect();
        let want: Vec<&str> = required.iter().map(String::as_str).collect();
        prop_assert_eq!(got, want);
        prop_assert!(m.plugin.is_none());
    }

    #[test]
    fn a_path_that_leaves_the_pack_is_always_refused(
        prefix in "([a-z]{1,5}/){0,3}",
        suffix in "(/[a-z]{1,5}){0,3}",
    ) {
        let text = format!(
            "schema_version = 3\nid = \"t\"\nversion = \"0.1.0\"\nslopvac_api = 1\nkind = \"data\"\n[provides]\nstyles = [\"{prefix}..{suffix}\"]\n"
        );
        let e = Manifest::parse(&text).unwrap_err();
        prop_assert_eq!(e.pointer(), Some("/provides/styles/0"));
    }
}
