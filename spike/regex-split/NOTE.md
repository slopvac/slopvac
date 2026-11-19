# Regex engine spike: split PCRE2 / Rust `regex`, shape check, JIT budget

Status: IN PROGRESS (partial results survive restarts; sections fill in as measured).

- Measured source: origin/main 13509167e1 plus the spike overlay; spike branch head is
  named per batch below.
- Host: dev, aarch64 Linux, 16 cores, PCRE2 10.46 (vendored by pcre2-sys 0.2.10) with JIT
  on aarch64 glibc. Toolchain is recorded per batch (`toolchain.txt`); the pin is 1.99.1.
  Any run before the helper fix (cargo 1.97.1) is void; none of this spike's results come
  from before it.
- Branch: spike/regex-split (local spike, never a PR).

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

## Method (harness on this branch)

- `overlay/crates/slopvac-pcre2/src/{lib.rs,spike.rs}` plus `tools/apply.py` build an
  instrumented copy of the tree (never main code). `SLOPVAC_SPIKE_ENGINE` selects
  `interp` (shipped policy) | `jit` | `rust` | `split` (twin for ids in
  `SLOPVAC_SPIKE_ALLOW`) | `parity` (interpreter result returned, Rust twin run on every
  call and every difference counted) | `callout` (JIT + `PCRE2_AUTO_CALLOUT`, a callout that
  counts steps and returns `PCRE2_ERROR_CALLOUT` past `SLOPVAC_SPIKE_BUDGET`, default 10M).
  `SLOPVAC_SPIKE_DUMP` writes one JSON line per distinct compiled pattern with its caller
  (`#[track_caller]` through the wrappers), rule label, Rust compile error and diff counts.
- Rust twin: the pre-`compat()` source for UCP patterns; ASCII `\w\d\s\b` spellings for the
  non-UCP context patterns; a whole-pattern `(?<![\w-])...(?![\w-])` guard becomes consumed
  context plus a named group, matched with a backtrack of one character. find_all follows
  Python finditer / slopvac-pcre2 (`NOTEMPTY_ATSTART` after an empty match).
- Parity: every engine call during `slopvac lint --format json` (strict profile, en-US and
  en-GB) over the golden inputs, the eval corpus, slopvac `docs/` and the Kubernetes
  website `content/en` (CC BY 4.0, commit recorded), plus a fixed edge suite run once per
  pattern (`spike.rs` `EDGES`: newlines, CRLF, NEL/LS, Unicode digits, Kelvin/long s/sharp s,
  combining marks, ZWJ/ZWNJ, curly quotes, emoji).
- Malicious inputs: `bench/` runs each pattern over one input per process mode, median of 3,
  per-pattern timeout 300 s (record "> 300 s, killed"), process `ulimit -v` cap, VmHWM.
  Inputs (`tools/gen_inputs.py`): one-block prose and one-sentence prose at 1, 8, 64 MiB;
  nesting 1k/10k (parens, mixed brackets/quotes, list indent); 1 MiB runs (spaces, dots,
  dashes, `a`, words, caps, quotes); near-miss runs of every core bad example; recheck
  attack strings per core pattern (native, 64 KiB, 1 MiB); malicious plugin patterns with
  their pumping strings (30 chars, 64 KiB, 1 MiB).
- Shape check prototype: `tools/shape_check.py`, hand-written recursive-descent parser for
  the PCRE2 subset plus tree rules (see section 2).

## Batches

| Batch | Head | Toolchain | Content | Load start/end | State |
|---|---|---|---|---|---|
| b1 | a46aa04683 | 1.99.1 (helper) | build + parity over 4 corpora | see log | running |

## 1. Split counts

(pending b1)

## 2. Shape check

(pending)

## 3. End-to-end timing

(pending)

## 4. Proposed contract text

(pending)

## 5. Implementation beads

(pending)
