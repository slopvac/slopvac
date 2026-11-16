//! Behavior of the casing check, term files and blocklists through the public API.

use slopvac_terms::{Blocklist, Pos, TermEntry, TermSet, TermsError, TermsFile};

fn term(t: &str) -> TermEntry {
    TermEntry {
        term: t.to_owned(),
        variants: vec![],
        kind: None,
        ignore_case: false,
        case_sensitive: false,
    }
}

fn set() -> TermSet {
    let mut k8s = term("Kubernetes");
    k8s.variants = vec!["K8s".to_owned()];
    TermSet::new([term("GitHub"), k8s, term("Node.js"), term("C++")])
}

fn found(text: &str) -> Vec<(String, String)> {
    set()
        .check_casing(text, &[])
        .into_iter()
        .map(|f| (f.found, f.expected))
        .collect()
}

#[test]
fn flags_a_wrong_case_with_the_canonical_spelling() {
    assert_eq!(
        found("We moved to Github last year."),
        [("Github".to_owned(), "GitHub".to_owned())]
    );
}

#[test]
fn reports_the_byte_span_of_the_term() {
    let f = set().check_casing("see github now", &[]);
    assert_eq!(f[0].span, 4..10);
}

#[test]
fn canonical_spelling_is_clean() {
    assert_eq!(found("GitHub and Node.js and C++ are fine."), []);
}

#[test]
fn declared_variant_is_clean_and_other_cases_are_flagged() {
    assert_eq!(found("Use K8s."), []);
    assert_eq!(
        found("Use k8s."),
        [("k8s".to_owned(), "Kubernetes".to_owned())]
    );
}

#[test]
fn plurals_and_possessives_need_no_declaration() {
    assert_eq!(found("GitHubs and GitHub's and GitHub\u{2019}s"), []);
    assert_eq!(
        found("github's repo"),
        [("github".to_owned(), "GitHub".to_owned())]
    );
}

#[test]
fn matches_inside_words_are_ignored() {
    assert_eq!(found("githubber and mygithub and github_token"), []);
}

#[test]
fn paths_domains_and_handles_are_ignored() {
    assert_eq!(
        found("see github.com/org and docs/github and @github and .github/workflows"),
        []
    );
}

#[test]
fn sentence_final_period_still_matches() {
    assert_eq!(found("We use github.").len(), 1);
}

#[test]
fn terms_with_punctuation_match_whole() {
    assert_eq!(
        found("node.js, c++ and ASP.NET"),
        [
            ("node.js".to_owned(), "Node.js".to_owned()),
            ("c++".to_owned(), "C++".to_owned())
        ]
    );
}

#[test]
fn ignore_case_terms_accept_any_case() {
    let mut t = term("OAuth");
    t.ignore_case = true;
    let s = TermSet::new([t]);
    assert_eq!(s.check_casing("oauth OAUTH OAuth", &[]), []);
}

#[test]
fn skip_ranges_hide_code_spans() {
    let text = "run `github` and github";
    let skip = std::iter::once(4..12).collect::<Vec<_>>();
    let f = set().check_casing(text, &skip);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].span, 17..23);
}

#[test]
fn finds_every_occurrence_in_order() {
    let f = set().check_casing("github, GITHUB, GitHub, Github", &[]);
    assert_eq!(
        f.iter().map(|x| x.span.start).collect::<Vec<_>>(),
        [0, 8, 24]
    );
}

#[test]
fn non_ascii_text_does_not_break_spans() {
    let f = set().check_casing("naïve — github", &[]);
    assert_eq!(f[0].span, 11..17);
    assert_eq!(&"naïve — github"[f[0].span.clone()], "github");
}

const FILE: &str = r#"
schema_version = 3
[[terms]]
term = "GitHub"
kind = "product"

[[terms]]
term = "Kubernetes"
variants = ["K8s"]
"#;

#[test]
fn terms_file_needs_only_term() {
    let f = TermsFile::parse("t.toml", FILE).unwrap();
    assert_eq!(f.terms.len(), 2);
    assert_eq!(f.terms[0].variants.len(), 0);
    assert_eq!(f.terms[1].variants, ["K8s"]);
    assert_eq!(TermSet::from_file(&f).len(), 2);
}

#[test]
fn terms_file_rejects_bad_input() {
    let cases = [
        ("schema_version = 2\n", "schema_version 2"),
        (
            "schema_version = 3\nterms = []\nfoo = 1\n",
            "unknown key `foo`",
        ),
        ("schema_version = 3\n", "missing `terms`"),
        (
            "schema_version = 3\n[[terms]]\nterm = \"A\"\ncase = \"exact\"\n",
            "unknown key `terms[1].case`",
        ),
        (
            "schema_version = 3\n[[terms]]\nterm = \"A\"\nkind = \"Proper Noun\"\n",
            "kebab-case",
        ),
        (
            "schema_version = 3\n[[terms]]\nterm = \"A\"\nvariants = [\"a\", \"a\"]\n",
            "variant `a` twice",
        ),
        (
            "schema_version = 3\n[[terms]]\nterm = \"A\"\nvariants = [\"\"]\n",
            "empty variant",
        ),
        (
            "schema_version = 3\n[[terms]]\nterm = \"A\"\n[[terms]]\nterm = \"A\"\n",
            "duplicate term `A`",
        ),
        (
            "schema_version = 3\n[[terms]]\nterm = \" \"\n",
            "term 1 is empty",
        ),
    ];
    for (text, needle) in cases {
        let e = TermsFile::parse("t.toml", text).unwrap_err().to_string();
        assert!(e.contains(needle), "{e} should contain {needle}");
    }
}

#[test]
fn terms_file_ignores_x_keys() {
    let text = "schema_version = 3\nx-note = 1\n[[terms]]\nterm = \"A\"\nx-source = \"doc\"\n";
    assert_eq!(TermsFile::parse("t.toml", text).unwrap().terms.len(), 1);
}

const BLOCK: &str = r#"
schema_version = 3

[[entries]]
word = "Utilize"
pos = "v"
reason = "Use `use`."
replacement = "use"

[[entries]]
word = "deploy"
pos = "noun"
reason = "The noun is `deployment`."
"#;

#[test]
fn blocklist_loads_toml_and_looks_up_by_word_and_pos() {
    let b = Blocklist::parse("b.toml", "toml", BLOCK).unwrap();
    assert_eq!(b.len(), 2);
    let e = b.lookup("UTILIZE", Pos::Verb).unwrap();
    assert_eq!(e.reason, "Use `use`.");
    assert_eq!(e.replacement.as_deref(), Some("use"));
    assert!(b.lookup("utilize", Pos::Noun).is_none());
}

#[test]
fn blocklist_loads_json() {
    let json =
        r#"{"schema_version":3,"entries":[{"word":"leverage","pos":"verb","reason":"Say use."}]}"#;
    let b = Blocklist::parse("b.json", "json", json).unwrap();
    assert!(b.lookup("leverage", Pos::Verb).is_some());
}

#[test]
fn blocklist_rejects_bad_files() {
    let cases = [
        (
            "schema_version = 3\n[[words]]\nword = \"a\"\n",
            "no `entries` array; it defines `words`",
        ),
        (
            "schema_version = 3\n[[entries]]\nword = \"a\"\npos = \"verb\"\n",
            "has no `reason`",
        ),
        (
            "schema_version = 3\n[[entries]]\nword = \"a\"\npos = \"zzz\"\nreason = \"r\"\n",
            "unrecognized `pos`",
        ),
        (
            "schema_version = 3\n[[entries]]\npos = \"verb\"\nreason = \"r\"\n",
            "has no `word`",
        ),
        (
            "schema_version = 3\n[[entries]]\nword = \"a\"\npos = \"verb\"\nreason = \"r\"\n[[entries]]\nword = \"A\"\npos = \"v\"\nreason = \"r\"\n",
            "duplicate entry `a`",
        ),
        (
            "schema_version = 3\n[[entries]]\nword = \"a\"\npos = \"verb\"\nreason = \"r\"\nextra = 1\n",
            "unknown key `entries[1].extra`",
        ),
    ];
    for (text, needle) in cases {
        let e = Blocklist::parse("b.toml", "toml", text)
            .unwrap_err()
            .to_string();
        assert!(e.contains(needle), "{e} should contain {needle}");
    }
}

#[test]
fn blocklist_rejects_unknown_formats() {
    let e = Blocklist::parse("b.txt", "txt", "").unwrap_err();
    assert!(matches!(e, TermsError::Parse { .. }));
}

fn go_set() -> TermSet {
    let mut go = term("Go");
    go.case_sensitive = true;
    TermSet::new([go, term("GitHub")])
}

#[test]
fn case_sensitive_terms_never_flag_ordinary_words() {
    let text = "We go to the store, then go home. GO team. Go is a language.";
    assert_eq!(go_set().check_casing(text, &[]), []);
}

#[test]
fn wrong_case_plurals_report_the_term_without_the_suffix() {
    let cases = [
        ("Githubs", 0..6, "Github"),
        ("GITHUBS", 0..6, "GITHUB"),
        ("github's", 0..6, "github"),
        ("GITHUB'S", 0..6, "GITHUB"),
    ];
    for (text, span, found) in cases {
        let f = set().check_casing(text, &[]);
        assert_eq!(f.len(), 1, "{text}");
        assert_eq!(f[0].span, span, "{text}");
        assert_eq!(f[0].found, found, "{text}");
    }
}

#[test]
fn whole_token_decides_path_and_handle_exclusion() {
    for text in [
        "docs/my-github-repo",
        "@my-github",
        "https://example.com/github-actions",
        "see github.io for more",
        "org/github",
        "me@github.com",
    ] {
        assert_eq!(found(text), [], "{text}");
    }
}

#[test]
fn hyphenated_prose_still_matches() {
    let f = set().check_casing("a github-hosted repo", &[]);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].span, 2..8);
}

#[test]
fn closing_unicode_punctuation_does_not_hide_a_term() {
    let cases = [
        "\u{201c}github.\u{201d}",
        "\u{2018}github.\u{2019}",
        "\u{00ab}github.\u{00bb}",
        "(github.)",
        "github\u{2026}",
        "\u{201c}github\u{201d}",
    ];
    for text in cases {
        let f = set().check_casing(text, &[]);
        assert_eq!(f.len(), 1, "{text}");
        assert_eq!(&text[f[0].span.clone()], "github", "{text}");
    }
}

#[test]
fn quoted_and_bracketed_terms_still_match() {
    assert_eq!(found("(github), \"github\" and github!").len(), 3);
}

#[test]
fn ignore_case_and_case_sensitive_together_is_an_error() {
    let text =
        "schema_version = 3\n[[terms]]\nterm = \"Go\"\nignore_case = true\ncase_sensitive = true\n";
    let e = TermsFile::parse("t.toml", text).unwrap_err().to_string();
    assert!(e.contains("both ignore_case and case_sensitive"), "{e}");
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("slopvac-terms-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn terms_file_loads_from_disk_and_reports_read_failures() {
    let dir = scratch("terms");
    let ok = dir.join("terms.toml");
    std::fs::write(&ok, FILE).unwrap();
    assert_eq!(TermsFile::load(&ok).unwrap().terms.len(), 2);

    let missing = TermsFile::load(&dir.join("missing.toml")).unwrap_err();
    assert!(matches!(missing, TermsError::Io { .. }));
    let directory = TermsFile::load(&dir).unwrap_err();
    assert!(matches!(directory, TermsError::Io { .. }));
    let bad = dir.join("bad.toml");
    std::fs::write(&bad, [0xff, 0xfe, 0x00]).unwrap();
    let invalid = TermsFile::load(&bad).unwrap_err();
    assert!(matches!(invalid, TermsError::Io { .. }));
    std::fs::remove_dir_all(&dir).unwrap();
}

const YAML_BLOCK: &str = "schema_version: 3
entries:
  - word: Utilize
    pos: v
    reason: Use `use`.
    replacement: use
  - word: deploy
    pos: noun
    reason: The noun is `deployment`.
";

#[test]
fn blocklist_loads_yml_and_yaml_files() {
    let dir = scratch("blocklist-yaml");
    for name in ["b.yml", "b.YAML"] {
        let path = dir.join(name);
        std::fs::write(&path, YAML_BLOCK).unwrap();
        let b = Blocklist::load(&path).unwrap();
        assert_eq!(b.len(), 2, "{name}");
        let e = b.lookup("utilize", Pos::Verb).unwrap();
        assert_eq!(e.reason, "Use `use`.");
        assert_eq!(e.replacement.as_deref(), Some("use"));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn yaml_blocklist_entries_get_the_same_validation() {
    let text = "schema_version: 3\nentries:\n  - word: leverage\n    pos: verb\n";
    let e = Blocklist::parse("b.yml", "yml", text).unwrap_err();
    assert!(matches!(e, TermsError::Entry { .. }), "{e}");
    assert!(e.to_string().contains("no `reason`"), "{e}");
    let e = Blocklist::parse("b.yml", "yaml", "schema_version: 3\nother: 1\n").unwrap_err();
    assert!(e.to_string().contains("no `entries` array"), "{e}");
}

#[test]
fn invalid_yaml_blocklist_names_the_file() {
    let dir = scratch("blocklist-bad-yaml");
    let path = dir.join("broken.yaml");
    std::fs::write(&path, "entries: [unclosed\n").unwrap();
    let e = Blocklist::load(&path).unwrap_err();
    assert!(matches!(e, TermsError::Parse { .. }), "{e}");
    assert!(e.to_string().contains("broken.yaml"), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn blocklist_field_types_are_strict_in_every_format() {
    let cases = [
        (
            "toml",
            "schema_version = 3\n[[entries]]\nword = 5\npos = \"n\"\nreason = \"r\"\n",
        ),
        (
            "toml",
            "schema_version = 3\n[[entries]]\nword = \"x\"\npos = \"n\"\nreason = true\n",
        ),
        (
            "json",
            r#"{"schema_version":3,"entries":[{"word":5,"pos":"n","reason":"r"}]}"#,
        ),
        (
            "json",
            r#"{"schema_version":3,"entries":[{"word":"x","pos":"n","reason":true}]}"#,
        ),
        (
            "yaml",
            "schema_version: 3\nentries:\n  - {word: 5, pos: n, reason: r}\n",
        ),
        (
            "yaml",
            "schema_version: 3\nentries:\n  - {word: x, pos: n, reason: true}\n",
        ),
        (
            "yaml",
            "schema_version: 3\nentries:\n  - {word: x, pos: n, reason: r, replacement: 1.5}\n",
        ),
    ];
    for (format, text) in cases {
        let e = Blocklist::parse("b", format, text).unwrap_err();
        assert!(
            matches!(e, TermsError::Parse { .. }),
            "{format}: {text}: {e}"
        );
    }
}

#[test]
fn quoted_scalars_load_from_yaml() {
    let text = "schema_version: 3\nentries:\n  - {word: \"5\", pos: n, reason: \"true\"}\n";
    assert_eq!(Blocklist::parse("b", "yaml", text).unwrap().len(), 1);
}

#[test]
fn blocklist_load_dispatches_on_suffix() {
    let dir = scratch("blocklist");
    let toml = dir.join("b.TOML");
    let json = dir.join("b.json");
    let yaml = dir.join("b.yaml");
    let none = dir.join("blocklist");
    std::fs::write(&toml, BLOCK).unwrap();
    std::fs::write(
        &json,
        r#"{"schema_version":3,"entries":[{"word":"leverage","pos":"verb","reason":"r"}]}"#,
    )
    .unwrap();
    std::fs::write(
        &yaml,
        "schema_version: 3\nentries:\n  - {word: x, pos: n, reason: r}\n",
    )
    .unwrap();
    std::fs::write(&none, BLOCK).unwrap();

    assert_eq!(Blocklist::load(&toml).unwrap().len(), 2);
    assert_eq!(Blocklist::load(&json).unwrap().len(), 1);
    assert_eq!(Blocklist::load(&yaml).unwrap().len(), 1);
    assert!(matches!(
        Blocklist::load(&none).unwrap_err(),
        TermsError::Parse { .. }
    ));
    let gone = Blocklist::load(&dir.join("gone.toml")).unwrap_err();
    assert!(matches!(gone, TermsError::Io { .. }));
    let bad = dir.join("bad.toml");
    std::fs::write(&bad, [0xff, 0xfe]).unwrap();
    let invalid = Blocklist::load(&bad).unwrap_err();
    assert!(matches!(invalid, TermsError::Io { .. }));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn blocklist_without_schema_version_points_to_migrate_in_every_format() {
    let cases = [
        (
            "toml",
            "[[entries]]\nword = \"a\"\npos = \"verb\"\nreason = \"r\"\n",
        ),
        (
            "json",
            r#"{"entries":[{"word":"a","pos":"verb","reason":"r"}]}"#,
        ),
        ("yaml", "entries:\n  - {word: a, pos: verb, reason: r}\n"),
    ];
    for (format, text) in cases {
        let e = Blocklist::parse("b", format, text).unwrap_err();
        assert!(
            matches!(e, TermsError::MissingSchemaVersion { .. }),
            "{format}: {e}"
        );
        let shown = e.to_string();
        assert!(shown.contains("schema_version = 3"), "{shown}");
        assert!(shown.contains("slopvac migrate"), "{shown}");
    }
}

#[test]
fn blocklist_with_another_schema_version_is_rejected_with_a_pointer_to_migrate() {
    let e = Blocklist::parse("b.toml", "toml", "schema_version = 2\nentries = []\n").unwrap_err();
    assert!(
        matches!(e, TermsError::SchemaVersion { ref found, .. } if found == "2"),
        "{e}"
    );
    assert!(e.to_string().contains("slopvac migrate"), "{e}");
}

#[test]
fn blocklist_ignores_x_keys_and_rejects_other_unknown_keys() {
    let ok = "schema_version = 3\nx-note = 1\n[[entries]]\nword = \"a\"\npos = \"verb\"\nreason = \"r\"\nx-source = \"s\"\n";
    assert_eq!(Blocklist::parse("b.toml", "toml", ok).unwrap().len(), 1);
    let top = "schema_version = 3\nextra = 1\nentries = []\n";
    let e = Blocklist::parse("b.toml", "toml", top).unwrap_err();
    assert!(
        matches!(e, TermsError::UnknownKey { ref key, .. } if key == "extra"),
        "{e}"
    );
}

#[test]
fn blocklist_schema_version_must_be_an_integer_in_every_format() {
    let cases = [
        ("toml", "schema_version = \"3\"\nentries = []\n"),
        ("json", r#"{"schema_version":"3","entries":[]}"#),
        ("yaml", "schema_version: \"3\"\nentries: []\n"),
        ("yaml", "schema_version: 3.5\nentries: []\n"),
        ("toml", "schema_version = true\nentries = []\n"),
    ];
    for (format, text) in cases {
        let e = Blocklist::parse("b", format, text).unwrap_err();
        assert!(
            matches!(e, TermsError::SchemaVersion { .. }),
            "{format}: {text}: {e}"
        );
        assert!(e.to_string().contains("slopvac migrate"), "{e}");
    }
}
