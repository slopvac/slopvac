//! The RST adapter against fake converters.
//!
//! Each fake is a shell script, so these tests run on Unix. The HTML fixtures under
//! `tests/fixtures/rst` come from docutils (`generate.py`), so the position mapping runs on
//! real converter output without docutils on the test machine.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use slopvac_engine::rst::{
    CONVERTER_ARGS, CONVERTER_NAMES, RstError, RstOptions, RstParse, find_converter, parse_rst,
};
use slopvac_model::Span;
use slopvac_parse::{BlockKind, TokenKind};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/rst");

/// What the probe converter saw.
struct Probe {
    argv: Vec<String>,
    env_names: Vec<String>,
    docutilsconfig: String,
    cwd: String,
}

struct Fakes {
    dir: PathBuf,
    probe: Probe,
}

/// Write an executable script.
fn script(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Every script is written once, before any test starts a process. A script that is still
/// open for writing when another thread forks cannot be executed (ETXTBSY).
fn fakes() -> &'static Fakes {
    static FAKES: OnceLock<Fakes> = OnceLock::new();
    FAKES.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("rst-fakes");
        let _ = fs::remove_dir_all(&dir);
        for folder in [FIXTURES.to_owned(), format!("{FIXTURES}/handwritten")] {
            for entry in fs::read_dir(folder).unwrap().flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "html") {
                    let name = path.file_stem().unwrap().to_str().unwrap();
                    script(
                        &dir.join(format!("html-{name}")),
                        &format!("cat >/dev/null\ncat '{}'", path.display()),
                    );
                }
            }
        }
        script(&dir.join("sleeper"), "exec sleep 1000");
        script(&dir.join("flood"), "cat >/dev/null\nprintf 'xxxxxxxxxxxxxxxxxxxx'");
        script(&dir.join("failing"), "cat >/dev/null\necho boom >&2\nexit 3");
        script(&dir.join("binary"), "cat >/dev/null\nprintf '\\377\\376'");
        script(
            &dir.join("probe"),
            "d=$(dirname \"$0\")\nprintf '%s\\n' \"$@\" > \"$d/probe.argv\"\nenv > \"$d/probe.env\"\npwd -P > \"$d/probe.cwd\"\ncat > \"$d/probe.stdin\"\nprintf '<p>ok</p>'",
        );
        for (bin, names) in [
            ("both", &["rst2html", "rst2html.py"][..]),
            ("py-only", &["rst2html.py"][..]),
        ] {
            for name in names {
                script(&dir.join(bin).join(name), "exit 0");
            }
        }
        fs::create_dir_all(dir.join("empty")).unwrap();
        fs::create_dir_all(dir.join("not-executable")).unwrap();
        fs::write(dir.join("not-executable/rst2html"), "#!/bin/sh\n").unwrap();
        fs::set_permissions(
            dir.join("not-executable/rst2html"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();

        let source = "probe input\n";
        let options = options_for(&dir.join("probe"));
        parse_rst(source, &options).expect("the probe converter runs");
        let read = |name: &str| fs::read(dir.join(name)).unwrap();
        let text = |name: &str| String::from_utf8(read(name)).unwrap();
        let env = text("probe.env");
        let probe = Probe {
            argv: text("probe.argv").lines().map(str::to_owned).collect(),
            docutilsconfig: env
                .lines()
                .find_map(|l| l.strip_prefix("DOCUTILSCONFIG="))
                .unwrap_or_default()
                .to_owned(),
            env_names: env
                .lines()
                .filter_map(|l| l.split_once('=').map(|(n, _)| n.to_owned()))
                .collect(),
            cwd: text("probe.cwd").trim().to_owned(),
        };
        Fakes { dir, probe }
    })
}

fn options_for(converter: &Path) -> RstOptions {
    let mut options = RstOptions::default();
    options.converter = Some(converter.to_owned());
    options
}

fn fake(name: &str) -> PathBuf {
    fakes().dir.join(name)
}

fn read_source(path: &str) -> String {
    fs::read_to_string(format!("{FIXTURES}/{path}.rst")).unwrap()
}

/// Parse the fixture `path` (such as `sections` or `handwritten/crlf`).
fn parse_fixture(path: &str) -> (String, RstParse) {
    let source = read_source(path);
    let name = path.rsplit('/').next().unwrap();
    let parsed = parse_rst(&source, &options_for(&fake(&format!("html-{name}")))).unwrap();
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

// --- discovery ---------------------------------------------------------------------

#[test]
fn converter_names_follow_2x_order() {
    assert_eq!(CONVERTER_NAMES, ["rst2html", "rst2html.py"]);
}

#[test]
fn rst2html_wins_over_rst2html_py() {
    let dir = fake("both");
    assert_eq!(
        find_converter(Some(dir.as_os_str())),
        Some(dir.join("rst2html"))
    );
}

#[test]
fn rst2html_py_is_found_when_alone() {
    let dir = fake("py-only");
    assert_eq!(
        find_converter(Some(dir.as_os_str())),
        Some(dir.join("rst2html.py"))
    );
}

#[test]
fn search_path_entries_are_tried_in_order() {
    let joined = std::env::join_paths([fake("empty"), fake("py-only")]).unwrap();
    assert_eq!(
        find_converter(Some(&joined)),
        Some(fake("py-only").join("rst2html.py"))
    );
}

#[test]
fn a_file_without_the_executable_bit_is_not_a_converter() {
    let dir = fake("not-executable");
    assert_eq!(find_converter(Some(dir.as_os_str())), None);
}

#[test]
fn a_missing_converter_is_an_incomplete_check_with_the_2x_text() {
    let mut options = RstOptions::default();
    options.search_path = Some(fake("empty").into_os_string());
    let error = parse_rst("Title\n=====\n", &options).unwrap_err();
    assert!(matches!(error, RstError::ConverterMissing));
    assert_eq!(
        error.message(&[Path::new("docs/a.rst"), Path::new("docs/b.rst")]),
        "RST target(s) skipped: rst2html or rst2html.py not found on PATH \
         (docs/a.rst, docs/b.rst); install with `pip install docutils`."
    );
}

// --- the converter process -----------------------------------------------------------

#[test]
fn the_converter_gets_exactly_the_fixed_arguments() {
    assert_eq!(
        fakes().probe.argv,
        [
            "--no-file-insertion",
            "--no-raw",
            "--quiet",
            "--report=5",
            "--halt=5",
            "--exit-status=5",
            "--link-stylesheet",
            "--syntax-highlight=none",
            "--input-encoding=utf-8",
            "--output-encoding=utf-8",
        ]
    );
    assert_eq!(CONVERTER_ARGS.to_vec(), fakes().probe.argv);
}

#[test]
fn the_converter_cannot_read_a_docutils_conf() {
    assert_eq!(fakes().probe.docutilsconfig, "/dev/null");
}

#[test]
fn the_converter_inherits_only_listed_variables() {
    let allowed = [
        "PATH",
        "HOME",
        "USERPROFILE",
        "SYSTEMROOT",
        "SYSTEMDRIVE",
        "PATHEXT",
        "TMPDIR",
        "TEMP",
        "TMP",
        "LANG",
        "LC_ALL",
        "VIRTUAL_ENV",
        "DOCUTILSCONFIG",
        "PYTHONIOENCODING",
        // Set by the shell that runs the script.
        "PWD",
        "OLDPWD",
        "SHLVL",
        "_",
    ];
    let unexpected: Vec<_> = fakes()
        .probe
        .env_names
        .iter()
        .filter(|n| !allowed.contains(&n.as_str()))
        .collect();
    assert!(
        unexpected.is_empty(),
        "unexpected variables: {unexpected:?}"
    );
}

#[test]
fn the_converter_runs_outside_the_repository() {
    let temp = fs::canonicalize(std::env::temp_dir()).unwrap();
    assert_eq!(Path::new(&fakes().probe.cwd), temp);
}

#[test]
fn the_document_goes_to_stdin_without_its_bom() {
    let dir = fakes().dir.clone();
    let options = options_for(&dir.join("probe"));
    parse_rst("\u{feff}probe input\n", &options).unwrap();
    assert_eq!(fs::read(dir.join("probe.stdin")).unwrap(), b"probe input\n");
}

#[test]
fn a_converter_that_blocks_is_stopped_at_the_timeout() {
    let mut options = options_for(&fake("sleeper"));
    options.timeout = Duration::from_millis(200);
    let error = parse_rst("text\n", &options).unwrap_err();
    assert!(matches!(error, RstError::Timeout { .. }), "{error}");
}

#[test]
fn output_over_the_cap_is_refused() {
    let mut options = options_for(&fake("flood"));
    options.max_output_bytes = 10;
    let error = parse_rst("text\n", &options).unwrap_err();
    assert!(
        matches!(error, RstError::OutputTooLarge { limit: 10, .. }),
        "{error}"
    );
}

#[test]
fn a_failing_converter_reports_status_and_stderr() {
    let error = parse_rst("text\n", &options_for(&fake("failing"))).unwrap_err();
    match error {
        RstError::Failed { status, stderr, .. } => {
            assert_eq!(status, "exit status 3");
            assert_eq!(stderr, "boom");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn output_that_is_not_utf8_is_refused() {
    let error = parse_rst("text\n", &options_for(&fake("binary"))).unwrap_err();
    assert!(matches!(error, RstError::InvalidOutput { .. }), "{error}");
}

#[test]
fn a_converter_that_cannot_start_is_an_error() {
    let error = parse_rst("text\n", &options_for(&fake("no-such-converter"))).unwrap_err();
    assert!(matches!(error, RstError::Spawn { .. }), "{error}");
}

#[test]
fn the_result_names_the_converter_that_ran() {
    let (_, parsed) = parse_fixture("sections");
    assert_eq!(parsed.converter, fake("html-sections"));
}

// --- position mapping on docutils output -----------------------------------------------

#[test]
fn sections_map_to_the_source_text() {
    let (source, parsed) = parse_fixture("sections");
    let paragraph = source.split("\n\n").nth(1).unwrap().to_owned();
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
}

#[test]
fn emphasis_markers_stay_outside_the_word_span() {
    let (source, parsed) = parse_fixture("sections");
    let projection = parsed.document.blocks[1].projection.as_ref().unwrap();
    for (word, expected) in [
        ("new", "new"),
        ("very", "very"),
        ("fast_path", "``fast_path``"),
    ] {
        let at = if word == "fast_path" {
            projection.text().find('\0').unwrap()
        } else {
            projection.text().find(word).unwrap()
        };
        let len = if word == "fast_path" { 1 } else { word.len() };
        let span = projection.span_for_bytes(at, at + len);
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
    let kinds: Vec<_> = projection
        .tokens()
        .iter()
        .filter(|t| t.kind == TokenKind::Backslash)
        .map(|t| slice(&source, t.src))
        .collect();
    assert_eq!(kinds, ["\\*", "\\*"]);
}

#[test]
fn an_inline_literal_is_a_code_span() {
    let (source, parsed) = parse_fixture("sections");
    let spans: Vec<_> = parsed
        .document
        .code_spans
        .iter()
        .map(|&s| slice(&source, s))
        .collect();
    assert_eq!(spans, ["``fast_path``"]);
}

#[test]
fn lists_map_each_item() {
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
}

#[test]
fn literal_blocks_and_directives_map_to_their_body() {
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
        .filter_map(|b| match b.kind {
            BlockKind::CodeBlock { body, .. } => Some(slice(&source, body)),
            _ => None,
        })
        .collect();
    assert_eq!(code, ["run --fast\n    run --slow", "print(\"hello\")"]);
}

#[test]
fn table_cells_map_to_their_text() {
    let (source, parsed) = parse_fixture("table");
    let cells: Vec<_> = parsed
        .document
        .blocks
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::TableCell { .. }))
        .map(|b| slice(&source, b.span))
        .collect();
    assert_eq!(
        cells,
        ["Flag", "Meaning", "-q", "Quiet mode", "-v", "Verbose mode"]
    );
}

#[test]
fn the_generated_table_of_contents_is_skipped() {
    let (source, parsed) = parse_fixture("contents");
    let blocks = prose(&source, &parsed);
    assert_eq!(
        blocks,
        pairs(&[
            ("Guide", "Guide"),
            ("Setup", "Setup"),
            ("Setup comes first.", "Setup comes first."),
            ("Usage", "Usage"),
            ("Usage comes second.", "Usage comes second."),
        ])
    );
    let setup = &parsed.document.blocks[1];
    assert_eq!(setup.span.start(), source.find("Setup\n-----").unwrap());
}

#[test]
fn text_docutils_dropped_leaves_no_blocks_and_no_content() {
    let (source, parsed) = parse_fixture("inserted");
    let texts: Vec<_> = prose(&source, &parsed).into_iter().map(|p| p.0).collect();
    assert_eq!(texts, ["Safe", "Before the include.", "After the include."]);
}

#[test]
fn a_word_that_is_not_in_the_source_gets_no_guessed_position() {
    let (source, parsed) = parse_fixture("handwritten/generated-word");
    assert_eq!(
        prose(&source, &parsed),
        pairs(&[("alpha beta", "alpha beta")])
    );
}

#[test]
fn a_crlf_line_break_maps_into_the_whitespace_token() {
    let (source, parsed) = parse_fixture("handwritten/crlf");
    assert_eq!(
        prose(&source, &parsed),
        pairs(&[("alpha beta", "alpha\r\nbeta")])
    );
    let projection = parsed.document.blocks[0].projection.as_ref().unwrap();
    let space = projection
        .tokens()
        .iter()
        .find(|t| t.kind == TokenKind::Collapse)
        .unwrap();
    assert_eq!(slice(&source, space.src), "\r\n");
}

#[test]
fn offsets_count_bytes_of_multibyte_text() {
    let (source, parsed) = parse_fixture("handwritten/unicode");
    let block = &parsed.document.blocks[0];
    assert_eq!(
        (block.span.start(), block.span.end()),
        (0, source.trim_end().len())
    );
    assert_eq!(source.trim_end().len(), 12);
}

#[test]
fn a_bom_shifts_offsets_by_three_bytes() {
    let source = format!("\u{feff}{}", read_source("handwritten/emphasis"));
    let parsed = parse_rst(&source, &options_for(&fake("html-emphasis"))).unwrap();
    let block = &parsed.document.blocks[0];
    assert_eq!(slice(&source, block.span), "Say hi to the *world*.");
    assert_eq!(block.span.start(), 3);
}
