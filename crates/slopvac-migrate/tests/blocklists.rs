//! Blocklists named by `vocabulary.path`: the migration rewrites them to the 3.0 format.
//!
//! The 2.x inputs follow what the 2.x loader accepted: no `schema_version`, and a `pos` that
//! it trimmed and lower-cased before matching its 25 listed spellings.

use std::fs;
use std::path::Path;

use rstest::rstest;
use slopvac_migrate::{
    Error, NullLinter, Options, Plan, RenameTable, apply, command, plan, render_diff, run,
};
use slopvac_terms::{Blocklist, Pos};

const STEERING: &str = "<!-- slopvac:begin -->\nnew\n<!-- slopvac:end -->\n";
const CONFIG: &str = "[vocabulary]\npath = \"words.toml\"\n";

/// A 2.x blocklist: no `schema_version`, comments, two entries.
const OLD: &str = "\
# Words we refuse
[[entries]]
word = \"utilize\"
pos = \"verb\"
reason = \"Use `use`.\"
replacement = \"use\"

[[entries]]
word = \"synergy\"
pos = \"n\" # short form
reason = \"Jargon.\"
";

fn write(root: &Path, name: &str, text: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn read(root: &Path, name: &str) -> String {
    fs::read_to_string(root.join(name)).unwrap()
}

fn plan_of(root: &Path) -> Result<Plan, Error> {
    plan(root, &RenameTable::bundled(), Options::default(), STEERING)
}

fn project(config: &str, blocklist: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "slopvac.toml", config);
    write(dir.path(), "words.toml", blocklist);
    dir
}

fn run_cli(args: &[&str]) -> (i32, String, String) {
    let m = command().get_matches_from(std::iter::once("migrate").chain(args.iter().copied()));
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(&m, &NullLinter, STEERING, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

fn dropped_text(error: &Error) -> String {
    assert_eq!(error.code(), "config-legacy-dropped", "{error}");
    error.to_string()
}

#[test]
fn a_2x_blocklist_migrates_and_loads() {
    let dir = project(CONFIG, OLD);
    let p = plan_of(dir.path()).unwrap();
    apply(dir.path(), &p).unwrap();
    assert_eq!(
        read(dir.path(), "words.toml"),
        "\
schema_version = 3
# Words we refuse
[[entries]]
word = \"utilize\"
pos = \"verb\"
reason = \"Use `use`.\"
replacement = \"use\"

[[entries]]
word = \"synergy\"
pos = \"n\" # short form
reason = \"Jargon.\"
"
    );
    assert_eq!(
        read(dir.path(), "slopvac.toml"),
        "schema_version = 3\n\n[terms]\nblocklists = [\"words.toml\"]\n"
    );
    let loaded = Blocklist::load(&dir.path().join("words.toml")).unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(
        loaded.lookup("utilize", Pos::Verb).unwrap().reason,
        "Use `use`."
    );
    assert!(loaded.lookup("synergy", Pos::Noun).is_some());
}

#[test]
fn the_diff_and_notes_show_the_blocklist_change() {
    let dir = project(CONFIG, OLD);
    let p = plan_of(dir.path()).unwrap();
    let diff = render_diff(&p);
    assert!(
        diff.contains("--- a/words.toml\n+++ b/words.toml\n"),
        "{diff}"
    );
    assert!(diff.contains("+schema_version = 3\n"), "{diff}");
    assert!(
        p.notes
            .contains(&"words.toml: added schema_version = 3".to_owned()),
        "{:?}",
        p.notes
    );
    let (code, out, _) = run_cli(&[dir.path().to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(out.contains("+++ b/words.toml"), "{out}");
    assert!(out.contains("Preview only"), "{out}");
    assert_eq!(
        read(dir.path(), "words.toml"),
        OLD,
        "a preview writes nothing"
    );
}

/// One row per way 2.x reached a part of speech that 3.0 reads differently. The expected
/// long form comes from the 2.x alias table: trim, try the value, then try it lower-cased.
#[rstest]
#[case::capitalised("Noun", "noun", Pos::Noun)]
#[case::padded(" noun", "noun", Pos::Noun)]
#[case::trailing_space("verb ", "verb", Pos::Verb)]
#[case::upper_short("N", "noun", Pos::Noun)]
#[case::upper_verb("V", "verb", Pos::Verb)]
#[case::title_adj("Adj", "adjective", Pos::Adjective)]
#[case::padded_tag("ADJ ", "adjective", Pos::Adjective)]
#[case::title_adv("Adv", "adverb", Pos::Adverb)]
#[case::upper_prep("PREP", "preposition", Pos::Preposition)]
#[case::upper_conj("CONJ", "conjunction", Pos::Conjunction)]
#[case::title_pron("Pron", "pronoun", Pos::Pronoun)]
#[case::upper_art("ART", "article", Pos::Article)]
#[case::long_upper("ARTICLE", "article", Pos::Article)]
fn a_pos_that_2x_normalised_becomes_its_long_form(
    #[case] raw: &str,
    #[case] long: &str,
    #[case] pos: Pos,
) {
    let old = format!("[[entries]]\nword = \"use\"\n  pos = \"{raw}\" # keep\nreason = \"Why.\"\n");
    let dir = project(CONFIG, &old);
    let p = plan_of(dir.path()).unwrap();
    apply(dir.path(), &p).unwrap();
    assert_eq!(
        read(dir.path(), "words.toml"),
        format!(
            "schema_version = 3\n[[entries]]\nword = \"use\"\n  pos = \"{long}\" # keep\nreason = \"Why.\"\n"
        )
    );
    assert!(
        Blocklist::load(&dir.path().join("words.toml"))
            .unwrap()
            .lookup("use", pos)
            .is_some()
    );
    assert!(
        p.notes
            .iter()
            .any(|n| n == &format!("words.toml:3: pos `{raw}` rewritten to `{long}`")),
        "{:?}",
        p.notes
    );
}

#[rstest]
#[case::short("n")]
#[case::long("adjective")]
#[case::spacy("ADP")]
fn a_pos_that_3_0_already_reads_stays_as_written(#[case] raw: &str) {
    let old = format!("[[entries]]\nword = \"use\"\npos = \"{raw}\"\nreason = \"Why.\"\n");
    let dir = project(CONFIG, &old);
    apply(dir.path(), &plan_of(dir.path()).unwrap()).unwrap();
    assert_eq!(
        read(dir.path(), "words.toml"),
        format!("schema_version = 3\n{old}")
    );
}

#[rstest]
#[case::lowercase_spacy_tag("adp")]
#[case::mixed_spacy_tag("Det")]
#[case::unknown_word("gerund")]
fn an_unknown_pos_is_reported_with_its_line_and_nothing_is_written(#[case] raw: &str) {
    let old = format!(
        "[[entries]]\nword = \"a\"\npos = \"verb\"\nreason = \"x\"\n\n[[entries]]\nword = \"b\"\npos = \"{raw}\"\nreason = \"y\"\n"
    );
    let dir = project(CONFIG, &old);
    let e = plan_of(dir.path()).unwrap_err();
    let text = dropped_text(&e);
    assert!(
        text.starts_with("words.toml:8: `entries[1].pos`: unrecognized part of speech"),
        "{text}"
    );
    assert!(text.contains(&format!("`{raw}`")), "{text}");

    let (code, _, err) = run_cli(&[dir.path().to_str().unwrap(), "--write"]);
    assert_eq!(code, 2);
    assert!(
        err.starts_with("error[config-legacy-dropped]: words.toml:8: "),
        "{err}"
    );
    assert_eq!(read(dir.path(), "words.toml"), old);
    assert_eq!(read(dir.path(), "slopvac.toml"), CONFIG);
    assert!(!dir.path().join(".slopvac-migrate-backup").exists());
}

#[test]
fn an_entry_the_loader_refuses_is_reported_at_its_line() {
    let old = "[[entries]]\nword = \"a\"\npos = \"verb\"\nreason = \"x\"\n\n[[entries]]\nword = \"b\"\npos = \"verb\"\n";
    let dir = project(CONFIG, old);
    let text = dropped_text(&plan_of(dir.path()).unwrap_err());
    assert_eq!(
        text,
        "words.toml:6: `entries[1]`: 3.0 refuses the migrated blocklist: entry 2 has no reason"
    );
}

#[rstest]
#[case::parent_directory("../outside.toml")]
#[case::absolute("/etc/hosts")]
fn a_blocklist_outside_the_project_is_refused_and_never_written(#[case] path: &str) {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("project");
    write(
        &root,
        "slopvac.toml",
        &format!("[vocabulary]\npath = \"{path}\"\n"),
    );
    write(parent.path(), "outside.toml", OLD);
    let e = plan_of(&root).unwrap_err();
    assert_eq!(e.code(), "migrate-unsafe-path");
    assert!(
        e.to_string().contains(
            "slopvac.toml:2: `vocabulary.path`: the blocklist is outside the project root"
        ),
        "{e}"
    );
    let (code, _, err) = run_cli(&[root.to_str().unwrap(), "--write"]);
    assert_eq!(code, 2);
    assert!(err.starts_with("error[migrate-unsafe-path]: "), "{err}");
    assert!(!err.contains(parent.path().to_str().unwrap()), "{err}");
    assert_eq!(read(parent.path(), "outside.toml"), OLD);
    assert!(!root.join(".slopvac-migrate-backup").exists());
}

#[test]
fn a_blocklist_that_is_already_3_0_is_left_alone() {
    let current =
        "schema_version = 3\n\n[[entries]]\nword = \"use\"\npos = \"verb\"\nreason = \"Why.\"\n";
    let dir = project(CONFIG, current);
    let p = plan_of(dir.path()).unwrap();
    let files: Vec<_> = p.changes.iter().map(|c| c.path.to_str().unwrap()).collect();
    assert_eq!(files, ["slopvac.toml"]);
    apply(dir.path(), &p).unwrap();
    assert_eq!(read(dir.path(), "words.toml"), current);
}

#[test]
fn a_second_run_finds_nothing_to_do() {
    let dir = project(CONFIG, OLD);
    apply(dir.path(), &plan_of(dir.path()).unwrap()).unwrap();
    assert!(plan_of(dir.path()).unwrap().is_empty());
}

#[test]
fn a_blocklist_two_settings_share_is_rewritten_once() {
    let config = "[vocabulary]\npath = \"words.toml\"\n\n[[overrides]]\nfiles = [\"docs/**\"]\n[overrides.vocabulary]\npath = \"./docs/../words.toml\"\n";
    let dir = project(config, OLD);
    let p = plan_of(dir.path()).unwrap();
    let files: Vec<_> = p.changes.iter().map(|c| c.path.to_str().unwrap()).collect();
    assert_eq!(files, ["slopvac.toml", "words.toml"]);
}

#[test]
fn a_nested_config_resolves_its_blocklist_from_its_own_directory() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "docs/slopvac.toml", CONFIG);
    write(dir.path(), "docs/words.toml", OLD);
    let p = plan_of(dir.path()).unwrap();
    let files: Vec<_> = p.changes.iter().map(|c| c.path.to_str().unwrap()).collect();
    assert_eq!(files, ["docs/slopvac.toml", "docs/words.toml"]);
}

#[test]
fn a_missing_blocklist_is_reported_at_the_setting() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "slopvac.toml", CONFIG);
    let text = dropped_text(&plan_of(dir.path()).unwrap_err());
    assert_eq!(
        text,
        "slopvac.toml:2: `vocabulary.path`: blocklist `words.toml`: file not found"
    );
}

#[test]
fn a_json_blocklist_that_3_0_refuses_is_reported_not_rewritten() {
    let old = "{\"entries\": [{\"word\": \"use\", \"pos\": \"verb\", \"reason\": \"Why.\"}]}\n";
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "slopvac.toml",
        "[vocabulary]\npath = \"words.json\"\n",
    );
    write(dir.path(), "words.json", old);
    let text = dropped_text(&plan_of(dir.path()).unwrap_err());
    assert!(
        text.starts_with(
            "words.json:1: `schema_version`: 3.0 refuses this .json blocklist (no `schema_version`)"
        ),
        "{text}"
    );
}

#[test]
fn a_json_blocklist_that_is_already_3_0_is_left_alone() {
    let current = "{\"schema_version\": 3, \"entries\": [{\"word\": \"use\", \"pos\": \"verb\", \"reason\": \"Why.\"}]}\n";
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "slopvac.toml",
        "[vocabulary]\npath = \"words.json\"\n",
    );
    write(dir.path(), "words.json", current);
    let p = plan_of(dir.path()).unwrap();
    let files: Vec<_> = p.changes.iter().map(|c| c.path.to_str().unwrap()).collect();
    assert_eq!(files, ["slopvac.toml"]);
}
