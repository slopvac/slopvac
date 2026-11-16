//! The generated CLI reference documents every visible argument, default, possible value,
//! built-in help and version flag, and `help` subcommand, and keeps help text inert.
#![cfg(test)]

#[path = "../examples/cli-reference.rs"]
#[expect(
    dead_code,
    reason = "the example's main() and helpers are only used when it runs as an example"
)]
mod reference;

use clap::{Arg, ArgAction, Command};

fn page<'a>(pages: &'a [(String, String)], path: &str) -> &'a str {
    &pages
        .iter()
        .find(|(p, _)| p == path)
        .unwrap_or_else(|| panic!("{path} missing"))
        .1
}

/// Table rows of a page: the lines that start with a table cell holding a code span.
fn rows(text: &str) -> Vec<&str> {
    text.lines().filter(|l| l.starts_with("| `")).collect()
}

/// The cell text that must start the row of `arg`, written from clap's accessors, not the
/// generator's code.
fn label(arg: &Arg) -> String {
    let mut parts = Vec::new();
    if let Some(short) = arg.get_short() {
        parts.push(format!("`-{short}`"));
    }
    if let Some(long) = arg.get_long() {
        parts.push(format!("`--{long}`"));
    }
    if parts.is_empty() {
        let name = arg.get_value_names().map_or_else(
            || arg.get_id().as_str().to_uppercase(),
            |n| n[0].to_string(),
        );
        parts.push(format!("`<{name}>`"));
    }
    format!("| {}", parts.join(", "))
}

/// The part of `page` that documents `where_` alone: from its heading to the next heading of
/// any level, so rows of a parent or a child never count for it.
fn own_section<'a>(page: &'a str, where_: &str) -> &'a str {
    let lines: Vec<&str> = page.split_inclusive('\n').collect();
    let at = lines
        .iter()
        .position(|l| {
            l.starts_with('#')
                && l.trim_end()
                    .trim_start_matches('#')
                    .trim()
                    .trim_matches('`')
                    == where_
        })
        .unwrap_or_else(|| panic!("no heading for {where_}"));
    let len = lines[at + 1..]
        .iter()
        .position(|l| l.starts_with('#'))
        .map_or(lines.len() - at, |n| n + 1);
    let start: usize = lines[..at].iter().map(|l| l.len()).sum();
    let size: usize = lines[at..at + len].iter().map(|l| l.len()).sum();
    &page[start..start + size]
}

/// Check each visible argument of `cmd` inside its own section of `page`, then recurse.
fn check_command(cmd: &Command, page: &str, where_: &str, checked: &mut usize) {
    let own = own_section(page, where_);
    assert!(
        own.contains(&format!("Usage: {where_}")),
        "{where_}: no usage line in its section"
    );
    check_args(cmd, own, where_, checked);
    for sub in cmd.get_subcommands().filter(|s| !s.is_hide_set()) {
        if sub.get_name() == "help" {
            let hint = format!("{where_} help [COMMAND]");
            assert!(own.contains(&hint), "{where_}: no help subcommand note");
        } else {
            check_command(sub, page, &format!("{where_} {}", sub.get_name()), checked);
        }
    }
}

fn check_args(cmd: &Command, text: &str, where_: &str, checked: &mut usize) {
    for arg in cmd.get_arguments().filter(|a| !a.is_hide_set()) {
        if arg.is_global_set() && where_ != "slopvac" {
            continue;
        }
        let start = label(arg);
        let row = rows(text)
            .into_iter()
            .find(|r| r.starts_with(&start) && r[start.len()..].starts_with([' ', ',', '|']))
            .unwrap_or_else(|| panic!("{where_}: no row for {start} in {:?}", rows(text)));
        for default in arg.get_default_values() {
            let want = format!("`{}`", default.to_string_lossy());
            assert!(row.contains(&want), "{where_}: {row} lacks default {want}");
        }
        for value in arg
            .get_possible_values()
            .iter()
            .filter(|v| !v.is_hide_set())
        {
            let want = format!("`{}`", value.get_name());
            assert!(row.contains(&want), "{where_}: {row} lacks value {want}");
        }
        *checked += 1;
    }
}

#[test]
fn every_visible_argument_has_a_row_with_its_defaults_and_possible_values() {
    let mut root = slopvac_cli::cli::command();
    root.build();
    let pages = reference::pages(&root);
    let mut checked = 0;
    for top in root
        .get_subcommands()
        .filter(|s| !s.is_hide_set() && s.get_name() != "help")
    {
        let path = format!("cli/{}.md", top.get_name());
        check_command(
            top,
            page(&pages, &path),
            &format!("slopvac {}", top.get_name()),
            &mut checked,
        );
    }
    check_args(&root, page(&pages, "cli/index.md"), "slopvac", &mut checked);
    assert!(checked > 50, "walked only {checked} arguments");
}

#[test]
fn the_lint_format_row_is_exactly_the_clap_definition() {
    let pages = reference::pages(&slopvac_cli::cli::command());
    let want = "| `--format` | one of `text`, `json`, `github`, `sarif`, `rdjsonl`, `codeclimate`, \
                `junit`, `checkstyle`, `markdown`, `html` | `text` | Report format. |";
    assert!(rows(page(&pages, "cli/lint.md")).contains(&want));
}

#[test]
fn positional_arguments_and_the_required_marker_are_documented() {
    let pages = reference::pages(&slopvac_cli::cli::command());
    assert!(
        rows(page(&pages, "cli/lint.md"))
            .contains(&"| `<PATH>` | `PATH` | none | Files or directories to lint. |")
    );
    assert!(rows(page(&pages, "cli/completions.md")).contains(
        &"| `<SHELL>` (required) | one of `bash`, `elvish`, `fish`, `powershell`, `zsh` | none |  |"
    ));
}

#[test]
fn built_in_help_on_every_command_and_version_on_the_root_are_documented() {
    let pages = reference::pages(&slopvac_cli::cli::command());
    let help = "| `-h`, `--help` | none | none | Print help |";
    assert!(rows(page(&pages, "cli/lint.md")).contains(&help));
    let index = page(&pages, "cli/index.md");
    assert!(rows(index).contains(&"| `-V`, `--version` | none | none | Print version |"));
    assert!(index.contains("## Root options"));
    assert!(index.contains("- `help`: Print this message or the help of the given subcommand(s)."));
}

#[test]
fn hidden_commands_get_no_page_and_the_index_lists_global_flags_and_exit_codes() {
    let pages = reference::pages(&slopvac_cli::cli::command());
    assert!(pages.iter().all(|(p, _)| p != "cli/cli-reference.md"));
    let index = page(&pages, "cli/index.md");
    for row in [
        "| `--offline` | none | `false` |",
        "| `--color` | one of `auto`, `always`, `never` | none |",
        "| `0` | The run finished and no gate failed. |",
        "| `1` | The run finished and at least one gate failed. |",
        "| `2` | The run was incomplete or the invocation was invalid. |",
    ] {
        assert!(
            rows(index).iter().any(|r| r.starts_with(row)),
            "index lacks {row}"
        );
    }
}

#[test]
fn help_text_with_markup_stays_inert_and_cannot_break_the_table() {
    let cmd = Command::new("slopvac").subcommand(
        Command::new("demo")
            .about("See [the docs](other.md) <b>now</b>")
            .arg(
                Arg::new("mode")
                    .long("mode")
                    .action(ArgAction::Set)
                    .default_value("a|b`c")
                    .help("Pick `x` | `y`, or ![img](i.png) and *bold*."),
            ),
    );
    let pages = reference::pages(&cmd);
    let text = page(&pages, "cli/demo.md");
    assert!(text.contains("See \\[the docs\\]\\(other.md\\) &lt;b&gt;now&lt;/b&gt;"));
    let row = rows(text)
        .into_iter()
        .find(|r| r.starts_with("| `--mode`"))
        .expect("mode row");
    assert_eq!(
        row,
        "| `--mode` | `MODE` | ``a\\|b`c`` | Pick \\`x\\` \\| \\`y\\`, or \\!\\[img\\]\\(i.png\\) and \\*bold\\*. |"
    );
}
