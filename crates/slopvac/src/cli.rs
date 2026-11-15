//! The clap command tree.
//!
//! The CLI epic owns this tree. Other epics expose `command()` and `run()`
//! for their commands and get wired in here. Until then a command is a
//! placeholder that accepts any arguments (see [`PLACEHOLDERS`]).

use clap::{Arg, ArgAction, Command, value_parser};

/// Output formats accepted by `lint --format`.
pub const LINT_FORMATS: [&str; 10] = [
    "text",
    "json",
    "github",
    "sarif",
    "rdjsonl",
    "codeclimate",
    "junit",
    "checkstyle",
    "markdown",
    "html",
];

/// Formats for `rules` and `explain`.
pub const INSPECT_FORMATS: [&str; 3] = ["text", "json", "md"];

/// 2.x options that 3.0 removes, with the pointer printed when one is used.
///
/// They are declared hidden so the error is specific, not a generic usage error.
pub const REMOVED_LINT_OPTIONS: [(&str, &str); 4] = [
    (
        "profile",
        "--profile is removed in 3.0. Set the style preset in slopvac.toml; `slopvac migrate` writes it from the 2.x profile.",
    ),
    (
        "rules-dir",
        "--rules-dir is removed in 3.0. Declare a local pack in slopvac.toml; `slopvac migrate` writes the entry.",
    ),
    ("no-vale", "--no-vale is removed in 3.0. Vale is gone."),
    (
        "comments",
        "--comments is removed in 3.0. Use --mode code-comments.",
    ),
];

/// Commands whose implementation another epic owns, with their owner bead.
// STUB(slopvac-8l4.12.13): replace each placeholder with the owner's `command()`.
pub const PLACEHOLDERS: [(&str, &str, &str); 12] = [
    (
        "terms",
        "Check and list the project terms.",
        "slopvac-8l4.4",
    ),
    (
        "genre",
        "Show or detect the document genre.",
        "slopvac-8l4.4",
    ),
    (
        "validate",
        "Validate configuration, packs and front matter.",
        "slopvac-8l4.4",
    ),
    (
        "plugins",
        "Manage plugins and packs: sync, trust, update.",
        "slopvac-8l4.9",
    ),
    ("packs", "List installed rule packs.", "slopvac-8l4.9"),
    (
        "migrate",
        "Rewrite a 2.x configuration for 3.0.",
        "slopvac-8l4.2",
    ),
    (
        "setup",
        "Install slopvac guidance into agent harnesses.",
        "slopvac-8l4.11",
    ),
    (
        "prime",
        "Print guidance for agents, by topic.",
        "slopvac-8l4.11",
    ),
    (
        "init",
        "Write 3.0 configuration templates.",
        "slopvac-8l4.12.8",
    ),
    (
        "onboard",
        "Print the agent onboarding text.",
        "slopvac-8l4.12.10",
    ),
    (
        "reference",
        "Generate or check the rules reference.",
        "slopvac-8l4.12.10",
    ),
    (
        "report",
        "Preview or submit an issue report.",
        "slopvac-8l4.12.6",
    ),
];

fn placeholder(name: &'static str, about: &'static str) -> Command {
    Command::new(name).about(about).arg(
        Arg::new("args")
            .num_args(0..)
            .trailing_var_arg(true)
            .allow_hyphen_values(true)
            .value_parser(value_parser!(std::ffi::OsString))
            .hide(true),
    )
}

fn format_arg(values: &'static [&'static str], help: &'static str) -> Arg {
    Arg::new("format")
        .long("format")
        .value_name("FORMAT")
        .value_parser(values.to_vec())
        .default_value("text")
        .help(help)
}

fn config_arg() -> Arg {
    Arg::new("config")
        .long("config")
        .value_name("FILE")
        .value_parser(value_parser!(std::path::PathBuf))
        .help("Use this slopvac.toml instead of the nearest one.")
}

fn lint() -> Command {
    let cmd = Command::new("lint")
        .about("Lint prose and source comments. Bare paths run this command.")
        .arg(
            Arg::new("targets")
                .value_name("PATH")
                .num_args(0..)
                .value_parser(value_parser!(std::path::PathBuf))
                .help("Files or directories to lint."),
        )
        .arg(
            Arg::new("stdin")
                .long("stdin")
                .action(ArgAction::SetTrue)
                .help("Lint one document from standard input."),
        )
        .arg(
            Arg::new("language")
                .long("language")
                .value_name("LANG")
                .requires("stdin")
                .help("Name the parser for standard input."),
        )
        .arg(
            Arg::new("mode")
                .long("mode")
                .value_parser(["prose", "code-comments"])
                .help("Input surface."),
        )
        .arg(config_arg())
        .arg(
            Arg::new("category")
                .long("category")
                .action(ArgAction::Append)
                .help("Run only these categories. Repeatable."),
        )
        .arg(
            Arg::new("disable")
                .long("disable")
                .action(ArgAction::Append)
                .help("Disable a category or a qualified rule ID. Repeatable."),
        )
        .arg(format_arg(&LINT_FORMATS, "Report format."))
        .arg(
            Arg::new("out")
                .long("out")
                .value_name("FILE")
                .value_parser(value_parser!(std::path::PathBuf))
                .help("Write the report here instead of standard output."),
        )
        .arg(
            Arg::new("open")
                .long("open")
                .action(ArgAction::SetTrue)
                .help("Open the HTML report in a browser."),
        );
    lint_run_args(cmd)
}

fn lint_run_args(cmd: Command) -> Command {
    let mut cmd = cmd
        .arg(
            Arg::new("min-score")
                .long("min-score")
                .value_parser(value_parser!(f64))
                .help("Fail below this 0-100 score."),
        )
        .arg(
            Arg::new("max-per-100-words")
                .long("max-per-100-words")
                .value_parser(value_parser!(f64))
                .help("Fail above this severity-weighted density."),
        )
        .arg(
            Arg::new("locale")
                .long("locale")
                .value_name("TAG")
                .help("Spelling target: en-US, en-GB, or und to disable."),
        )
        .arg(
            Arg::new("diff-base")
                .long("diff-base")
                .value_name("REV")
                .help("Scope findings to lines added in REV...HEAD."),
        )
        .arg(
            Arg::new("diff-working-tree")
                .long("diff-working-tree")
                .action(ArgAction::SetTrue)
                .help("Scope findings to staged, unstaged and untracked changes."),
        )
        .arg(
            Arg::new("fix")
                .long("fix")
                .action(ArgAction::SetTrue)
                .help("Apply safe replacements."),
        )
        .arg(
            Arg::new("verbose")
                .long("verbose")
                .action(ArgAction::SetTrue)
                .help("Show categories with no findings."),
        )
        .arg(
            Arg::new("explain-config")
                .long("explain-config")
                .action(ArgAction::SetTrue)
                .help("Print each resolved setting with its source, then exit."),
        );
    for (name, _) in REMOVED_LINT_OPTIONS {
        cmd = cmd.arg(Arg::new(name).long(name).num_args(0..=1).hide(true));
    }
    cmd
}

fn explain() -> Command {
    Command::new("explain")
        .about("Explain a rule.")
        .arg(
            Arg::new("rule")
                .value_name("RULE")
                .required(true)
                .help("Qualified rule ID."),
        )
        .arg(format_arg(&INSPECT_FORMATS, "Output format."))
        .arg(config_arg())
}

fn rules() -> Command {
    Command::new("rules")
        .about("List rules.")
        .arg(
            Arg::new("category")
                .long("category")
                .action(ArgAction::Append)
                .help("Only this category. Repeatable."),
        )
        .arg(Arg::new("kind").long("kind").help("Only this match kind."))
        .arg(
            Arg::new("genre")
                .long("genre")
                .help("Only rules that apply to this genre."),
        )
        .arg(
            Arg::new("include-excluded")
                .long("include-excluded")
                .action(ArgAction::SetTrue)
                .help("Include rules the genre excludes."),
        )
        .arg(format_arg(&INSPECT_FORMATS, "Output format."))
        .arg(config_arg())
}

fn doctor() -> Command {
    Command::new("doctor")
        .about("Check the binary, configuration and pack health.")
        .arg(format_arg(&["text", "json"], "Output format."))
        .arg(config_arg())
}

fn self_cmd() -> Command {
    Command::new("self")
        .about("Manage this installation.")
        .subcommand_required(true)
        .subcommand(Command::new("update").about("Update slopvac to the latest release."))
}

fn lsp() -> Command {
    Command::new("lsp").about("Run the language server on standard input and output.")
}

fn completions() -> Command {
    Command::new("completions")
        .about("Print a shell completion script.")
        .arg(
            Arg::new("shell")
                .value_name("SHELL")
                .required(true)
                .value_parser(value_parser!(clap_complete::Shell)),
        )
}

fn man() -> Command {
    Command::new("man").about("Write man pages.").arg(
        Arg::new("out-dir")
            .long("out-dir")
            .value_name("DIR")
            .value_parser(value_parser!(std::path::PathBuf))
            .help("Write one page per command into this directory. Default: print the main page."),
    )
}

/// Build the full command tree.
#[must_use]
pub fn command() -> Command {
    let mut cmd = Command::new("slopvac")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Lint prose for AI-generated text patterns.")
        .long_about(
            "Lint prose and source comments. `slopvac PATH...` lints, as does `slopvac lint PATH...`.\n\n\
             Exit codes: 0 pass, 1 a gate failed, 2 the run was incomplete or the invocation was invalid.",
        )
        .subcommand_required(false)
        .arg(
            Arg::new("offline")
                .long("offline")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Never use the network. SLOPVAC_OFFLINE=1 and `[network] offline = true` do the same, and nothing turns it off."),
        )
        .arg(
            Arg::new("color")
                .long("color")
                .global(true)
                .value_name("WHEN")
                .value_parser(["auto", "always", "never"])
                .help("Colour output. NO_COLOR is honoured."),
        )
        .arg(
            Arg::new("no-color")
                .long("no-color")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Alias for --color never."),
        )
        .subcommand(lint())
        .subcommand(explain())
        .subcommand(rules())
        .subcommand(doctor())
        .subcommand(self_cmd())
        .subcommand(lsp())
        .subcommand(completions())
        .subcommand(man())
        .subcommand(Command::new("cli-reference").about("Print the Markdown CLI reference.").hide(true));
    for (name, about, _owner) in PLACEHOLDERS {
        cmd = cmd.subcommand(placeholder(name, about));
    }
    cmd
}

/// Options that take a value before the subcommand and so must be skipped
/// when looking for the first real token.
const GLOBAL_WITH_VALUE: [&str; 1] = ["--color"];
const GLOBAL_FLAGS: [&str; 2] = ["--offline", "--no-color"];

/// Insert `lint` when the invocation starts with a path or a lint option.
#[must_use]
pub fn insert_default_command(
    args: Vec<std::ffi::OsString>,
    cmd: &Command,
) -> Vec<std::ffi::OsString> {
    let mut i = 1;
    while let Some(arg) = args.get(i).and_then(|a| a.to_str()) {
        if GLOBAL_FLAGS.contains(&arg) || (arg.starts_with("--color=")) {
            i += 1;
        } else if GLOBAL_WITH_VALUE.contains(&arg) {
            i += 2;
        } else {
            break;
        }
    }
    let Some(first) = args.get(i) else {
        return args;
    };
    let first = first.to_string_lossy();
    let known = first == "help"
        || matches!(first.as_ref(), "-h" | "--help" | "-V" | "--version")
        || cmd.get_subcommands().any(|s| s.get_name() == first);
    if known {
        return args;
    }
    let mut out = args;
    out.insert(i, "lint".into());
    out
}
