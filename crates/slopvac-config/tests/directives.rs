//! Inline directives for documents without front matter, through the public interface.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "test code: a panic is a failed test"
)]

mod common;

use common::NoPresets;
use serde_json::json;
use slopvac_config::directives::{find, read};
use slopvac_config::frontmatter::Policy;
use slopvac_config::{Code, ConfigLayer, Inputs, load};
use std::num::NonZeroUsize;
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
            (
                ConfigLayer::Directive,
                NonZeroUsize::new(line),
                NonZeroUsize::new(col)
            ),
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
    assert_eq!(out.settings[0].source.column, NonZeroUsize::new(26));
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
    assert_eq!(out.settings[1].source.line, NonZeroUsize::new(2));
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
        ConfigLayer::Cli,
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
        Some(ConfigLayer::Directive)
    );
    drop(cli);
}

fn found(doc: &str, file: &str) -> usize {
    find(doc, file).len()
}

#[test]
fn markdown_code_blocks_are_examples_not_directives() {
    let directive = "<!-- slopvac: locale=en-GB -->\n";
    let cases = [
        ("backtick fence", format!("```\n{directive}```\n"), 0),
        ("tilde fence", format!("~~~\n{directive}~~~\n"), 0),
        ("info string", format!("```markdown\n{directive}```\n"), 0),
        (
            "longer closing fence",
            format!("```\n{directive}`````\n{directive}"),
            1,
        ),
        (
            "shorter fence does not close",
            format!("````\n```\n{directive}````\n{directive}"),
            1,
        ),
        ("indented code", format!("    {directive}"), 0),
        ("tab indented code", format!("\t{directive}"), 0),
        (
            "inline code",
            "`<!-- slopvac: locale=en-GB -->`\n".to_owned(),
            0,
        ),
        (
            "after a closed fence",
            format!("```\ncode\n```\n{directive}"),
            1,
        ),
        ("plain", directive.to_owned(), 1),
    ];
    for (name, doc, want) in cases {
        assert_eq!(found(&doc, "a.md"), want, "{name}");
    }
}

#[test]
fn html_raw_elements_hold_text_not_directives() {
    let d = "<!-- slopvac: locale=en-GB -->\n";
    assert_eq!(found(&format!("<pre>\n{d}</pre>\n"), "a.html"), 0);
    assert_eq!(found(&format!("<script>\n{d}</script>\n{d}"), "a.html"), 1);
}

#[test]
fn restructuredtext_literal_blocks_hold_text_not_directives() {
    let d = ".. slopvac: locale=en-GB\n";
    let cases = [
        ("literal block", format!("Example::\n\n    {d}\n"), 0),
        ("code-block", format!(".. code-block:: rst\n\n   {d}\n"), 0),
        (
            "after the block",
            format!("Example::\n\n    code\n\n{d}"),
            1,
        ),
        ("plain", d.to_owned(), 1),
    ];
    for (name, doc, want) in cases {
        assert_eq!(found(&doc, "a.rst"), want, "{name}");
    }
}

#[test]
fn multi_line_strings_in_code_hold_data_not_directives() {
    let cases = [
        (
            "python docstring",
            "\"\"\"\n# slopvac: locale=fr\n\"\"\"\n",
            "a.py",
            0,
        ),
        (
            "single line docstring",
            "\"\"\"doc\"\"\"\n# slopvac: locale=en-GB\n",
            "a.py",
            1,
        ),
        (
            "after a docstring",
            "\"\"\"\ntext\n\"\"\"\n# slopvac: locale=en-GB\n",
            "a.py",
            1,
        ),
        (
            "template literal",
            "const s = `\n// slopvac: locale=fr\n`;\n",
            "a.js",
            0,
        ),
        (
            "comment with one backtick",
            "// it`s fine\n// slopvac: locale=en-GB\n",
            "a.js",
            1,
        ),
    ];
    for (name, doc, file, want) in cases {
        assert_eq!(found(doc, file), want, "{name}");
    }
}

#[test]
fn a_directive_in_an_example_does_not_change_the_settings() {
    let doc = "```\n<!-- slopvac: locale=fr -->\n```\n<!-- slopvac: locale=en-GB -->\n";
    let out = read(doc, "a.md", &default_policy());
    assert_eq!(out.diagnostics, []);
    assert_eq!(out.settings.len(), 1);
    assert_eq!(out.settings[0].source.line, NonZeroUsize::new(4));
}

#[test]
fn a_directive_can_pick_the_genre_that_supplies_the_chain() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    std::fs::write(root.join("slopvac.toml"), "schema_version = 3\n").unwrap();
    let cfg = load(&root.join("slopvac.toml"), &Genres).unwrap();
    let policy = Policy::from_config(&cfg);
    let dirs = read("<!-- slopvac: genre=reference -->\n", "a.md", &policy);
    let r = cfg.resolve(
        Path::new("a.md"),
        &Inputs {
            presets: &Genres,
            front_matter: &[],
            directives: &dirs.settings,
            cli: &[],
            env: &[],
        },
    );
    assert_eq!(r.get(&["locale"]), Some(&json!("en-GB")));
}

struct Genres;

impl slopvac_config::Presets for Genres {
    fn style(&self, _name: &str) -> Option<Vec<slopvac_config::Setting>> {
        None
    }

    fn genre_chain(&self, name: &str) -> Vec<Vec<slopvac_config::Setting>> {
        if name == "reference" {
            vec![vec![slopvac_config::Setting::bare(
                ConfigLayer::Genre,
                &["locale"],
                json!("en-GB"),
            )]]
        } else {
            Vec::new()
        }
    }

    fn pack_config(&self, _name: &str) -> Option<slopvac_config::PackFile> {
        None
    }
}
#[test]
fn rust_raw_strings_character_literals_and_lifetimes_do_not_confuse_the_scan() {
    let cases = [
        (
            "raw string",
            "let s = r#\"\n// slopvac: locale=fr\n\"#;\n// slopvac: locale=en-GB\n",
            1,
        ),
        (
            "raw string with hashes inside",
            "let s = r##\"a\"#b\n// slopvac: locale=fr\n\"##;\n",
            0,
        ),
        (
            "byte raw string",
            "let s = br#\"\n// slopvac: locale=fr\n\"#;\n",
            0,
        ),
        (
            "string spanning lines",
            "let s = \"a\n// slopvac: locale=fr\nb\";\n",
            0,
        ),
        (
            "quote character literal",
            "let q = '\"';\n// slopvac: locale=en-GB\n",
            1,
        ),
        (
            "escaped character literal",
            "let q = '\\'';\n// slopvac: locale=en-GB\n",
            1,
        ),
        (
            "lifetime",
            "fn f<'a>(x: &'a str) {}\n// slopvac: locale=en-GB\n",
            1,
        ),
        (
            "nested block comment",
            "/* a /* b */ // slopvac: locale=fr */\n",
            0,
        ),
        (
            "identifier ending in r",
            "let bar = 1;\n// slopvac: locale=en-GB\n",
            1,
        ),
    ];
    for (name, doc, want) in cases {
        assert_eq!(found(doc, "lib.rs"), want, "{name}");
    }
}

#[test]
fn javascript_strings_and_templates_do_not_hide_or_fake_comments() {
    let cases = [
        (
            "backtick in a string",
            "const s = \"`\";\n// slopvac: locale=en-GB\n",
            1,
        ),
        (
            "backtick in a comment",
            "// don`t\n// slopvac: locale=en-GB\n",
            1,
        ),
        (
            "template literal",
            "const s = `\n// slopvac: locale=fr\n`;\n// slopvac: locale=en-GB\n",
            1,
        ),
        (
            "template with an expression",
            "const s = `a ${ \"`\" } b\n// slopvac: locale=fr\n`;\n",
            0,
        ),
        (
            "nested template",
            "const s = `a ${ `b` } c\n// slopvac: locale=fr\n`;\n",
            0,
        ),
        (
            "escaped backtick",
            "const s = `a \\` b\n// slopvac: locale=fr\n`;\n",
            0,
        ),
        (
            "single quote in a string",
            "const s = \"it's\";\n// slopvac: locale=en-GB\n",
            1,
        ),
    ];
    for (name, doc, want) in cases {
        assert_eq!(found(doc, "a.ts"), want, "{name}");
    }
    assert_eq!(
        found("const s = `\n// slopvac: locale=fr\n`;\n", "main.go"),
        0
    );
}

#[test]
fn python_strings_and_apostrophes_do_not_hide_or_fake_comments() {
    let cases = [
        ("docstring", "\"\"\"\n# slopvac: locale=fr\n\"\"\"\n", 0),
        (
            "single quoted triple",
            "'''\n# slopvac: locale=fr\n'''\n",
            0,
        ),
        (
            "apostrophe in a comment",
            "# don't\n# slopvac: locale=en-GB\n",
            1,
        ),
        ("hash in a string", "x = '#'\n# slopvac: locale=en-GB\n", 1),
        (
            "after a docstring",
            "\"\"\"doc\n\"\"\"\n# slopvac: locale=en-GB\n",
            1,
        ),
    ];
    for (name, doc, want) in cases {
        assert_eq!(found(doc, "a.py"), want, "{name}");
    }
}

#[test]
fn a_directive_must_be_the_first_thing_on_its_line() {
    assert_eq!(found("x = 1 // slopvac: locale=en-GB\n", "a.rs"), 0);
    assert_eq!(found("    // slopvac: locale=en-GB\n", "a.rs"), 1);
    assert_eq!(found("x = 1  # slopvac: locale=en-GB\n", "a.py"), 0);
}

#[test]
fn block_comment_directives_stop_before_the_closing_marker() {
    let out = read(
        "/* slopvac: locale=en-GB */\nint x;\n",
        "a.c",
        &default_policy(),
    );
    assert_eq!(out.diagnostics, []);
    assert_eq!(out.settings[0].value, json!("en-GB"));
    assert_eq!(found("/* slopvac: locale=en-GB\n */\n", "a.c"), 1);
}

#[test]
fn comments_inside_template_interpolations_and_escaped_triple_quotes_are_handled() {
    let cases = [
        (
            "comment with a backtick in an interpolation",
            "const s = `a ${ x /* ` */ } b\n// slopvac: locale=fr\n`;\n",
            "a.js",
            0,
        ),
        (
            "line comment with a brace in an interpolation",
            "const s = `${ a // }\n}\n// slopvac: locale=fr\n`;\n",
            "a.js",
            0,
        ),
        (
            "escaped triple double quote",
            "\"\"\"a \\\"\"\" b\n# slopvac: locale=fr\n\"\"\"\n",
            "a.py",
            0,
        ),
        (
            "escaped triple single quote",
            "'''a \\''' b\n# slopvac: locale=fr\n'''\n",
            "a.py",
            0,
        ),
        (
            "code after a real close",
            "\"\"\"a\"\"\"\n# slopvac: locale=en-GB\n",
            "a.py",
            1,
        ),
        (
            "rust escaped quote then string",
            "let q = '\\''; let s = \"x\";\n// slopvac: locale=en-GB\n",
            "a.rs",
            1,
        ),
        (
            "rust unicode escape",
            "let c = '\\u{1F600}';\n// slopvac: locale=en-GB\n",
            "a.rs",
            1,
        ),
        (
            "go raw string has no escapes",
            "const s = `a \\`\n// slopvac: locale=en-GB\n",
            "a.go",
            1,
        ),
        (
            "go raw string",
            "const s = `\n// slopvac: locale=fr\n`\n",
            "a.go",
            0,
        ),
        ("shell", "echo '#'\n# slopvac: locale=en-GB\n", "a.sh", 1),
        ("lua", "x = 1\n-- slopvac: locale=en-GB\n", "a.lua", 1),
        (
            "build file by name",
            "# slopvac: locale=en-GB\n",
            "Rakefile",
            1,
        ),
    ];
    for (name, doc, file, want) in cases {
        assert_eq!(found(doc, file), want, "{name}");
    }
}
#[test]
fn regressions_that_the_first_scanner_failed() {
    let cases = [
        // A comment inside `${ … }` holds both a brace and a backtick; neither ends anything.
        (
            "brace and backtick in an interpolation comment",
            "const s = `a ${ x /* } ` */ y } b\n// slopvac: locale=fr\n`;\n",
            "a.js",
            0,
        ),
        // The template ends after the interpolation, so the next comment is real.
        (
            "real comment after a template with a commented backtick",
            "const s = `${ a /* ` */ }`;\n// slopvac: locale=en-GB\n",
            "a.js",
            1,
        ),
        // Two `'\''` literals and a string that holds an apostrophe, with no separator.
        (
            "adjacent character literals",
            "m!('\\'''\\''\"'\");\n// slopvac: locale=en-GB\n",
            "a.rs",
            1,
        ),
        // A line comment inside `${ … }` holds a backtick; the template still ends at its own.
        (
            "backtick in an interpolation line comment",
            "const s = `${ x // `\n }`;\n// slopvac: locale=en-GB\n",
            "a.js",
            1,
        ),
        // SConstruct is Python; its docstring is data.
        (
            "build file docstring",
            "\"\"\"\n# slopvac: locale=fr\n\"\"\"\n",
            "SConstruct",
            0,
        ),
        (
            "build file comment",
            "# slopvac: locale=en-GB\n",
            "SConstruct",
            1,
        ),
    ];
    for (name, doc, file, want) in cases {
        assert_eq!(found(doc, file), want, "{name}");
    }
}

/// One directive line of 31 bytes that sets one setting.
const LOCALE_LINE: &str = "<!-- slopvac: locale=en-GB -->\n";

#[test]
fn directives_past_the_cap_that_grows_with_the_document_are_refused() {
    // 20 000 lines of 31 bytes make 620 000 bytes, so the cap is 1024 + 620 000 / 32 = 20 399.
    // The lines hold 20 000 directives and 20 000 settings, which is 40 000 units.
    let doc = LOCALE_LINE.repeat(20_000);
    let read = read(&doc, "doc.md", &default_policy());
    assert_eq!(read.settings, Vec::new());
    let codes: Vec<Code> = read.diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(codes, [Code::InvalidValue]);
    assert_eq!(
        read.diagnostics[0].message,
        "the list holds more than 20399 entries"
    );
}

#[test]
fn directives_at_an_ordinary_density_stay_under_the_cap() {
    // 500 lines make 15 500 bytes, so the cap is 1024 + 484 = 1508. The lines use 1 000 units.
    let doc = LOCALE_LINE.repeat(500);
    let read = read(&doc, "doc.md", &default_policy());
    assert_eq!(read.diagnostics, Vec::new());
    assert_eq!(read.settings.len(), 500);
}
