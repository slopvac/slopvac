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
- `compat()` (slopvac-text/src/rewrite.rs) rewrites `\w`/`\W` to the class
  `[\p{Alphabetic}\p{M}\p{Nd}\x{200C}\x{200D}\p{Pc}]` (UTS #18, which is exactly Rust
  `regex`'s Unicode `\w`) and `\b`/`\B` to a 4-lookaround alternation. So the Rust path takes
  the ORIGINAL (pre-compat) pattern: Rust's native `\w`/`\b` equal the compat meaning.

## Spike plan (resume here after a restart)

1. Instrument `slopvac-pcre2::Regex` (spike branch only) with env `SLOPVAC_SPIKE_ENGINE`:
   `interp` (default) | `jit` | `rust` (twin when it compiles) | `split` (twin when in
   parity allowlist `SLOPVAC_SPIKE_ALLOW`) | `parity` (run both, log diffs) | `callout`
   (JIT + PCRE2_AUTO_CALLOUT step budget). Original source passed from slopvac-text
   `try_build`; context patterns (no UCP) get ASCII `\w\d\s\b` on the Rust side.
2. Compile-time dump (`SLOPVAC_SPIKE_DUMP`) records pattern, flags, caller frame -> sources.
3. Parity = every engine call during `slopvac lint` over golden inputs + eval corpus.
4. Shape check prototype over the PCRE2-remainder; adversarial pumping per flagged shape.
5. Timings via /usr/bin/time -l, timeout 300, heavy.sh per run; 64 MiB last.
