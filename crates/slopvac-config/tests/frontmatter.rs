//! Front-matter reading and the allow policy, through the public interface.

use serde_json::json;
use slopvac_config::frontmatter::{Policy, extract, read};
use slopvac_config::{Code, Inputs, Layer, NoPresets, Source, load};
use std::path::Path;

fn default_policy() -> Policy {
    Policy {
        enabled: true,
        allow: vec!["genre".into(), "locale".into(), "terms".into()],
    }
}

fn allow(fields: &[&str]) -> Policy {
    Policy {
        enabled: true,
        allow: fields.iter().map(|s| (*s).to_owned()).collect(),
    }
}

#[test]
fn allowed_fields_become_settings_with_document_positions() {
    let doc = "---\ntitle: Guide\nslopvac:\n  v: 3\n  locale: en-GB\n  terms:\n    allow: [colour]\n---\nBody\n";
    let out = read(doc, "docs/guide.md", &default_policy());
    assert_eq!(out.diagnostics, []);
    let locale = out.settings.iter().find(|s| s.path == ["locale"]).unwrap();
    assert_eq!(locale.value, json!("en-GB"));
    assert_eq!(
        locale.source,
        Source {
            layer: Layer::FrontMatter,
            file: Some("docs/guide.md".into()),
            line: Some(5),
            column: Some(3),
        }
    );
    let terms = out
        .settings
        .iter()
        .find(|s| s.path == ["terms", "allow"])
        .unwrap();
    assert_eq!(terms.value, json!(["colour"]));
    assert_eq!(terms.source.line, Some(7));
    assert!(out.settings.iter().all(|s| s.path != ["v"]));
}

#[test]
fn a_field_outside_the_allow_list_is_an_error_at_its_key_every_time() {
    let doc = "---\nslopvac:\n  v: 3\n  rules:\n    core:no-dash: \"off\"\n---\n";
    for _ in 0..2 {
        let out = read(doc, "a.md", &default_policy());
        assert_eq!(out.settings, []);
        assert_eq!(out.diagnostics.len(), 1);
        let d = &out.diagnostics[0];
        assert_eq!(d.code, Code::FrontmatterFieldDenied);
        assert_eq!(d.location(doc), (4, 3));
        assert_eq!(&doc[d.span.clone()], "rules");
        assert!(d.help.as_deref().unwrap().contains("[frontmatter] allow"));
    }
}

#[test]
fn a_project_can_allow_rule_and_threshold_changes() {
    let doc = "---\nslopvac:\n  v: 3\n  rules:\n    core:no-dash: \"off\"\n  thresholds:\n    min_score: 60\n---\n";
    let out = read(doc, "a.md", &allow(&["rules", "thresholds"]));
    assert_eq!(out.diagnostics, []);
    let paths: Vec<_> = out.settings.iter().map(|s| s.path.join(".")).collect();
    assert_eq!(paths, ["rules.core:no-dash", "thresholds.min_score"]);
}

#[test]
fn a_denied_field_blocks_the_allowed_fields_too() {
    let doc = "---\nslopvac:\n  v: 3\n  locale: en-GB\n  profile: relaxed\n---\n";
    let out = read(doc, "a.md", &default_policy());
    assert_eq!(out.settings, []);
    assert_eq!(out.diagnostics.len(), 1);
}

#[test]
fn disabled_front_matter_is_plain_data() {
    let doc = "---\nslopvac:\n  nonsense: [\n---\n";
    let policy = Policy {
        enabled: false,
        allow: vec![],
    };
    assert_eq!(
        read(doc, "a.md", &policy),
        slopvac_config::frontmatter::Read::default()
    );
}

#[test]
fn keys_outside_slopvac_are_never_checked() {
    let doc = "---\ntitle: 3\ntags: [a, b]\nslopvac:\n  v: 3\n  x-note: ignored\n---\n";
    let out = read(doc, "a.md", &default_policy());
    assert_eq!(out.diagnostics, []);
    assert_eq!(out.settings, []);
}

#[test]
fn a_document_without_a_slopvac_key_contributes_nothing() {
    for doc in [
        "no front matter\n",
        "---\ntitle: x\n---\n",
        "---\n- a\n- b\n---\n",
    ] {
        let out = read(doc, "a.md", &default_policy());
        assert_eq!(
            (out.settings.len(), out.diagnostics.len()),
            (0, 0),
            "{doc:?}"
        );
    }
}

#[test]
fn a_misspelled_key_is_an_error_at_its_position_with_a_suggestion() {
    let doc = "---\nslopvac:\n  v: 3\n  profil: strict\n---\n";
    let out = read(doc, "a.md", &default_policy());
    let d = &out.diagnostics[0];
    assert_eq!(d.code, Code::UnknownKey);
    assert_eq!(d.location(doc), (4, 3));
    assert_eq!(d.help.as_deref(), Some("did you mean `profile`?"));
}

#[test]
fn the_version_is_required_and_a_newer_one_asks_for_an_upgrade() {
    let missing = read(
        "---\nslopvac:\n  locale: en-GB\n---\n",
        "a.md",
        &default_policy(),
    );
    assert_eq!(missing.diagnostics[0].code, Code::MissingKey);

    let doc = "---\nslopvac:\n  v: 4\n---\n";
    let newer = read(doc, "a.md", &default_policy());
    let d = &newer.diagnostics[0];
    assert_eq!(d.location(doc), (3, 6));
    assert_eq!(d.help.as_deref(), Some("upgrade slopvac"));
}

#[test]
fn toml_and_json_front_matter_read_the_same_block() {
    let toml = "+++\ntitle = \"x\"\n\n[slopvac]\nv = 3\nlocale = \"en-GB\"\n+++\nBody\n";
    let json_doc =
        "{\n  \"title\": \"x\",\n  \"slopvac\": {\"v\": 3, \"locale\": \"en-GB\"}\n}\nBody\n";
    for (doc, line) in [(toml, 6), (json_doc, 3)] {
        let out = read(doc, "a.md", &default_policy());
        assert_eq!(out.diagnostics, [], "{doc:?}");
        let locale = out.settings.iter().find(|s| s.path == ["locale"]).unwrap();
        assert_eq!(locale.value, json!("en-GB"));
        assert_eq!(locale.source.line, Some(line), "{doc:?}");
    }
}

#[test]
fn broken_front_matter_that_mentions_slopvac_is_reported_and_other_front_matter_is_not() {
    let broken = "---\nslopvac: [\n---\n";
    let out = read(broken, "a.md", &default_policy());
    assert_eq!(out.diagnostics[0].code, Code::Parse);

    let unrelated = "---\ntitle: [\n---\n";
    assert_eq!(read(unrelated, "a.md", &default_policy()).diagnostics, []);
}

#[test]
fn extract_finds_each_format_and_ignores_unclosed_blocks() {
    let yaml = extract("---\na: 1\n---\nbody").unwrap();
    assert_eq!(&"---\na: 1\n---\nbody"[yaml.content], "a: 1\n");
    assert!(extract("---\na: 1\nbody").is_none());
    assert!(extract("text\n---\na: 1\n---\n").is_none());
    let crlf = "---\r\na: 1\r\n---\r\nbody";
    assert_eq!(&crlf[extract(crlf).unwrap().content], "a: 1\r\n");
    let json_doc = "{\"a\": \"}\"}\ntext";
    assert_eq!(
        &json_doc[extract(json_doc).unwrap().content],
        "{\"a\": \"}\"}"
    );
}

#[test]
fn front_matter_settings_sit_above_overrides_and_the_policy_comes_from_the_config() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    std::fs::write(
        root.join("slopvac.toml"),
        "schema_version = 3\n\n[frontmatter]\nallow = [\"locale\", \"rules\"]\n\n[[overrides]]\nfiles = [\"docs/**\"]\nlocale = \"en-US\"\n",
    )
    .unwrap();
    let cfg = load(&root.join("slopvac.toml"), &NoPresets).unwrap();
    let policy = Policy::from_config(&cfg);
    assert_eq!(policy.allow, ["locale", "rules"]);

    let doc = "---\nslopvac:\n  v: 3\n  locale: en-GB\n---\n";
    let fm = read(doc, "docs/a.md", &policy);
    let r = cfg.resolve(
        Path::new("docs/a.md"),
        &Inputs {
            presets: &NoPresets,
            front_matter: &fm.settings,
            directives: &[],
            cli: &[],
            env: &[],
        },
    );
    assert_eq!(r.get(&["locale"]), Some(&json!("en-GB")));
    assert_eq!(
        r.source(&["locale"]).map(|s| s.layer),
        Some(Layer::FrontMatter)
    );

    let disabled = dir.path().join("off");
    std::fs::create_dir_all(&disabled).unwrap();
    std::fs::write(
        disabled.join("slopvac.toml"),
        "schema_version = 3\n[frontmatter]\nenabled = false\n",
    )
    .unwrap();
    let off = load(&disabled.join("slopvac.toml"), &NoPresets).unwrap();
    assert!(!Policy::from_config(&off).enabled);
    assert!(Policy::from_config(&cfg).enabled);
}

#[test]
fn the_block_is_found_by_its_decoded_key() {
    let doc = "---\n\"sl\\u006fpvac\":\n  v: 3\n  locale: en-GB\n---\n";
    let out = read(doc, "a.md", &default_policy());
    assert_eq!(out.diagnostics, []);
    assert_eq!(out.settings.len(), 1);
    assert_eq!(out.settings[0].value, json!("en-GB"));
    assert_eq!(out.settings[0].source.line, Some(4));

    let json_doc = "{\"sl\\u006fpvac\": {\"v\": 3, \"locale\": \"en-GB\"}}\n";
    assert_eq!(read(json_doc, "a.md", &default_policy()).settings.len(), 1);

    let other = "---\n\"t\\u0069tle\": x\n---\n";
    let out = read(other, "a.md", &default_policy());
    assert_eq!((out.settings.len(), out.diagnostics.len()), (0, 0));

    let denied = "---\n\"sl\\u006fpvac\":\n  v: 3\n  rules:\n    core:x: \"off\"\n---\n";
    assert_eq!(
        read(denied, "a.md", &default_policy()).diagnostics[0].code,
        Code::FrontmatterFieldDenied
    );
}

#[test]
fn a_block_with_a_utf8_byte_order_mark_reports_columns_after_the_mark() {
    let doc = "\u{feff}---\nslopvac:\n  v: 3\n  profil: strict\n---\n";
    let out = read(doc, "a.md", &default_policy());
    assert_eq!(out.diagnostics[0].location(doc), (4, 3));
}

#[test]
fn an_escaped_slopvac_key_with_broken_contents_is_still_reported() {
    for doc in [
        "---\n\"sl\\u006fpvac\": [\n---\n",
        "{\"sl\\u006fpvac\": [1, }\n",
        "+++\n[\"sl\\u006fpvac\"]\nv = = 3\n+++\n",
    ] {
        let out = read(doc, "a.md", &default_policy());
        assert_eq!(out.diagnostics.len(), 1, "{doc:?}");
        assert_eq!(out.diagnostics[0].code, Code::Parse, "{doc:?}");
    }
    let unrelated = "---\n\"t\\u0069tle\": [\n---\n";
    assert_eq!(read(unrelated, "a.md", &default_policy()).diagnostics, []);
}

#[test]
fn front_matter_ended_by_lone_cr_terminators_is_read() {
    let doc = "---\rslopvac:\r  v: 3\r  profil: strict\r---\rBody\r";
    let out = read(doc, "a.md", &default_policy());
    assert_eq!(out.diagnostics.len(), 1);
    assert_eq!(out.diagnostics[0].code, Code::UnknownKey);
    assert_eq!(out.diagnostics[0].location(doc), (4, 3));

    let ok = "---\rslopvac:\r  v: 3\r  locale: en-GB\r---\r";
    let out = read(ok, "a.md", &default_policy());
    assert_eq!(out.diagnostics, []);
    assert_eq!(out.settings[0].source.line, Some(4));
    assert_eq!(extract("---\ra: 1\r---\r").map(|b| b.content), Some(4..9));
}
