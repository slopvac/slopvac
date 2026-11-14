//! Which string literals are docstrings, per language.
//!
//! A docstring is decided once per container, when the walk enters it, so the cost is
//! linear in the number of children. Asking a node for its previous or next sibling
//! would scan from the start of the parent each time.

use std::collections::HashSet;
use std::ops::Range;

use tree_sitter::Node;

use crate::extract::{ExtractError, Meter};
use crate::language::Language;

/// Docstring string nodes found so far in one walk, by node id.
#[derive(Default)]
pub(crate) struct Finder {
    strings: HashSet<usize>,
}

impl Finder {
    /// Look at `node` before its children. Returns the byte range of `node` when it is
    /// a docstring string literal.
    pub(crate) fn check(
        &mut self,
        language: Language,
        node: &Node<'_>,
        source: &str,
        meter: &mut Meter,
    ) -> Result<Option<Range<usize>>, ExtractError> {
        match language {
            Language::Python => self.python(node, source, meter)?,
            Language::Julia => self.julia(node, meter)?,
            Language::Elixir => return Ok(elixir(node, source)),
            _ => {}
        }
        Ok(self.strings.contains(&node.id()).then(|| node.byte_range()))
    }

    /// The first statement of a module, or of a class or function body, when it is only
    /// a plain, raw or unicode string. f-strings and bytes are not docstrings.
    fn python(
        &mut self,
        node: &Node<'_>,
        source: &str,
        meter: &mut Meter,
    ) -> Result<(), ExtractError> {
        let body = match node.kind() {
            "module" => *node,
            "function_definition" | "class_definition" => match node.child_by_field_name("body") {
                Some(body) => body,
                None => return Ok(()),
            },
            _ => return Ok(()),
        };
        let mut cursor = body.walk();
        for statement in body.named_children(&mut cursor) {
            meter.step()?;
            if statement.kind() == "comment" {
                continue;
            }
            let string = (statement.kind() == "expression_statement"
                && statement.named_child_count() == 1)
                .then(|| statement.named_child(0))
                .flatten()
                .filter(|child| child.kind() == "string")
                .filter(|child| plain_prefix(source, child.start_byte()));
            if let Some(string) = string {
                self.strings.insert(string.id());
            }
            break;
        }
        Ok(())
    }

    /// A string statement that precedes another statement at file or module level.
    /// Julia attaches it to the definition that follows.
    fn julia(&mut self, node: &Node<'_>, meter: &mut Meter) -> Result<(), ExtractError> {
        if !matches!(node.kind(), "source_file" | "module_definition") {
            return Ok(());
        }
        let mut cursor = node.walk();
        let mut pending: Option<Node<'_>> = None;
        for child in node.named_children(&mut cursor) {
            meter.step()?;
            if child.kind().ends_with("comment") {
                continue;
            }
            if child.kind() == "string_literal" {
                pending = Some(child);
                continue;
            }
            if let Some(string) = pending.take() {
                self.strings.insert(string.id());
            }
        }
        Ok(())
    }
}

/// Whether the string literal at `start` has no prefix, or only `r` or `u`.
fn plain_prefix(source: &str, start: usize) -> bool {
    let prefix: String = source
        .get(start..)
        .unwrap_or_default()
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .collect();
    matches!(prefix.to_ascii_lowercase().as_str(), "" | "r" | "u")
}

/// The string of `@doc`, `@moduledoc` and `@typedoc` attributes.
fn elixir(node: &Node<'_>, source: &str) -> Option<Range<usize>> {
    if node.kind() != "unary_operator" {
        return None;
    }
    let operator = node.child_by_field_name("operator")?;
    if source.get(operator.byte_range())? != "@" {
        return None;
    }
    let call = node
        .child_by_field_name("operand")
        .filter(|operand| operand.kind() == "call")?;
    let target = call.child_by_field_name("target")?;
    let name = source.get(target.byte_range())?;
    if !matches!(name, "doc" | "moduledoc" | "typedoc") {
        return None;
    }
    let mut cursor = call.walk();
    let arguments = call
        .named_children(&mut cursor)
        .find(|child| child.kind() == "arguments")?;
    let string = arguments
        .named_child(0)
        .filter(|child| child.kind() == "string")?;
    Some(string.byte_range())
}
