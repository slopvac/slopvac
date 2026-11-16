//! The Fluent export, checked against the shape of the Fluent grammar.
//!
//! No Fluent library is a dependency, so `check_ftl` below is a small grammar check for
//! the subset the exporter writes: `id = text { $param }` messages with `#` comments and
//! indented continuation lines.

mod support;

use std::collections::BTreeSet;

use slopvac_messages::{Catalog, to_ftl};
use support::{Help, ReadError, RunError};

/// A message found by the check: its id, its variables, its text without placeholders.
#[derive(Debug, PartialEq)]
struct Message {
    id: String,
    variables: BTreeSet<String>,
}

fn is_fluent_id(id: &str) -> bool {
    id.starts_with(|c: char| c.is_ascii_alphabetic())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Parse `source`, or say which line breaks the grammar.
fn check_ftl(source: &str) -> Result<Vec<Message>, String> {
    let mut messages: Vec<Message> = Vec::new();
    let mut in_message = false;
    for (n, line) in source.lines().enumerate() {
        let at = |why: &str| format!("line {}: {why}: {line:?}", n + 1);
        if line.is_empty() {
            // A blank line may sit inside a multi-line pattern.
            continue;
        }
        if line.starts_with('#') {
            in_message = false;
            if line.len() > 1 && !line.starts_with("# ") && !line.starts_with("##") {
                return Err(at("a comment needs a space after #"));
            }
            continue;
        }
        let text = if line.starts_with(' ') {
            if !in_message {
                return Err(at("indented line outside a message"));
            }
            if line.trim_start().starts_with(['[', '*', '.']) {
                return Err(at("continuation line starts an attribute or variant"));
            }
            line.trim_start()
        } else {
            let (id, text) = line
                .split_once(" = ")
                .ok_or_else(|| at("expected `id = text`"))?;
            if !is_fluent_id(id) {
                return Err(at("the id is not a Fluent identifier"));
            }
            messages.push(Message {
                id: id.to_owned(),
                variables: BTreeSet::new(),
            });
            in_message = true;
            text
        };
        let message = messages.last_mut().expect("a message is open");
        scan_text(text, &mut message.variables).map_err(|why| at(&why))?;
    }
    Ok(messages)
}

/// Check the placeholders of one line: `{ $name }` or `{ "x" }`, and no bare brace.
fn scan_text(text: &str, variables: &mut BTreeSet<String>) -> Result<(), String> {
    let mut rest = text;
    while let Some(pos) = rest.find(['{', '}']) {
        if rest[pos..].starts_with('}') {
            return Err("a bare `}`".to_owned());
        }
        let inner = rest[pos + 1..].trim_start();
        if let Some(quoted) = inner.strip_prefix('"') {
            let close = quoted.find('"').ok_or("an unclosed string")?;
            if close != 1 {
                return Err("a string placeholder holds one character".to_owned());
            }
            rest = quoted[close + 1..]
                .trim_start()
                .strip_prefix('}')
                .ok_or("a string placeholder is not closed by `}`")?;
        } else if let Some(var) = inner.strip_prefix('$') {
            let end = var.find('}').ok_or("an unclosed `{`")?;
            let name = var[..end].trim_end();
            if !is_fluent_id(name) {
                return Err(format!("`{name}` is not a Fluent variable name"));
            }
            variables.insert(name.to_owned());
            rest = &var[end + 1..];
        } else {
            return Err("a placeholder is neither a variable nor a string".to_owned());
        }
    }
    Ok(())
}

fn all_entries() -> Vec<slopvac_messages::Entry> {
    [ReadError::entries(), RunError::entries(), Help::entries()].concat()
}

#[test]
fn the_export_is_one_message_per_entry_with_the_code_as_id() {
    let entries = all_entries();
    let messages = check_ftl(&to_ftl(&entries)).expect("valid shape");
    let ids: Vec<_> = messages.iter().map(|m| m.id.as_str()).collect();
    let codes: Vec<_> = entries.iter().map(|e| e.code).collect();
    assert_eq!(ids, codes);
}

#[test]
fn each_message_uses_exactly_the_placeholders_of_its_template() {
    let entries = all_entries();
    let messages = check_ftl(&to_ftl(&entries)).expect("valid shape");
    let used: Vec<Vec<&str>> = messages
        .iter()
        .map(|m| m.variables.iter().map(String::as_str).collect())
        .collect();
    assert_eq!(
        used,
        [
            vec!["path", "reason"],
            vec!["found", "limit"],
            vec!["key"],
            vec!["key"],
            vec![],
            vec![],
            vec![],
        ]
    );
}

#[test]
fn placeholders_use_fluent_syntax_and_braces_are_quoted() {
    let ftl = to_ftl(ReadError::entries());
    assert!(ftl.contains("file-unreadable = cannot read { $path }: { $reason }\n"));
    assert!(ftl.contains("brace-example = write { \"{\" }key{ \"}\" } to name { $key }\n"));
}

#[test]
fn a_multi_line_template_becomes_indented_continuation_lines() {
    let ftl = to_ftl(ReadError::entries());
    let expected = "two-lines = first line\n    second { $key }\n\n    { \".\" }dotted line\n";
    assert!(ftl.contains(expected), "{ftl}");
}

#[test]
fn counts_are_documented_for_later_plural_selection() {
    let ftl = to_ftl(ReadError::entries());
    assert!(ftl.contains("#   $found (count, number)\n"));
}

#[test]
fn the_summary_becomes_the_comment_above_the_message() {
    let ftl = to_ftl(Help::entries());
    assert!(ftl.starts_with("# The `--check` flag.\nhelp-check = Fail when a finding remains\n"));
}

#[test]
fn the_check_rejects_text_outside_the_grammar() {
    for bad in [
        "_start = x\n",
        "1id = x\n",
        "id = { $1 }\n",
        "id = a } b\n",
        "id = { $open\n",
        "id = { \"xy\" }\n",
        "  orphan\n",
        "id = ok\n    .attr = x\n",
        "no equals sign\n",
    ] {
        assert!(check_ftl(bad).is_err(), "accepted {bad:?}");
    }
}
