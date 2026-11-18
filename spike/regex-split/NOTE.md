# Regex engine spike: split PCRE2 / Rust `regex`, shape check, JIT budget

Status: IN PROGRESS (partial results survive restarts; sections fill in as measured).

Measured commit: origin/main 13509167e1b46da977877f313314c017c1129adb
Branch: spike/regex-split (local spike, never a PR)

## Facts gathered so far

- Shipped core catalog: `crates/slopvac-rules/tests/fixtures/core-v3/*.json`, embedded by
  `crates/slopvac/src/rule_catalog.rs`. 237 rules: pattern 111, judgement 68,
  substitution 18, tokens 17, metric 13, structure 6, vocabulary 4. 94 rules have
  exceptions, 11 have allowlists.
- Pattern compile path: `slopvac-text/src/pattern.rs` -> `compat()` rewrite ->
  `slopvac_pcre2::Regex::new` with UTF + UCP, caseless per rule, interpreter only.
- Context patterns: `slopvac-context/src/pattern.rs` compile WITHOUT UCP.
- `regex` in Cargo.lock: 1.13.1 (regex-automata 0.4.18, regex-syntax 0.8.11).
