# Divergences from the Python reference

The Rust rule matcher matches `slopvac.engine.rule_pattern` on the parity corpus except where
this page lists a difference. The corpus is `pattern-parity.json`: 3,750 texts and 144 lexical
rules, 540,000 (rule, text) pairs. The pairs agree except for the Unicode case-folding gap in
the first row. 83 rules and 149 pairs are affected.

| Area | Python | Rust | Why |
| --- | --- | --- | --- |
| Case folding of U+0130 (İ) and U+0131 (ı) | The `regex` module folds İ onto `i` and ı onto `I` when it ignores case, so `figurative-sits` finds `sİts` | PCRE2 does not fold them, so those matches are absent | PCRE2 does not treat İ and ı as case variants of `i` and `I`. The corpus covers them with the `dotted-i` and `dotless-i` transforms in `generate.py`. Turkish and Azeri text is the realistic source. The differing pairs are the only ones the test tolerates. |
| Unicode version | `regex` 2026.11.9 carries Unicode 17 | The vendored PCRE2 10.46 carries Unicode 16 | A code point added in Unicode 17 is a letter or mark for Python and unassigned for PCRE2. A later PCRE2 closes the gap (slopvac-8l4.3.20). |
| `\w`, `\W`, `\b` and `\B` | UTS #18 word: Alphabetic, marks, Nd, Pc and Join_Control | The same class spelled as PCRE2 properties, rewritten by `slopvac-text` before compiling | PCRE2's own `\w` misses superscripts such as `robust¹`, spacing marks and a joiner inside a word. The rewrite costs match time, so the PCRE2 build policy (slopvac-8l4.3.20) assumes JIT. |
| Offsets | Code points | UTF-8 bytes | The position contract uses bytes. The test converts Python's offsets. |
| `ALLOWED_WORDS` placeholder | The text stays in the pattern until a locale injects words, so it matches nothing real | `(?!)` when no word is given, which never matches | `docs/contract/rules.md` defines the placeholder. The effect is the same unless the text contains `ALLOWED_WORDS`. |
| `\u` and `\U` escapes | Python `regex` reads them; the 2.x data has none since PR #276 | The loader rejects them | The v3 schema rejects both. Rule data uses the literal character or `\x{XXXX}`. |
| Match limits | Unbounded | The `pcre2` crate defaults, so a pathological input returns an error | `STUB(slopvac-8l4.3.25)` replaces the defaults with the policy from slopvac-8l4.3.20. |
| Substitution replacement lookup | `engine.match_substitution` finds the replacement for a matched span | Not ported here | The rules loader compiles the keys and exposes the entries in file order. The engine (slopvac-8l4.3.8) looks up the replacement. |
