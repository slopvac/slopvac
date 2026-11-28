//! `slopvac lint` reads plain text and HTML: a `.txt` file and `--language text` as Markdown,
//! as 2.x does, and a `.html` file and `--language html` as a standalone HTML page whose
//! visible text is the prose.
use std::path::Path;
use std::time::Duration;

use rstest::rstest;
use serde_json::Value;
use snapbox::cmd::{Command, cargo_bin};

/// The finding every case below expects: "leverage" is an error-level slop word.
const SLOP: &str = "error core:prose-inflation.slop-lexicon";

fn slopvac(dir: &Path) -> Command {
    let mut cmd = Command::new(cargo_bin!("slopvac"));
    for var in [
        "SLOPVAC_OFFLINE",
        "SLOPVAC_LOG",
        "FORCE_COLOR",
        "CLICOLOR_FORCE",
    ] {
        cmd = cmd.env_remove(var);
    }
    cmd.env("NO_COLOR", "1")
        .env("SLOPVAC_NO_UPDATE_CHECK", "1")
        .current_dir(dir)
        .timeout(Duration::from_secs(60))
}

/// Run `cmd`, expect `code`, and return (stdout, stderr).
fn run(cmd: Command, code: i32) -> (String, String) {
    let assert = cmd.assert().code(code);
    let out = assert.get_output();
    // The checks below read text, so invalid bytes must fail here and not turn into U+FFFD.
    assert!(
        std::str::from_utf8(&out.stdout).is_ok(),
        "stdout is not UTF-8: {:?}",
        out.stdout
    );
    assert!(
        std::str::from_utf8(&out.stderr).is_ok(),
        "stderr is not UTF-8: {:?}",
        out.stderr
    );
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[rstest]
// Read as Markdown: the heading is line 1, and `We ` puts the word at column 4 of line 3.
#[case::text("notes.txt", "# Notes\n\nWe leverage the data.\n", "3:4")]
// `<p>We ` is 6 bytes, so the word starts at column 7 of line 2.
#[case::html(
    "page.html",
    "<html><body>\n<p>We leverage the data.</p>\n</body></html>\n",
    "2:7"
)]
fn a_file_is_linted_with_its_finding_in_place(
    #[case] name: &str,
    #[case] text: &str,
    #[case] place: &str,
) {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join(name), text).expect("write");
    // The error fails the gate of the default profile, which allows none.
    let (out, err) = run(slopvac(dir.path()).args(["lint", name]), 1);
    assert!(out.contains(&format!("{place} {SLOP}")), "{out}");
    assert!(!err.contains("input-unsupported"), "{err}");
}

#[rstest]
#[case::text("text", "We leverage the data.\n", "1:4")]
#[case::html("html", "<p>We leverage the data.</p>\n", "1:7")]
fn stdin_is_linted_with_its_finding_in_place(
    #[case] language: &str,
    #[case] text: &str,
    #[case] place: &str,
) {
    let dir = tempfile::tempdir().expect("temp dir");
    let cmd = slopvac(dir.path())
        .args(["lint", "--stdin", "--language", language])
        .stdin(text);
    let (out, err) = run(cmd, 1);
    assert!(out.contains(&format!("{place} {SLOP}")), "{out}");
    assert!(!err.contains("input-unsupported"), "{err}");
}

#[rstest]
// Read as HTML, the word inside the code span or the fence would be prose and a slop error.
#[case::inline_code_in_a_file(&["notes.txt"], "We use `leverage` here.\n")]
#[case::fence_in_a_file(&["notes.txt"], "```\nWe leverage it.\n```\n")]
#[case::inline_code_on_stdin(&["--stdin", "--language", "text"], "We use `leverage` here.\n")]
#[case::fence_on_stdin(&["--stdin", "--language", "text"], "```\nWe leverage it.\n```\n")]
fn plain_text_code_is_not_prose_as_in_markdown(#[case] target: &[&str], #[case] text: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("notes.txt"), text).expect("write");
    let mut cmd = slopvac(dir.path()).arg("lint").args(target);
    if target.contains(&"--stdin") {
        cmd = cmd.stdin(text);
    }
    let (out, _) = run(cmd, 0);
    assert!(!out.contains("slop-lexicon"), "{out}");
}

/// A project whose `[[genres]]` block gives every file under `notes/` the genre `internal`.
const ASSIGNED: &str =
    "schema_version = 3\n\n[[genres]]\nfiles = [\"notes/**\"]\ngenre = \"internal\"\n";

#[rstest]
// Read as front matter, the first block would set the genre to `reference`, and the second,
// which lacks `v`, would make the run incomplete.
#[case::valid_block("---\nslopvac:\n  v: 3\n  genre: reference\n---\n")]
#[case::invalid_block("---\nslopvac:\n  genre: reference\n---\n")]
fn a_front_matter_block_in_an_html_file_is_prose(#[case] block: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("slopvac.toml"), ASSIGNED).expect("write config");
    std::fs::create_dir(dir.path().join("notes")).expect("mkdir");
    std::fs::write(
        dir.path().join("notes/n.html"),
        format!("{block}<p>We leverage the data.</p>\n"),
    )
    .expect("write");
    let (out, err) = run(
        slopvac(dir.path()).args(["lint", "--format", "json", "notes/n.html"]),
        1,
    );
    let report: Value = serde_json::from_str(&out).expect("a JSON report");
    let doc = &report["documents"][0];
    assert_eq!(doc["genre"]["id"], "internal", "{doc}");
    assert!(!err.contains("config-"), "{err}");
    assert!(
        doc["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .any(|f| f["rule_id"] == "core:prose-inflation.slop-lexicon"),
        "{doc}"
    );
}

/// A page whose `leverage` gets a fix in Markdown: `use`, from the core word list. `<p>We `
/// is 6 bytes, so the word starts at column 7 of line 2.
const FIXABLE: &str = "<html><body>\n<p>We leverage the data.</p>\n</body></html>\n";

/// A page whose findings carry no fix in any file type.
const UNFIXABLE: &str = "<p>We delve into it.</p>\n";

const NOTICE: &str = "fix-html-unavailable";

/// The root notices of a JSON report with the code `fix-html-unavailable`, as
/// `(level, path)`.
fn fix_notices(json: &str) -> Vec<(String, String)> {
    let report: Value = serde_json::from_str(json).expect("a JSON report");
    report["notices"]
        .as_array()
        .expect("notices")
        .iter()
        .filter(|n| n["code"] == NOTICE)
        .map(|n| {
            (
                n["level"].as_str().unwrap_or_default().to_owned(),
                n["path"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

#[test]
fn a_fix_run_leaves_an_html_file_unchanged_and_still_reports_its_findings() {
    let dir = tempfile::tempdir().expect("temp dir");
    let page = dir.path().join("page.html");
    std::fs::write(&page, FIXABLE).expect("write");
    // The slop error stays, so the gate fails.
    let (out, err) = run(slopvac(dir.path()).args(["lint", "--fix", "page.html"]), 1);
    assert!(out.contains(&format!("2:7 {SLOP}")), "{out}");
    assert!(!err.contains("input-unsupported"), "{err}");
    assert_eq!(std::fs::read(&page).expect("read"), FIXABLE.as_bytes());
}

#[rstest]
#[case::a_fix_run_with_a_fixable_finding(&["--fix"], FIXABLE, 1)]
#[case::a_lint_run(&[], FIXABLE, 0)]
#[case::a_fix_run_with_no_fixable_finding(&["--fix"], UNFIXABLE, 0)]
fn a_fix_run_reports_one_notice_only_when_a_fix_was_dropped(
    #[case] fix: &[&str],
    #[case] html: &str,
    #[case] notices: usize,
) {
    let dir = tempfile::tempdir().expect("temp dir");
    let page = dir.path().join("page.html");
    std::fs::write(&page, html).expect("write");
    let cmd = slopvac(dir.path())
        .args(["lint", "--format", "json"])
        .args(fix)
        .arg("page.html");
    let (out, _) = run(cmd, 1);
    let want = vec![("info".to_owned(), "page.html".to_owned()); notices];
    assert_eq!(fix_notices(&out), want, "{out}");
    assert_eq!(std::fs::read(&page).expect("read"), html.as_bytes());
}

#[rstest]
#[case::json("json")]
#[case::sarif("sarif")]
#[case::markdown("markdown")]
#[case::html("html")]
fn each_format_that_shows_root_notices_carries_the_notice(#[case] format: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("page.html"), FIXABLE).expect("write");
    let (out, _) = run(
        slopvac(dir.path()).args(["lint", "--fix", "--format", format, "page.html"]),
        1,
    );
    assert_eq!(out.matches(NOTICE).count(), 1, "{out}");
    let message = "page.html: fixes are not applied in HTML files";
    assert!(out.contains(message), "{out}");
    if format == "sarif" {
        let sarif: Value = serde_json::from_str(&out).expect("SARIF");
        let notes = &sarif["runs"][0]["invocations"][0]["toolExecutionNotifications"];
        let note = notes
            .as_array()
            .expect("notifications")
            .iter()
            .find(|n| n["descriptor"]["id"] == NOTICE)
            .expect("the notice");
        assert_eq!(note["level"], "note", "{note}");
    }
}
