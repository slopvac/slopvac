//! Which string literals are docstrings, per language.

use std::ops::Range;

use tree_sitter::Node;

use crate::language::Language;

/// The byte range of the string literal that `node` holds as a docstring, if any.
pub(crate) fn range(language: Language, node: &Node<'_>, source: &str) -> Option<Range<usize>> {
    match language {
        Language::Python => python(node),
        Language::Elixir => elixir(node, source),
        Language::Julia => julia(node),
        _ => None,
    }
}

/// The first statement of a module, or of a class or function body, when it is only a
/// string. Concatenated and f-strings are not docstrings.
fn python(node: &Node<'_>) -> Option<Range<usize>> {
    if node.kind() != "expression_statement" || node.named_child_count() != 1 {
        return None;
    }
    let string = node
        .named_child(0)
        .filter(|child| child.kind() == "string")?;
    let parent = node.parent()?;
    let documents = match parent.kind() {
        "module" => true,
        "block" => parent.parent().is_some_and(|owner| {
            matches!(owner.kind(), "function_definition" | "class_definition")
        }),
        _ => false,
    };
    if !documents || !is_first_statement(node, &parent) {
        return None;
    }
    Some(string.byte_range())
}

/// Whether `node` is the first child of `parent` that is not a comment.
fn is_first_statement(node: &Node<'_>, parent: &Node<'_>) -> bool {
    let mut cursor = parent.walk();
    parent
        .named_children(&mut cursor)
        .find(|child| child.kind() != "comment")
        .is_some_and(|first| first.id() == node.id())
}

/// The string of `@doc`, `@moduledoc` and `@typedoc` attributes.
fn elixir(node: &Node<'_>, source: &str) -> Option<Range<usize>> {
    if node.kind() != "unary_operator" {
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

/// A string statement at module level that precedes another statement. Julia attaches it
/// to the definition that follows.
fn julia(node: &Node<'_>) -> Option<Range<usize>> {
    if node.kind() != "string_literal" {
        return None;
    }
    let parent = node.parent()?;
    if !matches!(parent.kind(), "source_file" | "module_definition") {
        return None;
    }
    let mut next = node.next_named_sibling();
    while next.is_some_and(|sibling| sibling.kind().ends_with("comment")) {
        next = next.and_then(|sibling| sibling.next_named_sibling());
    }
    next.filter(|sibling| sibling.kind() != "string_literal")?;
    Some(node.byte_range())
}
