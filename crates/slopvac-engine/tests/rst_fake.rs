//! The RST adapter against fake interpreters.
//!
//! Each fake is a shell script, so these tests run on Unix. The JSON fixtures under
//! `tests/fixtures/rst` come from the real driver on docutils 0.23 (`generate.py`), so the
//! position mapping runs on real docutils output without Python on the test machine.
#![cfg(unix)]
#[expect(
    dead_code,
    reason = "each test binary uses the part of the shared helpers it needs"
)]
mod common;

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use slopvac_engine::rst::trust::{OverrideError, SkipReason, check_override, discover};
use slopvac_engine::rst::{
    ARGS_BEFORE_DRIVER, DRIVER, INHERITED_ENV, RstAdapter, RstError, RstOptions, RstParse,
    parse_rst, restricted_path,
};
use slopvac_model::Span;
use slopvac_parse::{BlockKind, Container, TokenKind};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/rst");

/// What the probe interpreter saw.
struct Probe {
    argc: String,
    first_args: Vec<String>,
    driver: String,
    env: Vec<(String, String)>,
    cwd: String,
    cwd_entries: String,
}

struct Fakes {
    dir: PathBuf,
    probe: Probe,
}

/// Write an executable script.
#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn script(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Every script is written once, before any test starts a process. A script that is still
/// open for writing when another thread forks cannot be executed (ETXTBSY).
#[expect(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn fakes() -> &'static Fakes {
    static FAKES: OnceLock<Fakes> = OnceLock::new();
    FAKES.get_or_init(|| {
        let dir = common::private_dir("rst-fakes");
        for entry in fs::read_dir(FIXTURES).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                let name = path.file_stem().unwrap().to_str().unwrap();
                script(
                    &dir.join(format!("json-{name}")),
                    &format!("cat >/dev/null\ncat '{}'", path.display()),
                );
            }
        }
        fs::write(dir.join("bad-shebang"), "#!/no/such/interpreter\n").unwrap();
        fs::set_permissions(dir.join("bad-shebang"), fs::Permissions::from_mode(0o755)).unwrap();
        script(&dir.join("sleeper"), "exec sleep 1000");
        script(
            &dir.join("flood"),
            "cat >/dev/null\nprintf 'xxxxxxxxxxxxxxxxxxxx'",
        );
        script(
            &dir.join("failing"),
            "cat >/dev/null\necho boom >&2\nexit 7",
        );
        script(&dir.join("no-docutils"), "cat >/dev/null\nexit 3");
        script(&dir.join("old-docutils"), "cat >/dev/null\nexit 4");
        script(&dir.join("not-json"), "cat >/dev/null\nprintf '<html>'");
        script(
            &dir.join("probe"),
            "d=$(dirname \"$0\")\n\
             echo $# > \"$d/probe.argc\"\n\
             printf '%s\\n' \"$1\" \"$2\" \"$3\" \"$4\" \"$5\" > \"$d/probe.args\"\n\
             printf '%s' \"$6\" > \"$d/probe.driver\"\n\
             env > \"$d/probe.env\"\n\
             pwd -P > \"$d/probe.cwd\"\n\
             ls -A > \"$d/probe.ls\"\n\
             cat > /dev/null\n\
             printf '{\"docutils\":\"0.0\",\"nodes\":[]}'",
        );
        // Discovery layouts. Probes pass for a script that exits 0.
        script(&dir.join("trusted/python3"), "exit 0");
        script(&dir.join("trusted-python-only/python"), "exit 0");
        script(&dir.join("no-docutils-bin/python3"), "exit 1");
        script(&dir.join("project/bin/python3"), "exit 0");
        script(&dir.join("project/bin/python"), "exit 0");
        script(&dir.join("open/python3"), "exit 0");
        script(
            &dir.join("marker-bin/python3"),
            "touch \"$(dirname \"$0\")/ran\"\nexit 0",
        );
        script(
            &dir.join("marker-bin-b/python3"),
            "touch \"$(dirname \"$0\")/ran\"\nexit 0",
        );
        fs::set_permissions(dir.join("open"), fs::Permissions::from_mode(0o777)).unwrap();
        fs::create_dir_all(dir.join("links")).unwrap();
        symlink(dir.join("project/bin/python3"), dir.join("links/python3")).unwrap();
        fs::create_dir_all(dir.join("empty")).unwrap();

        let mut options = RstOptions::default();
        options.interpreter = Some(dir.join("probe"));
        parse_rst("probe input\n", &options).expect("the probe interpreter runs");
        let text = |name: &str| fs::read_to_string(dir.join(name)).unwrap();
        let probe = Probe {
            argc: text("probe.argc").trim().to_owned(),
            first_args: text("probe.args").lines().map(str::to_owned).collect(),
            driver: text("probe.driver"),
            env: text("probe.env")
                .lines()
                .filter_map(|l| l.split_once('=').map(|(a, b)| (a.to_owned(), b.to_owned())))
                .collect(),
            cwd: text("probe.cwd").trim().to_owned(),
            cwd_entries: text("probe.ls"),
        };
        Fakes { dir, probe }
    })
}

fn fake(name: &str) -> PathBuf {
    fakes().dir.join(name)
}

fn using(interpreter: &Path) -> RstOptions {
    let mut options = RstOptions::default();
    options.interpreter = Some(interpreter.to_owned());
    options
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn read_source(name: &str) -> String {
    fs::read_to_string(format!("{FIXTURES}/{name}.rst.in")).unwrap()
}

/// Parse the fixture `name` through its recorded driver output.
#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn parse_fixture(name: &str) -> (String, RstParse) {
    let source = read_source(name);
    let parsed = parse_rst(&source, &using(&fake(&format!("json-{name}")))).unwrap();
    (source, parsed)
}

fn slice(source: &str, span: Span) -> &str {
    &source[span.start()..span.end()]
}

/// The projected text and the source text of every block that has a projection.
fn prose(source: &str, parsed: &RstParse) -> Vec<(String, String)> {
    parsed
        .document
        .blocks
        .iter()
        .filter_map(|b| {
            b.projection
                .as_ref()
                .map(|p| (p.text().to_owned(), slice(source, b.span).to_owned()))
        })
        .collect()
}

fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|&(a, b)| (a.to_owned(), b.to_owned()))
        .collect()
}

#[expect(
    clippy::unwrap_used,
    reason = "test helper outside a test fn: a setup failure should fail the calling test"
)]
fn path_list(dirs: &[&Path]) -> std::ffi::OsString {
    std::env::join_paths(dirs).unwrap()
}

// --- trust: which interpreter may run ---------------------------------------------------

fn found(search: &std::ffi::OsStr, roots: &[PathBuf]) -> (Option<PathBuf>, Vec<SkipReason>) {
    let result = discover(search, roots, |_| true);
    (
        result.interpreter,
        result.skipped.into_iter().map(|s| s.reason).collect(),
    )
}

#[test]
fn python3_comes_before_python() {
    let both = path_list(&[&fake("trusted-python-only"), &fake("trusted")]);
    assert_eq!(found(&both, &[]).0, Some(fake("trusted/python3")));
}

#[test]
fn python_is_used_when_there_is_no_python3() {
    let search = path_list(&[&fake("trusted-python-only")]);
    assert_eq!(
        found(&search, &[]).0,
        Some(fake("trusted-python-only/python"))
    );
}

#[test]
fn an_interpreter_without_docutils_is_passed_over() {
    let search = path_list(&[&fake("no-docutils-bin"), &fake("trusted")]);
    let result = discover(&search, &[], |p| p != fake("no-docutils-bin/python3"));
    assert_eq!(result.interpreter, Some(fake("trusted/python3")));
    assert_eq!(result.skipped[0].reason, SkipReason::NoDocutils);
}

#[test]
fn an_interpreter_planted_in_the_project_is_never_used() {
    let project = fake("project");
    let search = path_list(&[&fake("project/bin")]);
    let (interpreter, reasons) = found(&search, &[project]);
    assert_eq!(interpreter, None);
    assert_eq!(reasons.len(), 1);
    assert!(matches!(&reasons[0], SkipReason::InsideProject(_)));
}

#[test]
fn a_project_directory_in_front_of_a_trusted_one_loses() {
    let search = path_list(&[&fake("project/bin"), &fake("trusted")]);
    let (interpreter, _) = found(&search, &[fake("project")]);
    assert_eq!(interpreter, Some(fake("trusted/python3")));
}

#[test]
fn a_symlink_that_leads_into_the_project_is_not_used() {
    let search = path_list(&[&fake("links")]);
    let (interpreter, reasons) = found(&search, &[fake("project")]);
    assert_eq!(interpreter, None);
    assert!(matches!(&reasons[0], SkipReason::InsideProject(_)));
}

#[test]
fn relative_and_empty_path_entries_are_ignored() {
    let search = std::ffi::OsString::from(":relative/bin:./bin");
    let (interpreter, reasons) = found(&search, &[]);
    assert_eq!(interpreter, None);
    assert_eq!(
        reasons,
        [
            SkipReason::EmptyEntry,
            SkipReason::RelativeEntry,
            SkipReason::RelativeEntry
        ]
    );
}

#[test]
fn a_world_writable_directory_is_not_trusted() {
    let search = path_list(&[&fake("open")]);
    let (interpreter, reasons) = found(&search, &[]);
    assert_eq!(interpreter, None);
    assert_eq!(reasons, [SkipReason::WorldWritable]);
}

#[test]
fn an_override_outside_the_project_is_accepted() {
    let path = fake("trusted/python3");
    assert_eq!(check_override(&path, &[fake("project")]), Ok(path));
}

#[test]
fn an_override_must_be_absolute() {
    assert_eq!(
        check_override(Path::new("python3"), &[]),
        Err(OverrideError::NotAbsolute)
    );
}

#[test]
fn an_override_inside_the_project_is_refused() {
    let result = check_override(&fake("project/bin/python3"), &[fake("project")]);
    assert!(
        matches!(result, Err(OverrideError::InsideProject(_))),
        "{result:?}"
    );
}

#[test]
fn an_override_that_is_not_executable_is_refused() {
    assert_eq!(
        check_override(&fake("empty"), &[]),
        Err(OverrideError::NotExecutable)
    );
}

#[test]
fn a_planted_interpreter_leaves_the_adapter_without_a_converter() {
    let mut options = RstOptions::default();
    options.search_path = Some(path_list(&[&fake("project/bin")]));
    options.project_roots = vec![fake("project")];
    let error = parse_rst("text\n", &options).unwrap_err();
    assert!(error.is_missing(), "{error}");
    assert!(error.to_string().contains("inside the project"), "{error}");
}

#[test]
fn a_missing_interpreter_has_the_2x_incomplete_text() {
    let mut options = RstOptions::default();
    options.search_path = Some(path_list(&[&fake("empty")]));
    let error = parse_rst("Title\n=====\n", &options).unwrap_err();
    assert!(error.is_missing());
    let message = error.message(&[Path::new("docs/a.rst"), Path::new("docs/b.rst")]);
    assert!(
        message.starts_with(
            "RST target(s) skipped: rst2html or rst2html.py not found on PATH \
             (docs/a.rst, docs/b.rst); install with `pip install docutils`."
        ),
        "{message}"
    );
    assert!(message.contains("docutils 0.21 or later"), "{message}");
    assert!(message.contains("SLOPVAC_RST_CONVERTER"), "{message}");
}

#[test]
fn an_interpreter_without_docutils_is_a_missing_converter() {
    let error = parse_rst("text\n", &using(&fake("no-docutils"))).unwrap_err();
    assert!(error.is_missing(), "{error}");
    let error = parse_rst("text\n", &using(&fake("old-docutils"))).unwrap_err();
    assert!(error.to_string().contains("older than 0.21"), "{error}");
}

#[test]
fn the_adapter_resolves_the_interpreter_once() {
    let adapter = RstAdapter::new(using(&fake("json-sections")));
    let first = adapter.interpreter().unwrap().to_owned();
    assert_eq!(adapter.interpreter().unwrap(), first);
    assert_eq!(first, fake("json-sections"));
}

// --- the process --------------------------------------------------------------------------

#[test]
fn the_interpreter_gets_exactly_the_fixed_arguments() {
    let probe = &fakes().probe;
    assert_eq!(probe.argc, "6");
    assert_eq!(probe.first_args, ["-E", "-B", "-X", "utf8", "-c"]);
    assert_eq!(probe.first_args, ARGS_BEFORE_DRIVER);
    assert_eq!(probe.driver, DRIVER);
}

#[test]
fn the_driver_turns_off_file_insertion_and_raw() {
    for needle in [
        "file_insertion_enabled = False",
        "raw_enabled = False",
        "_disable_config = True",
        "report_level = 5",
    ] {
        assert!(DRIVER.contains(needle), "missing {needle}");
    }
}

#[test]
fn the_interpreter_cannot_read_a_docutils_conf() {
    let value = fakes()
        .probe
        .env
        .iter()
        .find(|(n, _)| n == "DOCUTILSCONFIG")
        .map(|(_, v)| v.as_str());
    assert_eq!(value, Some("/dev/null"));
}

#[test]
fn the_interpreter_inherits_only_listed_variables() {
    let mut allowed: Vec<&str> = INHERITED_ENV.to_vec();
    // Set by the adapter, and by the shell that runs the script.
    allowed.extend(["PATH", "DOCUTILSCONFIG", "PWD", "OLDPWD", "SHLVL", "_"]);
    let unexpected: Vec<_> = fakes()
        .probe
        .env
        .iter()
        .map(|(n, _)| n.as_str())
        .filter(|n| !allowed.contains(n))
        .collect();
    assert!(
        unexpected.is_empty(),
        "unexpected variables: {unexpected:?}"
    );
}

#[test]
fn the_child_path_holds_only_the_interpreter_directory_and_system_directories() {
    let dir = fakes().dir.clone();
    let value = fakes()
        .probe
        .env
        .iter()
        .find(|(n, _)| n == "PATH")
        .map(|(_, v)| v.clone())
        .unwrap();
    assert_eq!(value, format!("{}:/usr/bin:/bin", dir.display()));
    assert_eq!(
        restricted_path(&dir.join("probe")),
        std::ffi::OsString::from(value)
    );
}

#[test]
fn the_interpreter_runs_in_an_empty_private_directory_that_is_removed_after() {
    let probe = &fakes().probe;
    assert_eq!(probe.cwd_entries, "");
    let cwd = Path::new(&probe.cwd);
    assert!(!cwd.starts_with(env!("CARGO_MANIFEST_DIR")));
    assert!(!cwd.exists(), "{} was not removed", cwd.display());
}

#[test]
fn the_document_goes_to_stdin_without_its_bom() {
    let source = format!("\u{feff}{}", read_source("sections"));
    let parsed = parse_rst(&source, &using(&fake("json-sections"))).unwrap();
    // The recorded lines are the same, and offsets count the three BOM bytes.
    assert_eq!(parsed.document.blocks[0].span.start(), 3);
    assert_eq!(
        slice(&source, parsed.document.blocks[0].span),
        "Release notes"
    );
}

#[test]
fn an_interpreter_that_blocks_is_stopped_at_the_timeout() {
    let mut options = using(&fake("sleeper"));
    options.timeout = Duration::from_millis(200);
    let error = parse_rst("text\n", &options).unwrap_err();
    assert!(matches!(error, RstError::Timeout { .. }), "{error}");
}

#[test]
fn output_over_the_cap_is_refused() {
    let mut options = using(&fake("flood"));
    options.max_output_bytes = 10;
    let error = parse_rst("text\n", &options).unwrap_err();
    assert!(
        matches!(error, RstError::OutputTooLarge { limit: 10, .. }),
        "{error}"
    );
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "the enum is non_exhaustive or defined in another crate, so a wildcard arm is required"
)]
#[test]
fn a_failing_interpreter_reports_status_and_stderr() {
    match parse_rst("text\n", &using(&fake("failing"))).unwrap_err() {
        RstError::Failed { status, stderr, .. } => {
            assert_eq!(status, "exit status 7");
            assert_eq!(stderr, "boom");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn output_that_is_not_json_is_refused() {
    let error = parse_rst("text\n", &using(&fake("not-json"))).unwrap_err();
    assert!(matches!(error, RstError::InvalidOutput { .. }), "{error}");
}

#[test]
fn an_interpreter_that_cannot_start_is_an_error() {
    let error = parse_rst("text\n", &using(&fake("bad-shebang"))).unwrap_err();
    assert!(matches!(error, RstError::Spawn { .. }), "{error}");
}

#[test]
fn an_override_that_does_not_exist_is_a_missing_converter_that_says_so() {
    let error = parse_rst("text\n", &using(&fake("no-such-interpreter"))).unwrap_err();
    assert!(error.is_missing(), "{error}");
    assert!(
        error
            .to_string()
            .contains("SLOPVAC_RST_CONVERTER is unusable"),
        "{error}"
    );
}

#[test]
fn the_result_names_the_interpreter_and_docutils() {
    let (_, parsed) = parse_fixture("sections");
    assert_eq!(parsed.interpreter, fake("json-sections"));
    assert_eq!(parsed.docutils, "0.21.2");
}

// --- mapping on docutils output ----------------------------------------------------------

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "the enum is non_exhaustive or defined in another crate, so a wildcard arm is required"
)]
#[test]
fn sections_map_to_the_source_text() {
    let (source, parsed) = parse_fixture("sections");
    let paragraph = source.split("\n\n").nth(1).unwrap().trim_end().to_owned();
    assert_eq!(
        prose(&source, &parsed),
        vec![
            ("Release notes".to_owned(), "Release notes".to_owned()),
            (
                "Welcome to the new release. It is very fast, and \0 is its default. \
                 Escaped *stars* stay literal."
                    .to_owned(),
                paragraph
            ),
            ("Install".to_owned(), "Install".to_owned()),
            (
                "Run the installer, then restart the service.".to_owned(),
                "Run the installer, then restart the service.".to_owned()
            ),
        ]
    );
    let levels: Vec<_> = parsed
        .document
        .blocks
        .iter()
        .filter_map(|b| match b.kind {
            BlockKind::Heading { level } => Some(level),
            _ => None,
        })
        .collect();
    assert_eq!(levels, [1, 2]);
    assert_eq!(parsed.notices, []);
}

#[test]
fn emphasis_markers_stay_outside_the_word_span() {
    let (source, parsed) = parse_fixture("sections");
    let projection = parsed.document.blocks[1].projection.as_ref().unwrap();
    for (word, expected) in [("new", "new"), ("very", "very")] {
        let at = projection.text().find(word).unwrap();
        let span = projection.span_for_bytes(at, at + word.len());
        assert_eq!(slice(&source, span), expected, "{word}");
    }
}

#[test]
fn a_backslash_escape_maps_to_the_backslash_and_the_character() {
    let (source, parsed) = parse_fixture("sections");
    let projection = parsed.document.blocks[1].projection.as_ref().unwrap();
    let at = projection.text().find("*stars*").unwrap();
    let span = projection.span_for_bytes(at, at + "*stars*".len());
    assert_eq!(slice(&source, span), "\\*stars\\*");
    let escapes: Vec<_> = projection
        .tokens()
        .iter()
        .filter(|t| t.kind == TokenKind::Backslash)
        .map(|t| slice(&source, t.src))
        .collect();
    assert_eq!(escapes, ["\\*", "\\*"]);
}

#[test]
fn an_inline_literal_is_a_masked_code_span() {
    let (source, parsed) = parse_fixture("sections");
    let spans: Vec<_> = parsed
        .document
        .code_spans
        .iter()
        .map(|&s| slice(&source, s))
        .collect();
    assert_eq!(spans, ["``fast_path``"]);
    let projection = parsed.document.blocks[1].projection.as_ref().unwrap();
    let at = projection.text().find('\0').unwrap();
    assert_eq!(
        slice(&source, projection.span_for_bytes(at, at + 1)),
        "``fast_path``"
    );
}

#[test]
fn lists_map_each_item_and_know_their_containers() {
    let (source, parsed) = parse_fixture("lists");
    assert_eq!(
        prose(&source, &parsed),
        pairs(&[
            ("Checklist", "Checklist"),
            (
                "First item has emphasis and continues here.",
                "First item has *emphasis*\n  and continues here."
            ),
            ("Second item.", "Second item."),
            ("Numbered one.", "Numbered one."),
            ("Numbered two.", "Numbered two."),
            ("term", "term"),
            ("The definition of the term.", "The definition of the term."),
        ])
    );
    let blocks = &parsed.document.blocks;
    assert_eq!(
        blocks[1].containers,
        [Container::ListItem {
            ordered: false,
            depth: 1
        }]
    );
    assert_eq!(
        blocks[3].containers,
        [Container::ListItem {
            ordered: true,
            depth: 1
        }]
    );
    assert_eq!(blocks[0].containers, []);
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "the enum is non_exhaustive or defined in another crate, so a wildcard arm is required"
)]
#[test]
fn literal_blocks_and_directives_map_to_their_lines() {
    let (source, parsed) = parse_fixture("blocks");
    assert_eq!(
        prose(&source, &parsed),
        pairs(&[
            ("Examples", "Examples"),
            ("Call it like this:", "Call it like this:"),
            ("Then read the output.", "Then read the output."),
            (
                "Keep the directive body in mind.",
                "Keep the directive body in mind."
            ),
        ])
    );
    let code: Vec<_> = parsed
        .document
        .blocks
        .iter()
        .filter_map(|b| match &b.kind {
            BlockKind::CodeBlock { body, lang, .. } => Some((slice(&source, *body), lang.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        code,
        [
            ("    run --fast\n    run --slow", None),
            ("   print(\"hello\")", Some("python".to_owned())),
        ]
    );
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "the enum is non_exhaustive or defined in another crate, so a wildcard arm is required"
)]
#[test]
fn table_cells_map_to_their_text_with_row_and_column() {
    let (source, parsed) = parse_fixture("table");
    let cells: Vec<_> = parsed
        .document
        .blocks
        .iter()
        .filter_map(|b| match b.kind {
            BlockKind::TableCell { row, column } => Some((row, column, slice(&source, b.span))),
            _ => None,
        })
        .collect();
    assert_eq!(
        cells,
        [
            (0, 0, "Flag"),
            (0, 1, "Meaning"),
            (1, 0, "-q"),
            (1, 1, "Quiet mode"),
            (2, 0, "-v"),
            (2, 1, "Verbose mode"),
        ]
    );
}

#[test]
fn a_contents_directive_adds_no_prose_and_the_titles_map_where_they_are_written() {
    let (source, parsed) = parse_fixture("contents");
    assert_eq!(
        prose(&source, &parsed),
        pairs(&[
            ("Guide", "Guide"),
            ("Setup", "Setup"),
            ("Setup comes first.", "Setup comes first."),
            ("Usage", "Usage"),
            ("Usage comes second.", "Usage comes second."),
        ])
    );
    assert_eq!(
        parsed.document.blocks[1].span.start(),
        source.find("Setup\n-----").unwrap()
    );
}

#[test]
fn include_and_raw_directives_leave_no_content() {
    let (source, parsed) = parse_fixture("inserted");
    let texts: Vec<_> = prose(&source, &parsed).into_iter().map(|p| p.0).collect();
    assert_eq!(texts, ["Safe", "Before the include.", "After the include."]);
}

// --- regressions from the first review ----------------------------------------------------

#[test]
fn a_comment_that_repeats_a_paragraph_does_not_take_its_position() {
    let (source, parsed) = parse_fixture("comment-repeat");
    let phrase = "The deployment guide covers installation.";
    let starts: Vec<_> = parsed
        .document
        .blocks
        .iter()
        .skip(1)
        .map(|b| (b.span.start(), slice(&source, b.span)))
        .collect();
    assert_eq!(
        starts,
        [
            (source.find(phrase).unwrap(), phrase),
            (source.rfind(phrase).unwrap(), phrase),
        ]
    );
    assert_ne!(starts[0].0, starts[1].0);
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "the enum is non_exhaustive or defined in another crate, so a wildcard arm is required"
)]
#[test]
fn an_admonition_title_does_not_take_the_body_position() {
    let (source, parsed) = parse_fixture("admonition");
    let kinds: Vec<_> = parsed
        .document
        .blocks
        .iter()
        .map(|b| match &b.kind {
            BlockKind::Admonition(a) => format!("admonition:{}:{:?}", a.kind, a.title),
            _ => "other".to_owned(),
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "other",
            "admonition:admonition:Some(\"Read this first\")",
            "other",
            "admonition:note:None",
            "other",
        ]
    );
    let body = &parsed.document.blocks[2];
    assert_eq!(
        body.span.start(),
        source.find("Read this first, then").unwrap()
    );
    assert_eq!(
        slice(&source, body.span),
        "Read this first, then continue with the guide."
    );
    let inline = &parsed.document.blocks[4];
    assert_eq!(
        slice(&source, inline.span),
        "The note body says read this first too."
    );
}

#[test]
fn link_punctuation_roles_and_references_keep_their_prose() {
    let (source, parsed) = parse_fixture("links");
    let blocks = prose(&source, &parsed);
    assert_eq!(
        blocks[1].0,
        "See the guide, then another one, and read more. Footnote ref and stress, plus inside \
         prose."
    );
    let projection = parsed.document.blocks[1].projection.as_ref().unwrap();
    for (word, expected) in [
        ("the guide", "the guide"),
        ("another one", "another one"),
        ("stress", "stress"),
        ("read more.", "read more."),
    ] {
        let at = projection.text().find(word).unwrap();
        let span = projection.span_for_bytes(at, at + word.len());
        assert_eq!(slice(&source, span), expected, "{word}");
    }
}

#[test]
fn a_paragraph_of_only_literals_keeps_its_code_spans() {
    let (source, parsed) = parse_fixture("links");
    let literal_only: Vec<_> = prose(&source, &parsed)
        .into_iter()
        .filter(|p| {
            p.0.chars()
                .all(|c| c == '\0' || c == ' ' || c == 'a' || c == 'n' || c == 'd')
        })
        .collect();
    assert_eq!(
        literal_only,
        pairs(&[
            ("\0", "``solo_code``"),
            ("\0 and \0", "``one_code`` and ``two_code``"),
        ])
    );
    let codes: Vec<_> = parsed
        .document
        .code_spans
        .iter()
        .map(|&s| slice(&source, s))
        .collect();
    assert_eq!(codes, ["``solo_code``", "``one_code``", "``two_code``"]);
}

#[test]
fn prose_after_a_comment_longer_than_16_kib_is_still_found() {
    let (source, parsed) = parse_fixture("long-comment");
    assert!(source.len() > 20_000);
    assert_eq!(
        prose(&source, &parsed),
        pairs(&[(
            "Real prose after the long comment.",
            "Real prose after the long comment."
        )])
    );
    assert_eq!(
        parsed.document.blocks[0].span.start(),
        source.rfind("Real prose").unwrap()
    );
}

// --- round 2: boundaries, tables, generated text -------------------------------------------

#[test]
fn without_project_boundaries_nothing_on_the_path_is_run() {
    // A trusted-looking directory, but the adapter does not know where the project is.
    let marker = fake("marker-bin/ran");
    #[expect(
        clippy::let_underscore_must_use,
        reason = "removing a marker file that may not exist; nothing below depends on the result"
    )]
    let _ = fs::remove_file(&marker);
    let mut options = RstOptions::default();
    options.search_path = Some(path_list(&[&fake("marker-bin")]));
    let error = parse_rst("text\n", &options).unwrap_err();
    assert!(error.is_missing(), "{error}");
    assert!(error.to_string().contains("project"), "{error}");
    assert!(
        !marker.exists(),
        "the interpreter ran before the boundaries were known"
    );
}

#[test]
fn with_boundaries_the_same_search_path_runs_the_probe() {
    let marker = fake("marker-bin-b/ran");
    let mut options = RstOptions::default();
    options.search_path = Some(path_list(&[&fake("marker-bin-b")]));
    options.project_roots = vec![fake("empty")];
    #[expect(
        clippy::let_underscore_must_use,
        reason = "removing a marker file that may not exist; nothing below depends on the result"
    )]
    let _ = parse_rst("text\n", &options);
    assert!(marker.exists(), "the probe did not run");
    #[expect(
        clippy::let_underscore_must_use,
        reason = "the call is made for its side effect, which the next assertion checks"
    )]
    let _ = fs::remove_file(&marker);
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "the enum is non_exhaustive or defined in another crate, so a wildcard arm is required"
)]
#[test]
fn table_cells_with_the_same_text_keep_their_own_columns() {
    let (source, parsed) = parse_fixture("table-repeat");
    let cells: Vec<_> = parsed
        .document
        .blocks
        .iter()
        .filter_map(|b| match b.kind {
            BlockKind::TableCell { row, column } => {
                Some((row, column, b.span.start(), slice(&source, b.span)))
            }
            _ => None,
        })
        .collect();
    let line_start = |needle: &str| source.find(needle).unwrap();
    let first = line_start("| Same words | Same words |");
    let other = line_start("| Other      | Same words |");
    let grid: Vec<_> = cells.iter().take(4).copied().collect();
    assert_eq!(
        grid,
        [
            (0, 0, first + 2, "Same words"),
            (0, 1, first + 15, "Same words"),
            (1, 0, other + 2, "Other"),
            (1, 1, other + 15, "Same words |\n|            | continued"),
        ]
    );
    let simple: Vec<_> = cells.iter().skip(4).map(|c| (c.0, c.1, c.3)).collect();
    assert_eq!(
        simple,
        [
            (0, 0, "Left"),
            (0, 1, "Left"),
            (1, 0, "Alpha"),
            (1, 1, "Beta")
        ]
    );
    let left = line_start("Left       Left");
    assert_eq!([cells[4].2, cells[5].2], [left, left + 11]);
}

#[test]
fn a_table_is_a_block_that_covers_its_cells_and_borders() {
    let (source, parsed) = parse_fixture("table-repeat");
    let tables: Vec<_> = parsed
        .document
        .blocks
        .iter()
        .filter(|b| b.kind == BlockKind::Table)
        .map(|b| slice(&source, b.span))
        .collect();
    assert_eq!(tables.len(), 2);
    assert!(
        tables[0].starts_with("+----") && tables[0].ends_with("----+"),
        "{}",
        tables[0]
    );
    assert!(
        tables[1].starts_with("=====") && tables[1].ends_with("====="),
        "{}",
        tables[1]
    );
    assert_eq!(parsed.document.blocks[1].kind, BlockKind::Table);
}

#[test]
fn a_contents_directive_leaves_no_notice() {
    let (_, parsed) = parse_fixture("contents");
    assert_eq!(parsed.notices, []);
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "the enum is non_exhaustive or defined in another crate, so a wildcard arm is required"
)]
#[test]
fn table_cells_report_their_logical_columns_after_column_and_row_spans() {
    let (source, parsed) = parse_fixture("table-span");
    let cells: Vec<_> = parsed
        .document
        .blocks
        .iter()
        .filter_map(|b| match b.kind {
            BlockKind::TableCell { row, column } => Some((row, column, slice(&source, b.span))),
            _ => None,
        })
        .collect();
    assert_eq!(
        cells,
        [
            (0, 0, "A"),
            (0, 1, "B"),
            (0, 2, "C"),
            (1, 0, "Wide span"),
            (1, 2, "D"),
            (2, 0, "E"),
            (2, 1, "F"),
            (2, 2, "G"),
            (3, 1, "H"),
        ]
    );
    assert_eq!(parsed.notices, []);
}

#[test]
fn prose_nested_in_a_cell_keeps_the_cells_columns() {
    let (source, parsed) = parse_fixture("table-nested");
    let starts: Vec<_> = parsed
        .document
        .blocks
        .iter()
        .filter(|b| b.projection.is_some())
        .map(|b| b.span.start())
        .collect();
    let first = source.find("- same").unwrap();
    let second = first + source[first + 1..].find("- same").unwrap() + 1;
    assert_eq!(starts, [first + 2, second + 2]);
}

#[test]
fn the_decoded_driver_output_is_counted_as_scanned() {
    let (source, parsed) = parse_fixture("sections");
    let json = fs::metadata(format!("{FIXTURES}/sections.json"))
        .unwrap()
        .len();
    assert!(
        parsed.scanned >= usize::try_from(json).unwrap() + source.len(),
        "scanned {} for {json} bytes of output and {} of source",
        parsed.scanned,
        source.len()
    );
}
