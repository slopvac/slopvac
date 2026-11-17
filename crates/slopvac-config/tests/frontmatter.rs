//! Front-matter reading and the allow policy, through the public interface.

use serde_json::json;
use slopvac_config::frontmatter::{Format, Policy, extract, parse_block, read};
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

#[test]
fn only_top_level_keys_make_a_broken_block_ours() {
    // A broken block is reported when a top-level key decodes to `slopvac`: YAML key, JSON key,
    // TOML table header, dotted TOML header, dotted TOML root key.
    for doc in [
        "---\n\"sl\\u006fpvac\": [\n---\n",
        "{\"sl\\u006fpvac\": [1, }\n",
        "+++\n[\"sl\\u006fpvac\"]\nv = = 3\n+++\n",
        "+++\n[\"sl\\u006fpvac\".terms]\nallow = = 1\n+++\n",
        "+++\n\"sl\\u006fpvac\".v = = 3\n+++\n",
    ] {
        let out = read(doc, "a.md", &default_policy());
        assert_eq!(out.diagnostics.len(), 1, "{doc:?}");
        assert_eq!(out.diagnostics[0].code, Code::Parse, "{doc:?}");
    }
    // A value or a nested key that decodes to `slopvac` is the document's own data.
    for doc in [
        "---\ntags: [\"sl\\u006fpvac\"]\nbroken: [\n---\n",
        "---\nouter:\n  \"sl\\u006fpvac\": 1\nbroken: [\n---\n",
        "{\"tags\": [\"sl\\u006fpvac\"], \"nested\": {\"sl\\u006fpvac\": 1}, \"bad\": [1, }\n",
        "+++\n[\"x\".\"sl\\u006fpvac\"]\nv = = 3\n+++\n",
        "+++\ntags = [\"sl\\u006fpvac\"]\n[other]\nv = = 3\n+++\n",
    ] {
        let out = read(doc, "a.md", &default_policy());
        assert_eq!(out.diagnostics, [], "{doc:?}");
    }
}

#[test]
fn the_word_slopvac_in_a_value_does_not_make_a_broken_block_ours() {
    for doc in [
        "---\ntitle: slopvac\nbroken: [\n---\n",
        "{\"title\": \"slopvac\", \"bad\": [1, }\n",
        "+++\ntitle = \"slopvac\"\nv = = 3\n+++\n",
    ] {
        let out = read(doc, "a.md", &default_policy());
        assert_eq!(out.diagnostics, [], "{doc:?}");
    }
}

#[test]
fn lone_cr_line_ends_work_with_escaped_keys_in_broken_blocks() {
    // The escaped key is on the second line; a splitter that ignores a lone CR sees one line.
    for doc in [
        "---\rtitle: x\r\"sl\\u006fpvac\": [\r---\r",
        "+++\rtitle = \"x\"\r\"sl\\u006fpvac\".v = = 3\r+++\r",
        "+++\rtitle = \"x\"\r[\"sl\\u006fpvac\"]\rv = = 3\r+++\r",
    ] {
        let out = read(doc, "a.md", &default_policy());
        assert_eq!(out.diagnostics.len(), 1, "{doc:?}");
        assert_eq!(out.diagnostics[0].code, Code::Parse, "{doc:?}");
    }
    let nested = "---\rtitle: x\router:\r  \"sl\\u006fpvac\": 1\rbroken: [\r---\r";
    assert_eq!(read(nested, "a.md", &default_policy()).diagnostics, []);
}

#[test]
fn a_repeated_key_is_one_parse_error_at_the_second_key_in_every_format() {
    // (document, byte offset of the second key, message). JSON keeps every member in the crate's
    // tree and reads the last, so the check is ours; YAML and TOML refuse in their parsers. The
    // TOML text is `toml_edit`'s and names no key.
    for (doc, at, message) in [
        (
            "---\nslopvac:\n  v: 3\nslopvac:\n  v: 3\n---\n",
            20,
            "the front matter does not parse: duplicate key `slopvac`",
        ),
        (
            "---\nslopvac:\n  v: 3\n  v: 3\n---\n",
            22,
            "the front matter does not parse: duplicate key `v`",
        ),
        (
            "+++\n[slopvac]\nv = 3\n[slopvac]\nv = 3\n+++\n",
            21,
            "the front matter does not parse: duplicate key",
        ),
        (
            "{\"slopvac\": {\"v\": 3}, \"slopvac\": {\"v\": 3}}\n",
            22,
            "the front matter does not parse: duplicate key `slopvac`",
        ),
        (
            "{\"slopvac\": {\"v\": 3, \"v\": 4}}\n",
            21,
            "the front matter does not parse: duplicate key `v`",
        ),
    ] {
        let out = read(doc, "a.md", &default_policy());
        assert_eq!(out.settings, [], "{doc:?}");
        assert_eq!(out.diagnostics.len(), 1, "{doc:?}");
        let d = &out.diagnostics[0];
        assert_eq!(d.code, Code::Parse, "{doc:?}");
        assert_eq!(d.message, message, "{doc:?}");
        assert_eq!(d.span, at..at, "{doc:?}");
    }
}

#[test]
fn a_repeated_key_in_a_block_without_slopvac_is_not_ours() {
    let doc = "{\"title\": 1, \"title\": 2}\n";
    assert_eq!(read(doc, "a.md", &default_policy()).diagnostics, []);
}

#[test]
fn front_matter_yaml_is_the_strict_subset_of_the_crate() {
    // frontmatter-spans README, "The YAML subset": anchors, aliases and tags are errors, and JSON
    // is parsed as RFC 8259 JSON, so a YAML flow mapping or a single-quoted string is not. 2.x read
    // all of these through a YAML parser.
    for (doc, message) in [
        (
            "---\nslopvac: &a\n  v: 3\n---\n",
            "the front matter does not parse: a YAML anchor is not supported in front matter",
        ),
        (
            "---\nbase: &a 1\nslopvac:\n  v: *a\n---\n",
            "the front matter does not parse: a YAML anchor is not supported in front matter",
        ),
        (
            "---\nslopvac:\n  v: !!int 3\n---\n",
            "the front matter does not parse: a YAML tag is not supported in front matter",
        ),
        (
            "{\"slopvac\": {'v': 3}}\n",
            "the front matter does not parse: expected a string key",
        ),
    ] {
        let out = read(doc, "a.md", &default_policy());
        assert_eq!(out.settings, [], "{doc:?}");
        assert_eq!(out.diagnostics.len(), 1, "{doc:?}");
        assert_eq!(out.diagnostics[0].code, Code::Parse, "{doc:?}");
        assert_eq!(out.diagnostics[0].message, message, "{doc:?}");
    }
}

#[test]
fn a_yaml_flow_mapping_without_quoted_keys_is_no_longer_front_matter_of_ours() {
    // 2.x read these through a YAML parser. The crate treats a leading `{` as JSON, which needs
    // quoted keys, and the ownership check finds no `"slopvac"` key in the broken text, so the
    // document contributes nothing, as for any other block that is not ours.
    for doc in ["{slopvac: {v: 3}}\n", "{'slopvac': {'v': 3}}\n"] {
        assert_eq!(
            read(doc, "a.md", &default_policy()),
            slopvac_config::frontmatter::Read::default(),
            "{doc:?}"
        );
    }
}

#[test]
fn fence_lines_ignore_trailing_unicode_whitespace_as_in_2x() {
    // NBSP is Unicode whitespace but not a space or tab. 2.x trimmed it on both fences.
    let doc = "---\u{a0}\na: 1\n---\u{a0}\nbody";
    let block = extract(doc).unwrap();
    assert_eq!(block.format, Format::Yaml);
    assert_eq!(&doc[block.content.clone()], "a: 1\n");
    assert_eq!(&doc[block.outer.clone()], "---\u{a0}\na: 1\n---\u{a0}");
    assert_eq!(&doc[block.body_start..], "body");

    let toml = "+++\u{2003}\nslopvac.v = 3\n+++\u{2003}\n";
    assert_eq!(extract(toml).map(|b| b.format), Some(Format::Toml));

    let read_doc = "---\u{a0}\nslopvac:\n  v: 3\n  locale: en-GB\n---\u{a0}\n";
    let out = read(read_doc, "a.md", &default_policy());
    assert_eq!(out.diagnostics, []);
    assert_eq!(out.settings[0].value, json!("en-GB"));
}

/// A negative fixture is only a test of ownership if the block really fails to parse.
#[expect(
    clippy::panic,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn assert_malformed(doc: &str) {
    let block = extract(doc).unwrap_or_else(|| panic!("no block in {doc:?}"));
    assert!(
        parse_block(doc, &block).is_err(),
        "{doc:?} parses, so it never reaches ownership"
    );
}

#[test]
fn the_negative_fixtures_are_genuinely_malformed() {
    for doc in [
        "---\ntags: [\"sl\\u006fpvac\"]\nbroken: [\n---\n",
        "---\nouter:\n  \"sl\\u006fpvac\": 1\nbroken: [\n---\n",
        "{\"tags\": [\"sl\\u006fpvac\"], \"nested\": {\"sl\\u006fpvac\": 1}, \"bad\": [1, }\n",
        "+++\n[\"x\".\"sl\\u006fpvac\"]\nv = = 3\n+++\n",
        "+++\ntags = [\"sl\\u006fpvac\"]\n[other]\nv = = 3\n+++\n",
        "---\ntitle: slopvac\nbroken: [\n---\n",
        "{\"title\": \"slopvac\", \"bad\": [1, }\n",
        "+++\ntitle = \"slopvac\"\nv = = 3\n+++\n",
        "---\rtitle: x\router:\r  \"sl\\u006fpvac\": 1\rbroken: [\r---\r",
    ] {
        assert_malformed(doc);
    }
}

#[test]
fn a_wide_table_is_checked_for_repeats_in_linear_time() {
    // 100 000 distinct keys: a pairwise scan needs about 5e9 key comparisons, which takes far
    // longer than the bound even in a release build. A per-table set finishes in milliseconds.
    let keys: Vec<String> = (0..100_000).map(|i| format!("\"k{i}\": 0")).collect();
    let doc = format!(
        "{{\"slopvac\": {{\"v\": 3}}, \"meta\": {{{}}}}}\n",
        keys.join(", ")
    );
    let block = extract(&doc).expect("a JSON block");
    let start = std::time::Instant::now();
    assert!(parse_block(&doc, &block).is_ok());
    let took = start.elapsed();
    assert!(took < std::time::Duration::from_secs(5), "took {took:?}");
}
