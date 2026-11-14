//! The bundled terms list.

use std::collections::BTreeSet;

use slopvac_terms::{TermSet, TermsFile};

/// Ordinary words that the list must not carry at all.
const EXCLUDED: &[&str] = &[
    "go",
    "rust",
    "swift",
    "ruby",
    "react",
    "windows",
    "spark",
    "express",
    "rails",
    "yarn",
    "jest",
    "pandas",
    "vault",
    "rest",
    "ml",
    "os",
    "apple",
    "angular",
    "mocha",
    "slack",
    "notion",
    "bun",
    "snowflake",
    "ram",
    "id",
    "ok",
    "cd",
    "pr",
    "db",
    "vm",
    "ip",
    "nat",
    "spa",
    "soap",
    "helm",
    "pip",
    "bash",
    "safari",
    "edge",
    "opera",
    "gemini",
    "cargo",
    "spring",
    "prettier",
    "playwright",
    "cypress",
    "babel",
    "arm",
    "elm",
    "dart",
    "julia",
];

/// Names that are also ordinary words or personal names. The list may carry
/// them only as case-sensitive terms, so everyday prose never matches.
const ORDINARY: &[&str] = &[
    "mongoose",
    "flutter",
    "ionic",
    "hibernate",
    "anaconda",
    "grub",
    "chromium",
    "kinesis",
    "terraform",
    "spinnaker",
    "rancher",
    "cilium",
    "tableau",
    "looker",
    "cosign",
    "redshift",
    "anthropic",
    "radius",
    "raid",
    "urn",
    "crud",
    "redux",
    "markdown",
    "red hat",
    "new relic",
    "active directory",
    "beautiful soup",
    "python",
    "java",
    "perl",
    "scala",
    "lua",
    "elixir",
    "erlang",
    "haskell",
    "clojure",
    "sass",
    "zig",
    "delphi",
    "prolog",
    "solidity",
    "vyper",
    "selenium",
    "storybook",
    "maven",
    "homebrew",
    "confluence",
    "postman",
    "fedora",
    "vagrant",
    "swagger",
    "azure",
    "chrome",
    "git",
    "docker",
    "consul",
    "packer",
    "jenkins",
    "claude",
    "kafka",
    "redis",
    "cassandra",
    "android",
    "svelte",
    "deno",
    "prometheus",
    "oracle",
    "clang",
    "presto",
];

#[test]
fn builtin_list_meets_the_size_requirement() {
    let n = TermsFile::builtin().terms.len();
    assert!((500..=1500).contains(&n), "{n} terms");
    assert_eq!(TermSet::builtin().len(), n);
}

#[test]
fn builtin_has_no_excluded_words() {
    let file = TermsFile::builtin();
    let bad: Vec<_> = file
        .terms
        .iter()
        .flat_map(|t| std::iter::once(&t.term).chain(t.variants.iter()))
        .filter(|s| EXCLUDED.contains(&s.to_lowercase().as_str()))
        .collect();
    assert_eq!(bad, Vec::<&String>::new());
}

#[test]
fn ordinary_word_names_are_case_sensitive() {
    for t in TermsFile::builtin().terms {
        if ORDINARY.contains(&t.term.to_lowercase().as_str()) {
            assert!(t.case_sensitive, "{} must be case_sensitive", t.term);
        }
    }
}

#[test]
fn lowercase_brands_keep_their_spelling_and_accept_a_sentence_start() {
    let mut checked = 0;
    for t in TermsFile::builtin().terms {
        let all_lower = !t.term.chars().any(char::is_uppercase);
        if all_lower && !t.case_sensitive {
            let cap: String = t
                .term
                .chars()
                .enumerate()
                .map(|(i, c)| if i == 0 { c.to_ascii_uppercase() } else { c })
                .collect();
            assert!(t.variants.contains(&cap), "{} needs variant {cap}", t.term);
            checked += 1;
        }
    }
    assert!(checked >= 15, "only {checked} lowercase brands");
}

#[test]
fn builtin_spellings_do_not_collide_case_insensitively() {
    let mut seen = BTreeSet::new();
    for t in TermsFile::builtin().terms {
        let own: BTreeSet<String> = std::iter::once(t.term.clone())
            .chain(t.variants.clone())
            .map(|s| s.to_lowercase())
            .collect();
        for s in &own {
            assert!(!seen.contains(s), "duplicate spelling {s}");
        }
        seen.extend(own);
    }
}

#[test]
fn builtin_flags_common_misspellings() {
    let set = TermSet::builtin();
    let text = "We use Github, javascript and Postgres with kubernetes on Linux.";
    let found: Vec<_> = set
        .check_casing(text, &[])
        .into_iter()
        .map(|f| (f.found, f.expected))
        .collect();
    assert_eq!(
        found,
        [
            ("Github".to_owned(), "GitHub".to_owned()),
            ("javascript".to_owned(), "JavaScript".to_owned()),
            ("kubernetes".to_owned(), "Kubernetes".to_owned()),
        ]
    );
}

#[test]
fn lowercase_brand_accepts_its_sentence_start_and_flags_shouting() {
    let set = TermSet::builtin();
    assert_eq!(set.check_casing("npm installs it. Npm caches it.", &[]), []);
    let f = set.check_casing("Run WEBPACK now.", &[]);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].expected, "webpack");
}

#[test]
fn ordinary_prose_produces_no_findings() {
    let prose = "The python slid off the azure ledge. She packed a git of lunch, \
        took a chrome kettle, and a maven's advice. In the confluence of rivers the \
        fedora wore a jenkins hat; the oracle spoke of postman visits. I like \
        java and the sass of cats, an android, a docker at the port, and kafka's \
        novels. Redis and cassandra stories, a consul or a packer, vagrant swagger. \
        The mongoose fled; a flutter of wings, an ionic bond, to hibernate in an \
        anaconda's coil. Grub and chromium, a kinesis lesson, terraform the moon. \
        A spinnaker sail, rancher hands, a cilium cell, tableau vivant, a looker, \
        cosign the loan, redshift in light, anthropic principle, the radius of a \
        circle, a raid, an urn, crud on the boots, redux of the film, a markdown \
        sale, the red hat, new relic found, active directory of tasks, beautiful soup.";
    assert_eq!(TermSet::builtin().check_casing(prose, &[]), []);
}
