//! Python docstrings: a string that is the first statement of a module, class or function
//! body is extracted. Every expected value below was worked out by hand from the source.

mod common;

use common::{fragments, variants};
use rstest::rstest;
use slopvac_comments::{CommentKind, Fragment, Language};

/// The texts of the documentation fragments, in source order.
fn docstrings(source: &str) -> Vec<String> {
    fragments(Language::Python, source)
        .iter()
        .filter(|fragment| fragment.kind() == CommentKind::Doc)
        .map(|fragment| fragment.text().to_owned())
        .collect()
}

#[rstest]
#[case::module_after_leading_comments(
    "# header\n# more\n\n\"\"\"Module doc.\"\"\"\nx = 1\n",
    &["Module doc."]
)]
#[case::module_after_shebang_and_encoding_line(
    "#!/usr/bin/env python\n# -*- coding: utf-8 -*-\n'''Module doc.'''\n",
    &["Module doc."]
)]
#[case::module_after_blank_lines("\n\n\n\"Module doc.\"\n", &["Module doc."])]
#[case::class("class C:\n    \"\"\"Class doc.\"\"\"\n    x = 1\n", &["Class doc."])]
#[case::function("def f():\n    \"\"\"Function doc.\"\"\"\n    return 1\n", &["Function doc."])]
#[case::async_function("async def f():\n    '''Async doc.'''\n", &["Async doc."])]
#[case::class_and_method(
    "class C:\n    \"\"\"Class doc.\"\"\"\n\n    def m(self):\n        \"\"\"Method doc.\"\"\"\n",
    &["Class doc.", "Method doc."]
)]
#[case::decorated_function("@decorator(1)\ndef f():\n    \"Doc.\"\n", &["Doc."])]
#[case::comment_between_header_and_docstring(
    "def f():  # trailing\n    # note\n\n    \"\"\"Doc.\"\"\"\n",
    &["Doc."]
)]
#[case::body_on_the_header_line("def f(): \"Doc.\"\n", &["Doc."])]
#[case::header_over_several_lines(
    "def f(\n    a: int = 1,\n    b: dict = {1: 2},\n) -> \"X\":\n    \"\"\"Doc.\"\"\"\n",
    &["Doc."]
)]
#[case::class_with_bases("class C(Base, metaclass=Meta):\n    \"\"\"Doc.\"\"\"\n", &["Doc."])]
#[case::raw_prefix("def f():\n    r\"\"\"Raw \\d doc.\"\"\"\n", &["Raw \\d doc."])]
#[case::upper_case_raw_prefix("def f():\n    R'Raw doc.'\n", &["Raw doc."])]
#[case::unicode_prefix("def f():\n    u\"Unicode doc.\"\n", &["Unicode doc."])]
#[case::statement_ends_at_semicolon("def f():\n    \"Doc.\"; x = 1\n", &["Doc."])]
#[case::statement_ends_at_end_of_file("\"Doc.\"", &["Doc."])]
#[case::implicit_concatenation(
    "def f():\n    \"First \" \"second.\"\n",
    &["First", "second."]
)]
#[case::paragraphs_join_lines_and_split_at_blank_lines(
    "def f():\n    \"\"\"First.\n\n    Second\n    line.\n    \"\"\"\n",
    &["First.", "Second line."]
)]
#[case::escaped_quote_does_not_end_the_string(
    "def f():\n    \"Say \\\"hi\\\" now.\"\n",
    &["Say \\\"hi\\\" now."]
)]
fn a_leading_string_of_a_body_is_extracted(#[case] source: &str, #[case] expected: &[&str]) {
    assert_eq!(docstrings(source), expected);
}

#[rstest]
#[case::assignment("x = \"not a doc\"\n")]
#[case::second_statement_of_a_module("import os\n\"\"\"Not a doc.\"\"\"\n")]
#[case::second_statement_of_a_function("def f():\n    x = 1\n    \"Not a doc.\"\n")]
#[case::call_argument("print(\"not a doc\")\n")]
#[case::method_call_on_the_string("\"not a doc\".strip()\n")]
#[case::string_in_an_expression("def f():\n    \"a\" + \"b\"\n")]
#[case::parenthesised_expression_statement("def f():\n    (\"a\")\n")]
#[case::return_annotation("def f() -> \"X\":\n    pass\n")]
#[case::default_value("def f(a=\"x\"):\n    pass\n")]
#[case::base_class_argument("class C(\"x\"):\n    pass\n")]
#[case::body_of_an_if("if x:\n    \"Not a doc.\"\n")]
#[case::body_of_a_loop_in_a_function("def f():\n    for i in y:\n        \"Not a doc.\"\n")]
#[case::bytes_literal("def f():\n    b\"not a doc\"\n")]
#[case::raw_bytes_literal("def f():\n    rb\"not a doc\"\n")]
#[case::f_string("def f():\n    f\"not {a} doc\"\n")]
#[case::lambda_body("f = lambda: \"x\"\n")]
#[case::variable_annotation("x: \"int\" = 1\n")]
#[case::unterminated_string("def f():\n    \"\"\"never closed\n")]
#[case::empty_module("")]
fn any_other_string_is_not_extracted(#[case] source: &str) {
    assert_eq!(docstrings(source), Vec::<String>::new());
}

#[test]
fn a_docstring_inside_a_nested_function_is_extracted() {
    let source = "def outer():\n    def inner():\n        \"Inner doc.\"\n";
    assert_eq!(docstrings(source), ["Inner doc."]);
}

#[test]
fn a_comment_after_a_docstring_is_still_a_comment() {
    let source = "def f():\n    \"Doc.\"  # note\n";
    let got: Vec<_> = fragments(Language::Python, source)
        .iter()
        .map(|fragment| (fragment.kind(), fragment.text().to_owned()))
        .collect();
    assert_eq!(
        got,
        [
            (CommentKind::Doc, "Doc.".to_owned()),
            (CommentKind::Line, "note".to_owned())
        ]
    );
}

/// The one documentation fragment of `source`.
fn only(source: &str) -> Fragment {
    let mut found: Vec<_> = fragments(Language::Python, source)
        .into_iter()
        .filter(|fragment| fragment.kind() == CommentKind::Doc)
        .collect();
    assert_eq!(found.len(), 1, "{source:?}");
    found.remove(0)
}

#[rstest]
#[case::plain(
    "def f():\n    \"\"\"Doc é 😀.\"\"\"\n",
    "\"\"\"Doc é 😀.\"\"\"",
    "Doc é 😀."
)]
#[case::raw_prefix("r'''Raw doc.'''\n", "r'''Raw doc.'''", "Raw doc.")]
#[case::single_quote("class C:\n    'Doc.'\n", "'Doc.'", "Doc.")]
fn the_comment_span_covers_the_literal_and_the_text_span_the_text(
    #[case] source: &str,
    #[case] literal: &str,
    #[case] text: &str,
) {
    let fragment = only(source);
    let literal_start = source.find(literal).expect("literal in source");
    let text_start = literal_start + literal.find(text).expect("text in literal");
    assert_eq!(fragment.comment_span().start(), literal_start);
    assert_eq!(fragment.comment_span().end(), literal_start + literal.len());
    assert_eq!(fragment.text_span().start(), text_start);
    assert_eq!(fragment.text_span().end(), text_start + text.len());
}

#[test]
fn each_line_of_a_multi_line_docstring_maps_to_its_own_bytes() {
    let source = "def f():\n    \"\"\"\n    Ünï 😀\n      more\n    \"\"\"\n";
    let fragment = only(source);
    assert_eq!(fragment.text(), "Ünï 😀 more");
    let spans: Vec<_> = fragment
        .projection()
        .tokens()
        .iter()
        .filter(|token| token.kind == slopvac_comments::TokenKind::Identity)
        .map(|token| &source[token.src.start()..token.src.end()])
        .collect();
    assert_eq!(spans, ["Ünï 😀", "more"]);
}

#[test]
fn line_endings_and_a_byte_order_mark_do_not_change_the_docstring() {
    let source = "# c\n\ndef f():\n    \"\"\"First.\n\n    Second\n    line.\n    \"\"\"\n";
    for (name, variant) in variants(source) {
        let got = docstrings(&variant);
        assert_eq!(got, ["First.", "Second line."], "{name}");
    }
}

#[test]
fn a_byte_order_mark_does_not_hide_the_module_docstring() {
    let source = "\u{feff}\"\"\"Doc.\"\"\"\n";
    let fragment = only(source);
    assert_eq!(fragment.text_span().start(), 3 + 3);
}

#[test]
fn only_the_first_of_two_string_statements_is_a_docstring() {
    let source = "def f():\n    \"Doc.\"\n    \"Not a doc.\"\n";
    assert_eq!(docstrings(source), ["Doc."]);
}
