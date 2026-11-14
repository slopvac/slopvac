//! `terms suggest`: candidates listed with evidence, written only on request.

use std::path::{Path, PathBuf};

use slopvac_terms::cli::{command, run};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "slopvac-terms-suggest-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn terms(dir: &Path, cli_args: &[&str]) -> (i32, String) {
    let mut argv = vec!["terms"];
    argv.extend_from_slice(cli_args);
    let m = command().try_get_matches_from(argv).unwrap();
    let mut buf = Vec::new();
    let code = run(&m, dir, &mut buf).unwrap();
    (code, String::from_utf8(buf).unwrap())
}

const DOC: &str = "\
# Acme Platform

Acme builds tools. We deploy the Acme Platform with FluxCD and FluxCD syncs it.
Use QZX budgets: the QZX is enforced. We also run Quarkus. Quarkus is fast.
The `ZetaCorp` name in code is skipped.
";

#[test]
fn lists_candidates_with_counts_files_and_first_location() {
    let dir = scratch("list");
    std::fs::write(dir.join("a.md"), DOC).unwrap();
    std::fs::write(dir.join("b.md"), "FluxCD again. Nothing else.\n").unwrap();
    let (code, out) = terms(&dir, &["suggest", "a.md", "b.md"]);
    assert_eq!(code, 0);
    // Worked by hand, line by line:
    // - FluxCD: twice on a.md line 3 (first at col 53) and once in b.md.
    // - Platform: the heading "# Acme Platform" (col 8) and line 3.
    // - QZX: twice on line 4, first at col 5.
    // - Acme starts a sentence in the heading and on line 3, and appears
    //   mid-sentence once, so it stays under the minimum of 2.
    // - Quarkus is mid-sentence once and starts a sentence once.
    // - ZetaCorp is in a code span.
    assert_eq!(
        out,
        "FluxCD\t3\t2\ta.md:3:53\nPlatform\t2\t1\ta.md:1:8\nQZX\t2\t1\ta.md:4:5\n"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn known_terms_and_the_min_option_filter_candidates() {
    let dir = scratch("known");
    std::fs::write(
        dir.join("a.md"),
        "We use GitHub and Kubernetes and MyProduct and MyProduct.\n",
    )
    .unwrap();
    let (_, out) = terms(&dir, &["suggest", "a.md"]);
    assert_eq!(out, "MyProduct\t2\t1\ta.md:1:34\n");
    let (_, out) = terms(&dir, &["suggest", "a.md", "--min", "3"]);
    assert_eq!(out, "");
    terms(&dir, &["add", "MyProduct"]);
    let (_, out) = terms(&dir, &["suggest", "a.md"]);
    assert_eq!(out, "");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn nothing_is_written_without_accept() {
    let dir = scratch("no-write");
    std::fs::write(dir.join("a.md"), "We use MyProduct and MyProduct.\n").unwrap();
    terms(&dir, &["suggest", "a.md"]);
    assert!(!dir.join(".slopvac/terms.toml").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn accept_adds_only_the_named_suggestions() {
    let dir = scratch("accept");
    std::fs::write(
        dir.join("a.md"),
        "We use MyProduct and MyProduct, also OtherThing and OtherThing.\n",
    )
    .unwrap();
    let (code, out) = terms(&dir, &["suggest", "a.md", "--accept", "MyProduct"]);
    assert_eq!((code, out.as_str()), (0, "added MyProduct\n"));
    let text = std::fs::read_to_string(dir.join(".slopvac/terms.toml")).unwrap();
    assert!(text.contains("term = \"MyProduct\""), "{text}");
    assert!(!text.contains("OtherThing"), "{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn accepting_something_that_is_not_a_suggestion_changes_nothing() {
    let dir = scratch("accept-bad");
    std::fs::write(dir.join("a.md"), "We use MyProduct and MyProduct.\n").unwrap();
    let m = command()
        .try_get_matches_from([
            "terms",
            "suggest",
            "a.md",
            "--accept",
            "MyProduct",
            "--accept",
            "Nope",
        ])
        .unwrap();
    let e = run(&m, &dir, &mut Vec::new()).unwrap_err().to_string();
    assert!(e.contains("`Nope` is not a suggestion"), "{e}");
    assert!(!dir.join(".slopvac/terms.toml").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}
