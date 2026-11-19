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

| Batch | Head | Host | Toolchain | Content | Load start/end | State |
|---|---|---|---|---|---|---|
| b1 | a46aa04683 | dev aarch64, 16 cores | rustc 1.99.1 (58cf8f9c3) | build + parity over golden, docs, k8s (eval skipped by mistake, see below) | 36.6 / 36.0 (no timings in b1) | done, rc=0, 2026-11-19 11:25-11:39Z |
| b2 | 752411bf5d | dev aarch64, 16 cores | 1.99.1 | builds, eval parity re-run, differential fuzz, summary, shape check, recheck, inputs (no timings) | n/a | submitted |

Timing stages (batch 3: prep, small, large, e2e) all run on dev: arm64 (aarch64 Linux),
16 cores. Each starts only at a 1-minute load under 4. A stage whose end load is 4 or more is
discarded and rerun. Each stage below records its host, arch, nproc and start and end load.
CI runs on x86_64, so read these arm64 numbers as ratios between modes, not as absolute
CI times. The golden RST input, which b1 ran without docutils, gets its parity run again in
the prep stage now that the helper puts the pinned docutils venv on PATH.

### b1 results (parity, interpreter vs Rust twin, every engine call)

| Lint run (strict profile) | Exit | Wall (parity mode, both engines, load ~36) | Peak RSS | Distinct patterns | Exercised | Twin does not compile | Calls with a span difference |
|---|---|---|---|---|---|---|---|
| golden inputs, en-US / en-GB | 2 / 2 | 1.9 s | 225 MiB | 954 | 880 | 74 | 0 |
| slopvac `docs/`, en-US / en-GB | 1 / 1 | 8.0 s | 276 MiB | 408 | 346 | 62 | 0 |
| Kubernetes `content/en` (2530 `.md`, 101 MB; commit be58364903; CC BY 4.0), en-US / en-GB | 2 / 2 | 260-267 s | 1.5 GiB | 957 | 883 | 74 | 0 (16.8M calls) |
| eval corpus | 0 | 0.4 s | 43 MiB | 146 | 111 | 35 | 0, **void**: its files end in `.corpus`, so lint skipped them all; b2 re-runs it |

- Exit 2 on golden is the RST input with no docutils on dev (`input` incomplete); exit 2 on k8s is
  `input-unsupported` for its `.html` files. Every other file was linted.
- The edge suite (13 strings per pattern) also found no difference. Corpus parity alone is weak
  evidence: for example `^\W*(?:added|changed|fixed|removed)\W*$` (no `(?m)`) has a real
  semantic gap (PCRE2 `$` also matches before a final `\n`, Rust `$` does not) that no corpus
  call hit, because headings reach it without a trailing newline. b2 adds a differential fuzz
  (3000 generated strings per pattern from its own words plus newline, CRLF, case-fold and
  mark pieces, a quarter with a trailing `\n`) before any pattern counts as identical.
- Exercised counts include engine-built patterns: about 740 of the ~955 are per-rule probes
  built in `slopvac-engine/src/matching.rs` (`\A(?:probe)\z` exception anchors).

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
