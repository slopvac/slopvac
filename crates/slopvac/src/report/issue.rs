//! Validate `--field` answers and build the issue.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use super::messages::ReportError;
use super::scrub::Scrubber;
use super::{Category, Field, Kind, REPOSITORY};
use crate::errors::CliError;

/// Largest sample `--include-text` reads.
pub const MAX_SAMPLE_BYTES: u64 = 256 * 1024;
/// Longest summary `--title` takes. GitHub caps a whole title at 256 characters.
pub const MAX_TITLE_CHARS: usize = 200;
/// The label an agent report carries.
pub const AGENT_LABEL: &str = "agent-reported";
/// The tick that marks a report as written by an agent.
const REPORTER_FIELD: &str = "reporter";
/// The field that holds document text.
const TEXT_FIELD: &str = "input";
/// What GitHub writes for an empty optional field.
const NO_RESPONSE: &str = "_No response_";

/// Read the document sample for `--include-text`.
///
/// The file must be a regular file of valid UTF-8 within [`MAX_SAMPLE_BYTES`]. The error does not
/// name the path.
pub fn read_sample(path: &Path) -> Result<String, CliError> {
    let fail = |why: &str| {
        CliError::from(ReportError::SampleUnreadable {
            reason: why.to_owned(),
        })
    };
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A FIFO with no writer would block an ordinary open.
        opts.custom_flags(i32::try_from(rustix::fs::OFlags::NONBLOCK.bits()).unwrap_or_default());
    }
    let file = opts.open(path).map_err(|_| fail("cannot open the file"))?;
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return Err(fail("not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_SAMPLE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| fail("cannot read the file"))?;
    if bytes.len() as u64 > MAX_SAMPLE_BYTES {
        return Err(fail("the file is larger than 256 KiB"));
    }
    String::from_utf8(bytes).map_err(|_| fail("the file is not valid UTF-8"))
}

/// A checked and cleaned report, ready to preview.
#[derive(Debug, Clone)]
pub struct Report {
    /// The full title: prefix plus summary.
    pub title: String,
    /// The labels.
    pub labels: Vec<String>,
    /// The issue body in GitHub's form layout.
    pub body: String,
    /// The cleaned answers by field id, for the prefilled URL.
    pub answers: BTreeMap<&'static str, Vec<String>>,
    /// Warnings about text that may hold a credential or private data.
    pub warnings: Vec<String>,
}

impl Report {
    /// The text printed before anything is sent.
    #[must_use]
    pub fn preview(&self) -> String {
        format!(
            "Target: {REPOSITORY}\nTitle: {}\n\n{}\n",
            self.title, self.body
        )
    }
}

fn error(detail: impl Into<String>) -> CliError {
    ReportError::Invalid {
        detail: detail.into(),
    }
    .into()
}

/// Collects the warnings for text that looks like a credential or private data. The scrub is a
/// best-effort layer, so a warning asks the reader to look before posting; it never stops the run
/// and never quotes the text.
#[derive(Debug, Default)]
struct Warnings(Vec<String>);

impl Warnings {
    fn note(&mut self, field: &str, found: Option<super::gate::Kind>) {
        if let Some(kind) = found {
            let text = format!(
                "possible {} in field `{field}`: review it before posting",
                kind.label()
            );
            if !self.0.contains(&text) {
                self.0.push(text);
            }
        }
    }
}

fn truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "yes" | "1" | "on" | "x"
    )
}

/// Split `ID=VALUE` and check the id belongs to the category.
fn parse_answers(
    category: &Category,
    answers: &[String],
) -> Result<BTreeMap<&'static str, Vec<String>>, CliError> {
    let mut map: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    for answer in answers {
        let (id, value) = answer
            .split_once('=')
            .ok_or_else(|| error("--field takes ID=VALUE"))?;
        // The id the caller typed is not echoed: it may be a pasted secret.
        let field = category.fields.iter().find(|f| f.id == id).ok_or_else(|| {
            let known: Vec<&str> = category.fields.iter().map(|f| f.id).collect();
            error(format!(
                "unknown field id for `{}`; its fields are: {}",
                category.key,
                known.join(", ")
            ))
        })?;
        if field.id == TEXT_FIELD {
            return Err(error(
                "the `input` field holds document text; pass it with --include-text FILE so the preview shows it",
            ));
        }
        map.entry(field.id).or_default().push(value.to_owned());
    }
    Ok(map)
}

/// A terminal acts on escape sequences (cursor moves, OSC 52 clipboard writes) in the preview, so
/// text with a control character other than a tab or a newline is refused. A CRLF pair becomes a
/// newline first, because files from Windows carry it. The refusal does not echo the text.
fn clean_controls(given: &mut BTreeMap<&'static str, Vec<String>>) -> Result<(), CliError> {
    for values in given.values_mut() {
        for value in values {
            if value.contains("\r\n") {
                *value = value.replace("\r\n", "\n");
            }
            if value
                .chars()
                .any(|c| c.is_control() && c != '\t' && c != '\n')
            {
                return Err(error(
                    "a field or the document text holds a control character such as an escape sequence",
                ));
            }
        }
    }
    Ok(())
}

fn check(field: &Field, values: &[String]) -> Result<(), CliError> {
    match field.kind {
        Kind::Dropdown => {
            if values.len() > 1 && !field.multiple {
                return Err(error(format!("`{}` takes one option", field.id)));
            }
            for v in values {
                if !field.options.contains(&v.as_str()) {
                    return Err(error(format!(
                        "`{}` must be one of: {}",
                        field.id,
                        field.options.join(" | ")
                    )));
                }
            }
        }
        Kind::Input => {
            if values.len() > 1 || values.iter().any(|v| v.contains('\n')) {
                return Err(error(format!("`{}` is a single line", field.id)));
            }
        }
        Kind::Textarea | Kind::Checkboxes => {
            if values.len() > 1 {
                return Err(error(format!("`{}` was given more than once", field.id)));
            }
        }
    }
    Ok(())
}

fn fence(language: &str, text: &str) -> String {
    // A fence longer than any backtick run inside the text cannot be closed by the text.
    let longest = text
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or(0)
        .max(2);
    let ticks = "`".repeat(longest + 1);
    format!("{ticks}{language}\n{text}\n{ticks}")
}

/// The summary is one non-empty line within the title limit.
fn check_summary(summary: &str) -> Result<(), CliError> {
    if summary.trim().is_empty() || summary.contains('\n') {
        return Err(error("--title takes one non-empty line"));
    }
    if summary.chars().any(char::is_control) {
        return Err(error("--title holds a control character"));
    }
    if summary.chars().count() > MAX_TITLE_CHARS {
        return Err(error(format!(
            "--title is longer than {MAX_TITLE_CHARS} characters; GitHub caps a title at 256 with its prefix"
        )));
    }
    Ok(())
}

/// Build the report for `category`.
///
/// `answers` are the `--field ID=VALUE` strings, `sample` the `--include-text` content, and
/// `version` the slopvac version that fills the `version` field when none was given.
pub fn build(
    category: &Category,
    summary: &str,
    answers: &[String],
    sample: Option<&str>,
    version: &str,
    scrubber: &Scrubber,
) -> Result<Report, CliError> {
    check_summary(summary)?;
    let mut given = parse_answers(category, answers)?;
    if let Some(text) = sample {
        if !category.fields.iter().any(|f| f.id == TEXT_FIELD) {
            return Err(error(format!(
                "`{}` has no document text field, so --include-text does not apply",
                category.key
            )));
        }
        given.insert(TEXT_FIELD, vec![text.to_owned()]);
    }
    given
        .entry("version")
        .or_insert_with(|| vec![version.to_owned()]);
    clean_controls(&mut given)?;

    let mut sections = Vec::new();
    let mut cleaned: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    let mut missing = Vec::new();
    let mut agent = false;
    let mut warnings = Warnings::default();
    for field in &category.fields {
        let values = given.get(field.id).cloned().unwrap_or_default();
        check(field, &values)?;
        for value in &values {
            warnings.note(field.id, scrubber.check_raw(value));
        }
        let values: Vec<String> = values
            .iter()
            .map(|v| {
                if field.id == TEXT_FIELD {
                    scrubber.sample(v)
                } else {
                    scrubber.field(v)
                }
            })
            .collect();
        let joined = values.join(", ");
        warnings.note(field.id, scrubber.check(&joined));
        let heading = format!("### {}", field.label);
        match field.kind {
            Kind::Checkboxes => {
                let ticked = values.first().is_some_and(|v| truthy(v));
                if field.id == REPORTER_FIELD {
                    agent = ticked;
                }
                let lines: Vec<String> = field
                    .checkbox_options
                    .iter()
                    .map(|(label, required)| {
                        if *required && !ticked {
                            missing.push(field.id);
                        }
                        format!("- [{}] {label}", if ticked { "x" } else { " " })
                    })
                    .collect();
                sections.push(format!("{heading}\n\n{}", lines.join("\n")));
            }
            _ => {
                if joined.trim().is_empty() {
                    if field.required {
                        missing.push(field.id);
                    }
                    sections.push(format!("{heading}\n\n{NO_RESPONSE}"));
                } else if let (Kind::Textarea, Some(language)) = (field.kind, field.render) {
                    sections.push(format!("{heading}\n\n{}", fence(language, &joined)));
                } else {
                    sections.push(format!("{heading}\n\n{joined}"));
                }
            }
        }
        if !values.is_empty() {
            cleaned.insert(field.id, values);
        }
    }
    if !missing.is_empty() {
        missing.dedup();
        return Err(ReportError::Incomplete {
            fields: missing.join(", "),
        }
        .into());
    }

    warnings.note("title", scrubber.check_raw(summary));
    let title = format!("{}{}", category.title_prefix, scrubber.field(summary));
    warnings.note("title", scrubber.check(&title));
    let body = sections.join("\n\n");
    let mut labels: Vec<String> = category.labels.iter().map(|l| (*l).to_owned()).collect();
    if agent {
        labels.push(AGENT_LABEL.to_owned());
    }
    Ok(Report {
        title,
        labels,
        body,
        answers: cleaned,
        warnings: warnings.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::MapEnv;
    use crate::report::categories;

    fn scrubber() -> Scrubber {
        Scrubber::from_env(&MapEnv::default(), Path::new("/nonexistent-cwd-for-test"))
    }

    fn category(key: &str) -> Category {
        categories().into_iter().find(|c| c.key == key).unwrap()
    }

    fn bug_answers() -> Vec<String> {
        [
            "reporter=true",
            "agent=Claude Code",
            "model=claude-opus",
            "harness=omp 1.0",
            "latest=Yes",
            "duplicates=true",
            "environment={\"ok\":true}",
            "command=slopvac lint --stdin",
            "what-happened=it crashed",
            "expected=no crash",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
    }

    #[test]
    fn a_complete_bug_report_renders_in_github_layout() {
        let r = build(
            &category("bug"),
            "crash on empty input",
            &bug_answers(),
            None,
            "3.0.0",
            &scrubber(),
        )
        .unwrap();
        assert_eq!(r.title, "[bug]: crash on empty input");
        assert_eq!(r.labels, ["bug", "needs-triage", "agent-reported"]);
        let want = "### Reporter\n\n- [x] Reported by an AI agent\n\n\
### Agent\n\nClaude Code\n\n\
### Model\n\nclaude-opus\n\n\
### Harness and version\n\nomp 1.0\n\n\
### slopvac version\n\n3.0.0\n\n\
### Reproduces on the latest release\n\nYes\n\n\
### Duplicate search\n\n- [x] I searched existing issues for duplicates\n\n\
### Environment\n\n```json\n{\"ok\":true}\n```\n\n\
### Command\n\n```shell\nslopvac lint --stdin\n```\n\n\
### What happened\n\nit crashed\n\n\
### Expected behavior\n\nno crash\n\n\
### Output\n\n_No response_\n\n\
### Configuration\n\n_No response_";
        assert_eq!(r.body, want);
    }

    #[test]
    fn an_untouched_reporter_box_adds_no_agent_label() {
        let mut a = bug_answers();
        a.retain(|s| !s.starts_with("reporter="));
        let r = build(&category("bug"), "x", &a, None, "3.0.0", &scrubber()).unwrap();
        assert_eq!(r.labels, ["bug", "needs-triage"]);
        assert!(r.body.contains("- [ ] Reported by an AI agent"));
    }

    #[test]
    fn missing_required_fields_are_listed() {
        let err = build(&category("bug"), "x", &[], None, "3.0.0", &scrubber()).unwrap_err();
        assert_eq!(err.code, "report-incomplete");
        for id in [
            "agent",
            "model",
            "harness",
            "latest",
            "duplicates",
            "environment",
            "command",
            "what-happened",
            "expected",
        ] {
            assert!(err.message.contains(id), "{id}: {}", err.message);
        }
        assert!(
            !err.message.contains("version,"),
            "version defaults: {}",
            err.message
        );
    }

    #[test]
    fn proposals_do_not_require_environment_and_command() {
        let a: Vec<String> = [
            "agent=a",
            "model=m",
            "harness=h",
            "latest=Yes",
            "duplicates=true",
            "pattern=p",
            "bad-examples=b",
            "good-examples=g",
            "kind=Unsure",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        assert!(build(&category("new-rule"), "x", &a, None, "3.0.0", &scrubber()).is_ok());
    }

    #[test]
    fn bad_answers_are_refused() {
        let c = category("bug");
        let s = scrubber();
        for (answers, needle) in [
            (vec!["nope=1".to_owned()], "unknown field id"),
            (vec!["agent".to_owned()], "ID=VALUE"),
            (vec!["latest=Maybe".to_owned()], "must be one of"),
            (vec!["agent=a\nb".to_owned()], "single line"),
            (
                vec!["expected=a".to_owned(), "expected=b".to_owned()],
                "more than once",
            ),
            (vec!["input=text".to_owned()], "unknown field id"),
        ] {
            let err = build(&c, "x", &answers, None, "3.0.0", &s).unwrap_err();
            assert_eq!(err.code, "report-invalid", "{answers:?}");
            assert!(err.message.contains(needle), "{answers:?}: {}", err.message);
        }
    }

    #[test]
    fn document_text_comes_only_from_the_sample() {
        let c = category("false-positive");
        let err = build(
            &c,
            "x",
            &["input=text".to_owned()],
            None,
            "3.0.0",
            &scrubber(),
        )
        .unwrap_err();
        assert!(err.message.contains("--include-text"), "{}", err.message);
        let err = build(
            &category("bug"),
            "x",
            &[],
            Some("text"),
            "3.0.0",
            &scrubber(),
        )
        .unwrap_err();
        assert!(
            err.message.contains("no document text field"),
            "{}",
            err.message
        );
    }

    #[test]
    fn a_sample_with_backticks_stays_inside_its_fence() {
        let c = category("false-positive");
        let a: Vec<String> = [
            "agent=a",
            "model=m",
            "harness=h",
            "latest=Yes",
            "duplicates=true",
            "environment=e",
            "command=c",
            "rule-id=core:x",
            "context=g p",
            "finding={}",
            "why=w",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        let sample = "before\n```\n## Injected heading\n```\n````\nafter";
        let r = build(&c, "x", &a, Some(sample), "3.0.0", &scrubber()).unwrap();
        // The field opens with a fence longer than any run in the sample (five backticks vs four).
        let start = r.body.find("### Flagged text\n\n").unwrap() + "### Flagged text\n\n".len();
        let section = &r.body[start..];
        let open_line = section.lines().next().unwrap();
        assert_eq!(open_line, "`````markdown");
        let after_open = &section[open_line.len() + 1..];
        // The sample comes out whole, then exactly one closing fence, then the next section.
        let (inside, rest) = after_open.split_once("\n`````\n").expect("a closing fence");
        assert_eq!(inside, sample);
        assert!(rest.starts_with("\n### "), "{rest}");
        assert!(!rest.contains("Injected heading"), "{rest}");
    }

    #[test]
    fn multiple_dropdown_options_join_with_commas() {
        let c = category("mis-tagged");
        let a: Vec<String> = [
            "agent=a",
            "model=m",
            "harness=h",
            "latest=Yes",
            "duplicates=true",
            "environment=e",
            "command=c",
            "rule-id=core:x",
            "context=g p",
            "finding={}",
            "problem=Wrong rule",
            "problem=Wrong severity",
            "expected=e",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        let r = build(&c, "x", &a, Some("sample"), "3.0.0", &scrubber()).unwrap();
        assert!(
            r.body
                .contains("### What is wrong\n\nWrong rule, Wrong severity"),
            "{}",
            r.body
        );
    }

    #[test]
    fn a_title_must_be_one_line() {
        let c = category("bug");
        for t in ["", "  ", "a\nb"] {
            assert!(
                build(&c, t, &bug_answers(), None, "3.0.0", &scrubber()).is_err(),
                "{t:?}"
            );
        }
    }
}
