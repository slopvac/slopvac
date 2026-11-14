//! Behavior of the casing check, term files and blocklists through the public API.

use slopvac_terms::{Blocklist, Pos, TermEntry, TermSet, TermsError, TermsFile};

fn term(t: &str) -> TermEntry {
    TermEntry {
        term: t.to_owned(),
        variants: vec![],
        kind: None,
        ignore_case: false,
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
    let skip = vec![4..12];
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
        ("schema_version = 3\nfoo = 1\n", "unknown key `foo`"),
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
    assert!(TermsFile::parse("t.toml", "schema_version = 3\nx-note = 1\n").is_ok());
}

const BLOCK: &str = r#"
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
    let json = r#"{"entries":[{"word":"leverage","pos":"verb","reason":"Say use."}]}"#;
    let b = Blocklist::parse("b.json", "json", json).unwrap();
    assert!(b.lookup("leverage", Pos::Verb).is_some());
}

#[test]
fn blocklist_rejects_bad_files() {
    let cases = [
        (
            "[[words]]\nword = \"a\"\n",
            "no `entries` array; it defines `words`",
        ),
        (
            "[[entries]]\nword = \"a\"\npos = \"verb\"\n",
            "has no `reason`",
        ),
        (
            "[[entries]]\nword = \"a\"\npos = \"zzz\"\nreason = \"r\"\n",
            "unrecognized `pos`",
        ),
        (
            "[[entries]]\npos = \"verb\"\nreason = \"r\"\n",
            "has no `word`",
        ),
        (
            "[[entries]]\nword = \"a\"\npos = \"verb\"\nreason = \"r\"\n[[entries]]\nword = \"A\"\npos = \"v\"\nreason = \"r\"\n",
            "duplicate entry `a`",
        ),
        (
            "[[entries]]\nword = \"a\"\npos = \"verb\"\nreason = \"r\"\nextra = 1\n",
            "unknown field",
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
