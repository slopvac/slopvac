"""`escape_text` and its table agree with the Rust crate `slopvac-sanitize`."""

import importlib.util
import re
from pathlib import Path

import pytest

from slopvac_plugin.conformance._unsafe_ranges import UNSAFE_RANGES
from slopvac_plugin.conformance.runner import escape_text

REPO = Path(__file__).resolve().parents[3]
GENERATOR = REPO / "crates/slopvac-sanitize/tools/gen_tables.py"
RUST_TABLE = REPO / "crates/slopvac-sanitize/src/tables.rs"


def load_generator():
    spec = importlib.util.spec_from_file_location("gen_tables", GENERATOR)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


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
        # Reserved code points that are Default_Ignorable: an unassigned
        # character that a terminal or a reader still skips.
        ("\u2065", "\\u2065"),
        ("\ufff0", "\\ufff0"),
        ("\U000e0080", "\\U000e0080"),
        # A lone surrogate cannot appear in Rust text but can in a Python str.
        ("\ud800", "\\ud800"),
    ],
)
def test_unsafe_characters_become_escapes(char, want):
    assert escape_text(f"a{char}b") == f"a{want}b"


@pytest.mark.parametrize(
    "text",
    [
        "Zürich – ok ✓ 日本語 😀",
        "a\u00a0b",
        "~ .",
        # Unassigned in every Unicode version so far, and not Default_Ignorable.
        "a\u0378b",
        "a\U000e1000b",
    ],
)
def test_ordinary_and_merely_unassigned_text_is_unchanged(text):
    assert escape_text(text) == text


@pytest.mark.parametrize(
    ("text", "want"),
    [
        ("\\u202e", "\\\\u202e"),
        ("a\\b", "a\\\\b"),
        ("\\\x1b", "\\\\\\x1b"),
    ],
)
def test_a_literal_backslash_is_doubled(text, want):
    assert escape_text(text) == want


@pytest.mark.parametrize(
    ("text", "want"),
    [
        ("::error::x", "\\x3a:error::x"),
        # Newline and tab are escaped, so the second "line" is not a line.
        ("ok\n::set-output name=a::b", "ok\\x0a::set-output name=a::b"),
        ("  \t::warning::x", "  \\x09::warning::x"),
        ("  ::warning::x", "  \\x3a:warning::x"),
        ("\u00a0::error::x", "\u00a0\\x3a:error::x"),
        ("##[error]x", "\\x23#[error]x"),
        ("prefix ##[error]x", "prefix \\x23#[error]x"),
        ("mid :: line", "mid :: line"),
        ("std::vec", "std::vec"),
    ],
)
def test_workflow_command_introducers_are_escaped(text, want):
    assert escape_text(text) == want


def test_the_python_table_is_the_generators_output_from_the_pinned_ucd_data():
    gen = load_generator()
    assert tuple(gen.table()) == UNSAFE_RANGES
    assert (
        gen.render_python(gen.table())
        == (
            REPO
            / "packages/slopvac-plugin/src/slopvac_plugin/conformance/_unsafe_ranges.py"
        ).read_text()
    )


def test_the_rust_table_is_the_generators_output_from_the_pinned_ucd_data():
    gen = load_generator()
    assert gen.render_rust(gen.table()) == RUST_TABLE.read_text()


def test_the_rust_and_python_tables_hold_the_same_ranges():
    text = RUST_TABLE.read_text()
    rust = tuple(
        (int(low.replace("_", ""), 16), int(high.replace("_", ""), 16))
        for low, high in re.findall(r"\((0x[0-9a-f_]+), (0x[0-9a-f_]+)\),", text)
    )
    assert rust == UNSAFE_RANGES


def test_the_generator_refuses_data_that_differs_from_its_pinned_digest(tmp_path):
    gen = load_generator()
    tampered = tmp_path / "UnicodeData.subset.txt"
    tampered.write_bytes(b"0000;<control>;Cc;0;BN;;;;;N;;;;;\n")
    with pytest.raises(SystemExit, match="SHA-256"):
        gen.read_verified(tampered, gen.SUBSET_SHA256["UnicodeData.subset.txt"])
