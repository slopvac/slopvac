//! `slopvac report`: preview an issue report, and file it only on `--gh --yes`.
//!
//! Contract: `docs/contract/policies.md`, section 1.5. The command files nothing unless the caller
//! passes `--gh --yes`, and the body in the file is byte for byte the body it prints.
//!
//! - [`fields`]: the field definitions. The GitHub issue forms are generated from them.
//! - [`forms`]: the form renderer.
//! - [`issue`]: validation of the `--field` values and the issue body.
//! - [`scrub`]: removal of paths, names and secrets from everything that leaves the machine.
//! - [`github`]: the prefilled form URL and the browser.
//! - [`gh`]: the GitHub CLI, which holds the login.

pub mod fields;
pub mod forms;
pub mod gate;
pub mod gh;
pub mod github;
pub mod issue;
pub mod messages;
pub mod scrub;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use clap::{Arg, ArgAction, ArgMatches, Command, value_parser};

use crate::env::Env;
use crate::errors::CliError;
use crate::exit;

use self::messages::ReportError;

/// The repository that receives reports.
pub const REPOSITORY: &str = "slopvac/slopvac";

/// What a form element asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A single line.
    Input,
    /// Several lines.
    Textarea,
    /// One or more of a fixed set of options.
    Dropdown,
    /// A list of boxes to tick.
    Checkboxes,
}

/// One field of an issue form.
#[derive(Debug, Clone)]
pub struct Field {
    /// The id GitHub uses for prefill and the key of `--field ID=VALUE`.
    pub id: &'static str,
    /// The label.
    pub label: &'static str,
    /// What the field asks for.
    pub kind: Kind,
    /// Whether the form requires a value. Checkbox requirements are per box.
    pub required: bool,
    /// The code-fence language of a rendered textarea.
    pub render: Option<&'static str>,
    /// The help text.
    pub description: Option<&'static str>,
    /// Dropdown options.
    pub options: &'static [&'static str],
    /// Checkbox options with their required flags.
    pub checkbox_options: &'static [(&'static str, bool)],
    /// Whether a dropdown takes several options.
    pub multiple: bool,
}

impl Field {
    /// A required field with no extras.
    #[must_use]
    pub fn new(id: &'static str, label: &'static str, kind: Kind) -> Self {
        Self {
            id,
            label,
            kind,
            required: true,
            render: None,
            description: None,
            options: &[],
            checkbox_options: &[],
            multiple: false,
        }
    }

    /// Make the field optional.
    #[must_use]
    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    /// Render the answer as a fenced block in this language.
    #[must_use]
    pub fn render(mut self, language: &'static str) -> Self {
        self.render = Some(language);
        self
    }

    /// Set the help text.
    #[must_use]
    pub fn describe(mut self, text: &'static str) -> Self {
        self.description = Some(text);
        self
    }

    /// Set the dropdown options.
    #[must_use]
    pub fn options(mut self, options: &'static [&'static str]) -> Self {
        self.options = options;
        self
    }

    /// Let a dropdown take several options.
    #[must_use]
    pub fn multiple(mut self) -> Self {
        self.multiple = true;
        self
    }

    /// Set the checkbox options.
    #[must_use]
    pub fn checks(mut self, options: &'static [(&'static str, bool)]) -> Self {
        self.checkbox_options = options;
        self
    }
}

/// One kind of report and its issue form.
#[derive(Debug, Clone)]
pub struct Category {
    /// The command-line key.
    pub key: &'static str,
    /// The file in `.github/ISSUE_TEMPLATE`.
    pub form_file: &'static str,
    /// The form name.
    pub form_name: &'static str,
    /// The form description.
    pub description: &'static str,
    /// The title prefix.
    pub title_prefix: &'static str,
    /// The labels the form sets.
    pub labels: &'static [&'static str],
    /// When to use this category.
    pub meaning: &'static str,
    /// The fields, in form order.
    pub fields: Vec<Field>,
}

/// Every category.
#[must_use]
pub fn categories() -> Vec<Category> {
    fields::categories()
}

/// The `report` subcommand.
#[must_use]
pub fn command() -> Command {
    let keys: Vec<&'static str> = categories().iter().map(|c| c.key).collect();
    Command::new("report")
        .about("Preview an issue report, and file it with the GitHub CLI.")
        .long_about(
            "Preview an issue report for the slopvac repository. The command prints the exact \
             target, title and body, with paths, user and host names and secrets removed \
             as far as it can tell, and warns about text that may still hold a credential. It \
             posts nothing. It prints a prefilled issue-form URL. --body-file writes the body to a \
             file, --gh prints the exact `gh issue create` command for that file, and --gh --yes \
             runs it through the GitHub CLI after you have read the preview. Slopvac never reads \
             a token: `gh` holds the login. Run `slopvac prime report` for the workflow an agent \
             follows, including the user's approval of the exact text.",
        )
        .arg(
            Arg::new("category")
                .value_name("CATEGORY")
                .required(true)
                .value_parser(keys)
                .help("The kind of report; it picks the issue form."),
        )
        .arg(
            Arg::new("title")
                .long("title")
                .value_name("TEXT")
                .required(true)
                .help("The issue title, without the category prefix."),
        )
        .arg(
            Arg::new("field")
                .long("field")
                .value_name("ID=VALUE")
                .action(ArgAction::Append)
                .help("A form field. Repeat for several fields, or several dropdown options."),
        )
        .arg(
            Arg::new("include-text")
                .long("include-text")
                .value_name("FILE")
                .value_parser(value_parser!(PathBuf))
                .help("Read the document sample for the `input` field from this file."),
        )
        .arg(
            Arg::new("body-file")
                .long("body-file")
                .value_name("FILE")
                .value_parser(value_parser!(PathBuf))
                .help("Write the previewed body to this file. The write replaces the file in one step."),
        )
        .arg(
            Arg::new("gh")
                .long("gh")
                .action(ArgAction::SetTrue)
                .requires("body-file")
                .help("Print the exact `gh issue create` command for the body file."),
        )
        .arg(
            Arg::new("yes")
                .long("yes")
                .action(ArgAction::SetTrue)
                .requires("gh")
                .help("With --gh: run that command after the preview. Offline mode refuses it."),
        )
        .arg(
            Arg::new("open")
                .long("open")
                .action(ArgAction::SetTrue)
                .help("Open the prefilled issue form in a browser."),
        )
}

/// Everything `run` touches outside the process, so tests can stand in for it.
pub struct Deps<'a> {
    /// Environment access.
    pub env: &'a dyn Env,
    /// Offline mode is on.
    pub offline: bool,
    /// Working directory. Redaction removes it from the report.
    pub cwd: PathBuf,
    /// Opens a URL in a browser.
    pub opener: &'a dyn github::Opener,
    /// The GitHub CLI.
    pub gh: &'a dyn gh::Gh,
}

/// Run `slopvac report`. Writes the preview to `out` and the warnings to `err_out`.
pub fn run(
    m: &ArgMatches,
    deps: &Deps<'_>,
    out: &mut dyn std::io::Write,
    err_out: &mut dyn std::io::Write,
) -> Result<u8, CliError> {
    let key = m.get_one::<String>("category").expect("required");
    let category = categories()
        .into_iter()
        .find(|c| c.key == key)
        .expect("clap limits the values to the keys");
    let answers: Vec<String> = m
        .get_many::<String>("field")
        .map(|v| v.cloned().collect())
        .unwrap_or_default();
    let text = m
        .get_one::<PathBuf>("include-text")
        .map(|p| issue::read_sample(p))
        .transpose()?;
    let run_gh = m.get_flag("yes");
    if run_gh && deps.offline {
        return Err(ReportError::OfflineNetworkNeeded.into());
    }
    let scrubber = scrub::Scrubber::from_env(deps.env, &deps.cwd);
    let report = issue::build(
        &category,
        m.get_one::<String>("title").expect("required"),
        &answers,
        text.as_deref(),
        env!("CARGO_PKG_VERSION"),
        &scrubber,
    )?;

    let url = github::prefilled_url(&category, &report);
    let mut preview = report.preview();
    preview.push_str("\n---\n");
    write_all(out, preview.as_bytes())?;
    for warning in &report.warnings {
        write_all(err_out, format!("warning: {warning}\n").as_bytes())?;
    }

    let body_file = m.get_one::<PathBuf>("body-file");
    if let Some(path) = body_file {
        write_body(path, &report.body)?;
        write_all(out, b"Wrote the body to the file you named.\n")?;
    }
    if m.get_flag("gh") {
        let path = body_file.expect("clap: --gh requires --body-file");
        let line = gh::command_line(&report.title, path).ok_or_else(|| {
            CliError::from(ReportError::BodyFile {
                reason: "the path must be UTF-8 without control characters, so the command can be printed"
                    .to_owned(),
            })
        })?;
        write_all(out, format!("GitHub CLI command:\n{line}\n").as_bytes())?;
        if run_gh {
            create_with_gh(deps, &report.title, path, out)?;
        }
    } else {
        write_all(out, footer(&url, &report.labels).as_bytes())?;
    }
    if !run_gh {
        open_if_asked(m, deps, &url.url);
    }
    Ok(exit::PASS)
}

/// Run `gh issue create` after the preview: check `gh` and its login first.
fn create_with_gh(
    deps: &Deps<'_>,
    title: &str,
    body_file: &Path,
    out: &mut dyn std::io::Write,
) -> Result<(), CliError> {
    deps.gh.check().map_err(|e| match e {
        gh::GhError::Missing => CliError::from(ReportError::GhMissing),
        _ => CliError::from(ReportError::GhNotLoggedIn),
    })?;
    let created = deps.gh.create(title, body_file).map_err(|e| {
        CliError::from(ReportError::GhFailed {
            reason: match e {
                gh::GhError::Failed(Some(code)) => format!("exit {code}"),
                _ => "it failed or timed out".to_owned(),
            },
        })
    })?;
    write_all(out, format!("Created: {created}\n").as_bytes())
}

/// Replace `path` with `body` in one step: a temporary file in the same directory, then a rename.
fn write_body(path: &Path, body: &str) -> Result<(), CliError> {
    let fail = |why: &str| {
        CliError::from(ReportError::BodyFile {
            reason: why.to_owned(),
        })
    };
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::Builder::new()
        .prefix(".slopvac-report-")
        .tempfile_in(dir)
        .map_err(|_| fail("cannot create a file there"))?;
    std::io::Write::write_all(&mut tmp, body.as_bytes())
        .map_err(|_| fail("cannot write the file"))?;
    tmp.persist(path)
        .map_err(|_| fail("cannot replace that path"))?;
    Ok(())
}

fn footer(url: &github::FormUrl, labels: &[String]) -> String {
    let mut text = String::from(
        "Nothing was posted. Add --body-file FILE --gh to print the GitHub CLI command, and --yes to run it.\n",
    );
    let _ = writeln!(text, "Prefilled issue form: {}", url.url);
    let _ = writeln!(
        text,
        "The issue form adds the labels: {}",
        labels.join(", ")
    );
    if !url.omitted.is_empty() {
        let _ = writeln!(
            text,
            "The URL is too long to hold these fields; paste them from the preview: {}",
            url.omitted.join(", ")
        );
    }
    text
}

fn open_if_asked(m: &ArgMatches, deps: &Deps<'_>, url: &str) {
    // Offline mode opens no remote page either: the URL is printed for the user to use.
    if m.get_flag("open") && !deps.offline {
        // A failure to start a browser is not a failure of the report: the URL is printed.
        let _ = deps.opener.open(url);
    }
}

fn write_all(out: &mut dyn std::io::Write, bytes: &[u8]) -> Result<(), CliError> {
    out.write_all(bytes)
        .and_then(|()| out.flush())
        .map_err(|e| {
            CliError::new(
                "output-failed",
                format!("cannot write output: {}", e.kind()),
            )
        })
}
