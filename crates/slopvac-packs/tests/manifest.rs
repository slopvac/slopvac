//! Behaviors of `Manifest::parse` beyond the conformance corpus: what a parsed manifest holds,
//! which defect each corpus pointer stands for, and how errors print.

use rstest::rstest;
use slopvac_packs::{Manifest, ManifestError, PackKind, PackPath};

const ROOT: &str =
    "schema_version = 3\nid = \"tiny\"\nversion = \"0.1.0\"\nslopvac_api = 1\nkind = \"data\"\n";

const DATA: &str = r#"
schema_version = 3
id = "acme-docs"
version = "1.2.0"
slopvac_api = 2
kind = "data"
description = "House style."
license = "MIT"
homepage = "https://example.com/acme"
x-note = "ignored"

[provides]
styles = ["styles/acme.toml", "styles/*.toml"]
blocklists = ["lists/words.yml"]

[[requires]]
id = "core"
version = ">=3.0, <4"
"#;

const PLUGIN: &str = r#"
schema_version = 3
id = "acme-tagger"
version = "0.3.1-rc.1"
slopvac_api = 1
kind = "plugin"

[plugin]
command = "bin/acme"
args = ["--stdio"]
env = ["ACME_REGION"]
network = true
hooks = [{ kind = "tagger", id = "acme" }]

[plugin.features]
required = ["document-path"]
optional = ["x-private"]
"#;

#[test]
fn a_data_manifest_holds_what_the_file_says() {
    let m = Manifest::parse(DATA).unwrap();
    assert_eq!(m.id.as_str(), "acme-docs");
    assert_eq!(m.version.to_string(), "1.2.0");
    assert_eq!(m.slopvac_api.get(), 2);
    assert_eq!(m.kind, PackKind::Data);
    assert_eq!(m.description.as_deref(), Some("House style."));
    assert_eq!(m.homepage.as_deref(), Some("https://example.com/acme"));
    let styles: Vec<_> = m.provides.styles.iter().map(PackPath::as_str).collect();
    assert_eq!(styles, ["styles/acme.toml", "styles/*.toml"]);
    assert_eq!(m.provides.blocklists.len(), 1);
    assert_eq!(m.provides.genres.len(), 0);
    assert_eq!(m.requires.len(), 1);
    assert_eq!(m.requires[0].id.as_str(), "core");
    assert!(m.requires[0].version.matches(&"3.4.0".parse().unwrap()));
    assert!(!m.requires[0].version.matches(&"4.0.0".parse().unwrap()));
    assert!(m.plugin.is_none());
}

#[test]
fn a_plugin_manifest_holds_the_declaration() {
    let m = Manifest::parse(PLUGIN).unwrap();
    assert_eq!(m.kind, PackKind::Plugin);
    let p = m.plugin.unwrap();
    assert_eq!(p.command, "bin/acme");
    assert_eq!(p.args, ["--stdio"]);
    assert_eq!(p.env, ["ACME_REGION"]);
    assert!(p.network);
    assert_eq!(
        (p.hooks[0].kind.as_str(), p.hooks[0].id.as_str()),
        ("tagger", "acme")
    );
    assert_eq!(p.features_required, ["document-path"]);
    assert_eq!(p.features_optional, ["x-private"]);
}

#[rstest]
#[case::unknown_key("kind = \"data\"\ntier = 1", "", "unknown key")]
#[case::kind_missing("", "", "missing required key `kind`")]
#[case::plugin_table_missing("kind = \"plugin\"", "", "needs a `[plugin]` table")]
#[case::plugin_table_on_data(
    "kind = \"data\"\n[plugin]\ncommand = \"x\"\nnetwork = false\nhooks = []\nfeatures = { required = [], optional = [] }",
    "",
    "must not have a `[plugin]` table"
)]
#[case::schema_version_two("kind = \"data\"\nschema_version = 2", "/schema_version", "must be 3")]
#[case::build_metadata(
    "kind = \"data\"\nversion = \"1.0.0+build\"",
    "/version",
    "build metadata"
)]
#[case::api_too_large(
    "kind = \"data\"\nslopvac_api = 4294967296",
    "/slopvac_api",
    "at least 1"
)]
#[case::kind_unknown("kind = \"theme\"", "/kind", "not a pack kind")]
#[case::duplicate_path(
    "kind = \"data\"\n[provides]\nterms = [\"a.toml\", \"a.toml\"]",
    "/provides/terms/1",
    "repeats"
)]
#[case::homepage_not_uri("kind = \"data\"\nhomepage = \"no scheme\"", "/homepage", "not a URI")]
#[case::wildcard_requirement(
    "kind = \"data\"\n[[requires]]\nid = \"core\"\nversion = \"*\"",
    "/requires/0/version",
    "not a version requirement"
)]
fn a_defect_is_reported_at_its_pointer(
    #[case] extra: &str,
    #[case] pointer: &str,
    #[case] message: &str,
) {
    let base = [
        ("schema_version", "3"),
        ("id", "\"tiny\""),
        ("version", "\"0.1.0\""),
        ("slopvac_api", "1"),
    ];
    // Root keys in `extra` replace the base keys of the same name.
    let root_lines: Vec<&str> = extra.lines().take_while(|l| !l.starts_with('[')).collect();
    let mut text = String::new();
    for (key, value) in base {
        if !root_lines
            .iter()
            .any(|l| l.starts_with(&format!("{key} =")))
        {
            text.extend([key, " = ", value, "\n"]);
        }
    }
    text.push_str(extra);
    let e = Manifest::parse(&text).unwrap_err();
    assert_eq!(e.pointer(), Some(pointer), "{e}");
    assert!(e.to_string().contains(message), "{e}");
}

#[rstest]
#[case::caret("^1.2")]
#[case::range(">=3.0, <4")]
#[case::bare("1")]
#[case::prerelease("^1.0.0-rc.1")]
fn a_requirement_in_the_grammar_is_accepted(#[case] version: &str) {
    let text = format!("{ROOT}[[requires]]\nid = \"core\"\nversion = \"{version}\"\n");
    assert!(Manifest::parse(&text).is_ok(), "{version}");
}

#[rstest]
#[case::wildcard("*")]
#[case::empty("")]
#[case::four_parts("1.2.3.4")]
#[case::trailing_comma(">=1,")]
#[case::word("latest")]
#[case::partial_prerelease("1.0-rc")]
fn a_requirement_outside_the_grammar_is_refused(#[case] version: &str) {
    let text = format!("{ROOT}[[requires]]\nid = \"core\"\nversion = \"{version}\"\n");
    let e = Manifest::parse(&text).unwrap_err();
    assert_eq!(e.pointer(), Some("/requires/0/version"), "{version}");
}

#[test]
fn a_homepage_with_user_info_is_refused() {
    let text = format!("{ROOT}homepage = \"https://user:pw@example.com/\"\n");
    let e = Manifest::parse(&text).unwrap_err();
    assert_eq!(e.pointer(), Some("/homepage"));
}

#[test]
fn a_reader_over_the_cap_is_refused() {
    let big = vec![b'#'; 256 * 1024 + 1];
    assert!(matches!(
        Manifest::from_reader(big.as_slice()),
        Err(ManifestError::Read(_))
    ));
}

#[test]
fn a_reader_with_invalid_utf8_is_refused_at_the_root() {
    let e = Manifest::from_reader(&[0xff, 0xfe][..]).unwrap_err();
    assert_eq!(e.pointer(), Some(""));
}

#[test]
fn a_reader_yields_the_same_manifest_as_text() {
    assert_eq!(
        Manifest::from_reader(DATA.as_bytes()).unwrap(),
        Manifest::parse(DATA).unwrap()
    );
}
