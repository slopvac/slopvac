//! Inline directives for documents without front matter, through the public interface.

use serde_json::json;
use slopvac_config::directives::{find, read};
use slopvac_config::frontmatter::Policy;
use slopvac_config::{Code, Inputs, Layer, NoPresets, load};
use std::path::Path;

fn default_policy() -> Policy {
    Policy {
        enabled: true,
        allow: vec!["genre".into(), "locale".into(), "terms".into()],
    }
}

fn allow_all() -> Policy {
    Policy {
        enabled: true,
        allow: [
            "genre",
            "profile",
            "locale",
            "thresholds",
            "categories",
            "rules",
            "terms",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect(),
    }
}

#[test]
fn each_comment_style_sets_fields_with_document_positions() {
    let cases = [
        (
            "guide.md",
            "Title\n\n<!-- slopvac: genre=how-to locale=en-GB -->\n",
            3,
            15,
        ),
        (
            "guide.rst",
            "Title\n=====\n\n.. slopvac: genre=how-to locale=en-GB\n",
            4,
            13,
        ),
        (
            "notes.txt",
            "# slopvac: genre=how-to locale=en-GB\ntext\n",
            1,
            12,
        ),
        (
            "main.rs",
            "fn main() {}\n// slopvac: genre=how-to locale=en-GB\n",
            2,
            13,
        ),
        ("lib.c", "/* slopvac: genre=how-to locale=en-GB */\n", 1, 13),
        (
            "query.sql",
            "-- slopvac: genre=how-to locale=en-GB\n",
            1,
            13,
        ),
    ];
    for (file, doc, line, col) in cases {
        let out = read(doc, file, &default_policy());
        assert_eq!(out.diagnostics, [], "{file}");
        let paths: Vec<_> = out.settings.iter().map(|s| s.path.join(".")).collect();
        assert_eq!(paths, ["genre", "locale"], "{file}");
        assert_eq!(out.settings[0].value, json!("how-to"), "{file}");
        assert_eq!(out.settings[1].value, json!("en-GB"), "{file}");
        let s = &out.settings[0].source;
        assert_eq!(
            (s.layer, s.line, s.column),
            (Layer::Directive, Some(line), Some(col)),
            "{file}"
        );
    }
}

#[test]
fn a_comment_style_only_counts_in_the_documents_that_use_it() {
    assert_eq!(find("# slopvac: locale=en-GB\n", "README.md"), []);
    assert_eq!(find("<!-- slopvac: locale=en-GB -->\n", "notes.txt"), []);
    assert_eq!(find(".. slopvac: locale=en-GB\n", "notes.md"), []);
    assert_eq!(find("<!-- slopvac: locale=en-GB\n", "a.md"), []);
    assert_eq!(find("<!-- note: slopvac: locale=en-GB -->\n", "a.md"), []);
}

#[test]
fn dotted_keys_quotes_and_lists_set_nested_fields() {
    let doc = "<!-- slopvac: thresholds.min_score=60 rules.core:no-dash=off terms.allow=colour,slopvac profile=\"relaxed\" -->\n";
    let out = read(doc, "a.md", &allow_all());
    assert_eq!(out.diagnostics, []);
    let got: Vec<_> = out
        .settings
        .iter()
        .map(|s| (s.path.join("."), s.value.clone()))
        .collect();
    assert_eq!(
        got,
        [
            ("thresholds.min_score".to_owned(), json!(60)),
            ("rules.core:no-dash".to_owned(), json!("off")),
            ("terms.allow".to_owned(), json!(["colour", "slopvac"])),
            ("profile.name".to_owned(), json!("relaxed")),
        ]
    );
    assert_eq!(out.settings[0].source.column, Some(26));
}

#[test]
fn a_field_outside_the_allow_list_is_an_error_at_its_key() {
    let doc = "intro\n<!-- slopvac: locale=en-GB rules.core:no-dash=off -->\n";
    let out = read(doc, "a.md", &default_policy());
    assert_eq!(out.settings, []);
    assert_eq!(out.diagnostics.len(), 1);
    let d = &out.diagnostics[0];
    assert_eq!(d.code, Code::FrontmatterFieldDenied);
    assert_eq!(d.location(doc), (2, 28));
    assert_eq!(&doc[d.span.clone()], "rules");
    assert!(d.message.contains("inline directive"), "{}", d.message);
}

#[test]
fn an_unknown_key_a_bad_value_and_an_empty_directive_are_errors_with_positions() {
    let doc = "<!-- slopvac: contexts=code -->\n<!-- slopvac: locale=fr -->\n<!-- slopvac: -->\n";
    let out = read(doc, "a.md", &default_policy());
    let found: Vec<_> = out
        .diagnostics
        .iter()
        .map(|d| (d.code, d.location(doc)))
        .collect();
    assert_eq!(
        found,
        [
            (Code::UnknownKey, (1, 15)),
            (Code::InvalidValue, (2, 22)),
            (Code::MissingKey, (3, 14)),
        ]
    );
}

#[test]
fn a_later_directive_wins_and_both_stay_in_the_history() {
    let doc = "<!-- slopvac: locale=en-US -->\n<!-- slopvac: locale=en-GB -->\n";
    let out = read(doc, "a.md", &default_policy());
    assert_eq!(out.settings.len(), 2);
    assert_eq!(out.settings[1].value, json!("en-GB"));
    assert_eq!(out.settings[1].source.line, Some(2));
}

#[test]
fn disabled_front_matter_turns_directives_off_too() {
    let policy = Policy {
        enabled: false,
        allow: vec![],
    };
    let out = read("<!-- slopvac: locale=fr -->\n", "a.md", &policy);
    assert_eq!((out.settings.len(), out.diagnostics.len()), (0, 0));
}

#[test]
fn directives_sit_above_front_matter_and_below_flags() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    std::fs::write(
        root.join("slopvac.toml"),
        "schema_version = 3\nlocale = \"en-US\"\n",
    )
    .unwrap();
    let cfg = load(&root.join("slopvac.toml"), &NoPresets).unwrap();
    let policy = Policy::from_config(&cfg);
    let front = slopvac_config::frontmatter::read(
        "---\nslopvac:\n  v: 3\n  locale: und\n---\n",
        "a.md",
        &policy,
    );
    let dirs = read("<!-- slopvac: locale=en-GB -->\n", "a.md", &policy);
    let cli = [slopvac_config::Setting::bare(
        Layer::Cli,
        &["locale"],
        json!("en-US"),
    )];
    let inputs = |cli: &'static [slopvac_config::Setting]| Inputs {
        presets: &NoPresets,
        front_matter: &front.settings,
        directives: &dirs.settings,
        cli,
        env: &[],
    };
    let r = cfg.resolve(Path::new("a.md"), &inputs(&[]));
    assert_eq!(r.get(&["locale"]), Some(&json!("en-GB")));
    assert_eq!(
        r.source(&["locale"]).map(|s| s.layer),
        Some(Layer::Directive)
    );
    drop(cli);
}
