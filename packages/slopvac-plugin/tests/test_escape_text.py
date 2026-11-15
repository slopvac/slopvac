"""`escape_text` covers the same characters as the Rust crate `slopvac-sanitize`."""

import pytest

from slopvac_plugin.conformance.runner import escape_text


@pytest.mark.parametrize(
    ("char", "want"),
    [
        ("\x1b", "\\x1b"),
        ("\x9b", "\\x9b"),
        ("\u00ad", "\\xad"),
        ("\u202e", "\\u202e"),
        ("\u200b", "\\u200b"),
        ("\u2028", "\\u2028"),
        ("\u3164", "\\u3164"),
        ("\ufe0f", "\\ufe0f"),
        ("\ufdd0", "\\ufdd0"),
        ("\uffff", "\\uffff"),
        ("\U0010ffff", "\\U0010ffff"),
        ("\U000e0041", "\\U000e0041"),
        ("\ue000", "\\ue000"),
    ],
)
def test_unsafe_characters_become_escapes(char, want):
    assert escape_text(f"a{char}b") == f"a{want}b"


@pytest.mark.parametrize("text", ["Zürich – ok ✓ 日本語 😀", "a\u00a0b", "~ ."])
def test_ordinary_text_is_unchanged(text):
    assert escape_text(text) == text
