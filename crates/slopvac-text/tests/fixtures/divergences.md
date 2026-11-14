# Divergences from the Python reference

The Rust text crate matches `slopvac.analyze` except where listed here. The golden suite
(slopvac-8l4.2.7) should expect these differences.

| Area | Python | Rust | Why |
| --- | --- | --- | --- |
| Offset of a vertical list's lead-in sentence when the text starts with whitespace | 0 | the true byte offset | Python's offset does not slice back to the sentence. Tested in `list_lead_in_keeps_its_true_offset_after_leading_whitespace`. |
| Sentence offsets | code points | UTF-8 bytes | The position contract uses bytes. |
| Unicode data version | `str.casefold` and `str.isdigit` use the interpreter's own tables | The same tables, generated from Python 3.13 (Unicode 15.1.0) into `src/unicode_data.rs` by `tools/generate_unicode.py`, whose `--check` detects drift | Python 3.11 ships Unicode 14.0 and 3.12 ships 15.0, so those interpreters differ for characters added after their version. |
| `\w` and `\b` in built-in patterns | `regex` module `\w`, UTS #18: Alphabetic, M, Nd, Pc, Join_Control (Unicode 17 in `regex` 2026.11.9) | The same class spelled as PCRE2 properties, `\p{Alphabetic}\p{M}\p{Nd}\p{Pc}` plus U+200C and U+200D | A scan of every code point finds no extra character and 17,559 missing ones. All are unassigned in Unicode 15.1, because the bundled PCRE2 has older Unicode data than `regex`. A PCRE2 upgrade (slopvac-8l4.3.20) closes the gap. |
