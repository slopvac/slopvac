//! Write the CLI reference pages under a directory.
//!
//! `cargo run -p slopvac --example reference -- <out-dir>` writes `cli/index.md` and one
//! page per top-level command below `<out-dir>`. It reads the clap tree through its public
//! API and changes nothing in it.

use std::fmt::Write as _;
use std::path::PathBuf;

use clap::{Arg, Command};
use slopvac_cli::exit;

/// Exit codes with their meaning, from [`slopvac_cli::exit`].
const EXIT_CODES: [(u8, &str); 3] = [
    (exit::PASS, "The run finished and no gate failed."),
    (
        exit::GATE_FAILED,
        "The run finished and at least one gate failed.",
    ),
    (
        exit::INCOMPLETE,
        "The run was incomplete or the invocation was invalid.",
    ),
];

fn main() -> std::io::Result<()> {
    let out = PathBuf::from(std::env::args().nth(1).expect("usage: reference <out-dir>"));
    for (path, text) in pages(&slopvac_cli::cli::command()) {
        let file = out.join(path);
        std::fs::create_dir_all(file.parent().expect("page paths have a directory"))?;
        std::fs::write(file, text)?;
    }
    Ok(())
}

/// Every page as a (site path, Markdown) pair.
pub(crate) fn pages(root: &Command) -> Vec<(String, String)> {
    let mut root = root.clone();
    root.build();
    let mut pages = vec![("cli/index.md".to_owned(), index(&root))];
    for sub in visible_subcommands(&root) {
        let mut text = format!("# slopvac {}\n\n", sub.get_name());
        section(&mut text, sub, &format!("slopvac {}", sub.get_name()), 2);
        pages.push((format!("cli/{}.md", sub.get_name()), tidy(text)));
    }
    pages
}

fn visible_subcommands(cmd: &Command) -> impl Iterator<Item = &Command> {
    cmd.get_subcommands()
        .filter(|s| !s.is_hide_set() && s.get_name() != "help")
}

fn shown_args(cmd: &Command) -> impl Iterator<Item = &Arg> {
    cmd.get_arguments()
        .filter(|a| !a.is_hide_set() && !matches!(a.get_id().as_str(), "help" | "version"))
}

fn tidy(mut text: String) -> String {
    text.truncate(text.trim_end().len());
    text.push('\n');
    text
}

fn index(root: &Command) -> String {
    let mut out = String::from("# Command-line reference\n\n");
    if let Some(about) = root.get_about() {
        let _ = writeln!(out, "{about}\n");
    }
    out.push_str("Run `slopvac PATH...` or `slopvac lint PATH...` to lint.\n\n");
    out.push_str("## Commands\n\n");
    for sub in visible_subcommands(root) {
        let name = sub.get_name();
        let about = sub.get_about().map(ToString::to_string).unwrap_or_default();
        let _ = writeln!(out, "- [`{name}`]({name}.md): {}", one_line(&about));
    }
    out.push_str("\n## Global options\n\n");
    options_table(&mut out, root, true);
    out.push_str("## Exit codes\n\n| Code | Meaning |\n| --- | --- |\n");
    for (code, meaning) in EXIT_CODES {
        let _ = writeln!(out, "| `{code}` | {meaning} |");
    }
    tidy(out)
}

fn section(out: &mut String, cmd: &Command, path: &str, depth: usize) {
    if depth > 2 {
        let _ = writeln!(out, "{} `{path}`\n", "#".repeat(depth));
    }
    if let Some(about) = cmd.get_long_about().or_else(|| cmd.get_about()) {
        let _ = writeln!(out, "{about}\n");
    }
    let mut usage = cmd.clone();
    let _ = writeln!(out, "```text\n{}\n```\n", usage.render_usage());
    if cmd.get_all_aliases().next().is_some() {
        let aliases: Vec<_> = cmd.get_all_aliases().map(|a| format!("`{a}`")).collect();
        let _ = writeln!(out, "Aliases: {}.\n", aliases.join(", "));
    }
    options_table(out, cmd, false);
    for sub in visible_subcommands(cmd) {
        section(out, sub, &format!("{path} {}", sub.get_name()), depth + 1);
    }
}

/// A table of the arguments of `cmd`. `global` picks the global ones or the local ones.
fn options_table(out: &mut String, cmd: &Command, global: bool) {
    let args: Vec<_> = shown_args(cmd)
        .filter(|a| a.is_global_set() == global)
        .collect();
    if args.is_empty() {
        if global {
            out.push_str("None.\n\n");
        }
        return;
    }
    out.push_str("| Argument | Value | Default | Description |\n| --- | --- | --- | --- |\n");
    for arg in args {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} |",
            name(arg),
            value(arg),
            defaults(arg),
            description(arg)
        );
    }
    out.push('\n');
}

fn name(arg: &Arg) -> String {
    let mut names = Vec::new();
    if let Some(short) = arg.get_short() {
        names.push(format!("`-{short}`"));
    }
    if let Some(long) = arg.get_long() {
        names.push(format!("`--{long}`"));
    }
    if names.is_empty() {
        names.push(format!("`<{}>`", value_name(arg)));
    }
    if arg.is_required_set() {
        names.push("(required)".to_owned());
    }
    names.join(", ")
}

fn value_name(arg: &Arg) -> String {
    arg.get_value_names()
        .and_then(|n| n.first())
        .map_or_else(|| arg.get_id().as_str().to_uppercase(), ToString::to_string)
}

fn value(arg: &Arg) -> String {
    if !arg.get_action().takes_values() {
        return "none".to_owned();
    }
    let possible: Vec<_> = arg
        .get_possible_values()
        .iter()
        .filter(|v| !v.is_hide_set())
        .map(|v| format!("`{}`", v.get_name()))
        .collect();
    if possible.is_empty() {
        format!("`{}`", value_name(arg))
    } else {
        format!("one of {}", possible.join(", "))
    }
}

fn defaults(arg: &Arg) -> String {
    let values: Vec<_> = arg
        .get_default_values()
        .iter()
        .map(|v| format!("`{}`", v.to_string_lossy()))
        .collect();
    if values.is_empty() {
        "none".to_owned()
    } else {
        values.join(", ")
    }
}

fn description(arg: &Arg) -> String {
    let help = arg
        .get_long_help()
        .or_else(|| arg.get_help())
        .map(ToString::to_string)
        .unwrap_or_default();
    cell(&help)
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn cell(text: &str) -> String {
    one_line(text).replace('|', "\\|").replace('<', "&lt;")
}
