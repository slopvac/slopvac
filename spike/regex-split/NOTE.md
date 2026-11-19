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
| b2 | 752411bf5d | dev aarch64, 16 cores | 1.99.1 | builds, eval parity re-run, random fuzz, summary, shape check, inputs (recheck failed: ESM import) | n/a (no timings) | done, rc=0, 14:25-14:31Z |
| b2b | 23c8854367 | dev aarch64, 16 cores | 1.99.1 | directed fuzz, gap scan, RST reruns, shape check, recheck | n/a (no timings) | done, rc=0, 14:37-14:46Z |
| b2c | ec0af61eb3 | dev aarch64, 16 cores | 1.99.1 | shape check rerun (fixed parser), RST reruns via `SLOPVAC_RST_CONVERTER`, stats | n/a (no timings) | done, rc=0, 15:20Z |
| t-prep | 8f32d8061e | dev aarch64, 16 cores | 1.99.1 | timing binaries (instrumented, bench, uninstrumented main), inputs, recheck attack strings | n/a (no timings) | done, rc=0, 15:25-15:34Z |
| t-* | 78992a08e0 | dev aarch64, 16 cores | 1.99.1 | 13 timing stages, one mode each, through `tools/gate_run.sh` (each starts only at 1-min load < 4, up to 60 min per stage, DISCARD on end load >= 4, 64 MiB last) | per stage, `load.txt` | waiting for load < 4 since 15:37Z (load 9.7) |

The timing set leaves out the ~740 `matching.rs` exception probes. They're anchored
(`\A...\z`) and only run on a finding's span, so timing them on whole documents measures
nothing.

Timing stages (batch 3: prep, small, large, e2e) all run on dev: arm64 (aarch64 Linux),
16 cores. Each starts only at a 1-minute load under 4. A stage whose end load is 4 or more is
discarded and rerun. Each stage below records its host, arch, nproc and start and end load.
CI runs on x86_64, so read these arm64 numbers as ratios between modes, not as absolute
CI times. The RST inputs get parity reruns in b2c (see below).

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
  evidence, so b2 and b2b add two differential fuzzes before a pattern counts as identical:
  3000 random strings per pattern from its own words plus newline, CRLF, case-fold and mark
  pieces (b2), and about 150 strings per pattern sampled from the pattern's own parse tree, each
  wrapped in 12 variants (trailing `\n`, `\n\n`, CRLF, leading newline, surrounding words,
  upper and title case, NBSP/LS) (b2b).
- Exercised counts include engine-built patterns: about 740 of the ~955 are per-rule probes
  built in `slopvac-engine/src/matching.rs` (`\A(?:probe)\z` exception anchors).

### b2 / b2b (dev aarch64, 1.99.1, heads 752411bf5d / 23c8854367; no timings)

- Eval corpus parity re-run with real extensions (`tools/eval_copy.py`: md 452, txt 19, rst 18,
  json 15, c 4, rs 2, js 2, py 1, java 1): exit 2 in both locales, from `input-unsupported` on
  the `.txt` files and, in b2, the `.rst` files with no docutils. 0 span differences.
- b2b's golden `sample.rst` run exited 1 with empty stderr. Exit 1 is the findings gate, and
  the no-docutils failure is exit 2 with a `pip install docutils` message (b1), so the engine read
  the RST input through the helper's venv. b2b's own `import docutils` check failed because it
  used the mise `python3` on PATH, not the venv. That is a check bug, not an RST failure. b2c
  checks through `$SLOPVAC_RST_CONVERTER`, keeps full stderr, and reruns the 18 eval `.rst`
  files and `sample.rst`.
- recheck 4.6.2 ran over every pattern in b2b (b2's run failed on an ESM import, fixed).
- b2c (head ec0af61eb3): RST parity with `SLOPVAC_RST_CONVERTER=$HOME/tmp/rst-venv-344/bin/python`
  (docutils 0.21.2). The 18 eval `.rst` files and the golden `sample.rst` each exited 1 in both
  locales, with no stderr and no docutils message. That is the findings gate, so the RST
  inputs are fully linted. The split counts are unchanged by these runs.

## 1. Split counts

As of b2b (`results/b2b/split.md`, `gaps.jsonl`). Each distinct compiled pattern is counted
once, keyed by PCRE2 source and flags. The Rust twin takes the pre-`compat()` source with the
same caseless flag. "Identical" means byte-equal `find_all` spans (Python finditer empty-match
semantics) on every engine call of every corpus run, the 13-string edge suite and both fuzzes.

| Source | Patterns | compiles + identical | compiles + differs | doesn't compile |
|---|---|---|---|---|
| core rules: `pattern` | 111 | 80 | 0 | 31 (lookaround) |
| core rules: `substitution` | 18 | 14 | 0 | 4 (lookaround) |
| core rules: `tokens` | 17 | 17 | 0 | 0 |
| engine-built (`matching.rs` exception probes, `exceptions.rs` quotation span) | 743 | 726 | 0 | 17 (lookaround) |
| other engine/CLI built-ins (LazyLock, caller not recorded) | 13 | 12 | 1 | 0 |
| `slopvac-context` (genre/context, compiled without UCP) | 17 | 15 | 1 | 1 (lookaround) |
| `slopvac-text` built-ins (tokens, classify, measure, sentences) | 38 | 17 | 0 | 21 (lookaround; `measure.rs:43` also a backreference) |
| **total** | **957** | **881** | **2** | **74** |

Each locale run (en-US, en-GB) compiled the same number of distinct patterns per corpus. No
PCRE2 limit fired in any parity run (`limit_hits=0` in every dump).

Differences found (both by fuzz, none by corpus):

1. `^(?:true|yes)$` (a CLI/config built-in, LazyLock) on `"YES\n"`: PCRE2 `[0..3]`, Rust none.
   A real engine gap: PCRE2 `$` without `(?m)` also matches before a final `\n`, Rust `$` is
   `\z`.
2. `slopvac-context` `builtin/sentence.rs:55` (negated literal, caseless, no UCP) on
   `"never\r\t..//ſ-0"`: PCRE2 `[0..11]`, Rust `[0..15]`. This is a spike-translator gap, not
   an engine gap. The twin spelt non-UCP `\w` inside a class as `0-9A-Za-z_` without `(?-i)`,
   and Rust's caseless `s` then folds U+017F LONG S. The production translator must emit
   `(?-i:[...])` for ASCII classes under caseless.

Gap scan (static, `tools/gap_scan.py`, over all 957). 29 patterns use a construct whose meaning
differs between the engines: 13 identical, 1 differs, 15 don't compile.

- Non-multiline `$`: 22 (8 identical, 1 differs, 13 don't compile). The 8 identical ones are
  masked by construction: `\W*$`, `[^/]*$` (the class takes the `\n`), or `(?:\b|$)`.
- `(?m)^`: 7 (5 identical, 2 don't compile). Rust also matches `(?m)^` after a final `\n`, which
  only shows as an extra empty match. All 5 identical ones need a character after `^`.
- `\Z` and POSIX classes: none in the corpus.
- The split must not depend on masking. Rule: a pattern with a non-multiline `$` or a
  `(?m)^` that can match empty stays on PCRE2. Otherwise the translator rewrites the core
  data to `\z` / `(?m)$`, and the CI check rejects bare `$` in core patterns.

PCRE2-only patterns (74) with reasons: `results/b2b/split.md`, section "PCRE2-only". All are
lookaround except `measure.rs:43` (inline code span, `(?P=ticks)` backreference). Lookaround
uses fall into four shapes: word-edge guards `(?<![\w-])` / `(?![\w-])` (the twin's
consumed-context rewrite only handles a whole-pattern guard), exclusion lookaheads
`(?!(?:a|an|the)\b)`, sentence-start lookbehinds `(?<=[.!?]\s)`, and the
`title-case-heading` lookahead stack.

Interim verdict: 881 of 957 distinct patterns (92 %) can move to `regex`. For the shipped rule
kinds, that is 111 of 146: `pattern` 80/111, `substitution` 14/18, `tokens` 17/17.

## 2. Shape check

Prototype: `tools/shape_check.py`. It is a hand-written recursive-descent parser for the PCRE2
syntax slopvac patterns use: groups, all lookaround, atomic, named, branch reset, inline flags,
classes with `\p{..}` and POSIX, `\Q..\E`, backreferences, recursion and subroutines, `\K`, and
greedy, lazy and possessive quantifiers. Rules run over the tree, with character sets
evaluated on a finite alphabet (representative characters plus the pattern's literals):

- NEST: an unbounded backtracking quantifier inside another, with no mandatory sibling on the
  path whose characters are disjoint from the inner one (exponential, `(a+)+`).
- NEST-COUNTED: the same under a counted repeat (polynomial, `(.*a){12}`).
- ALT-OVERLAP: a repeated alternation with overlapping branches (`(a|aa)*`, `(\w|\d)+`).
- NULLABLE-REP: an unbounded repeat of a group that can match empty.
- LOOK-IN-REP, BACKREF-IN-REP, K-IN-REP: lookaround with an unbounded quantifier, a
  backreference or `\K` inside a repeated group.
- RECURSION: `(?R)`, `(?1)`, `\g<1>`.
- POLY: 3 or more adjacent overlapping unbounded quantifiers. Reject.
- POLY2 (2 of them) and LONG-LOOKBEHIND: warn only.
- Atomic groups and possessive quantifiers reset the analysis (no backtracking across them).

b2b run (before the parser fixes in 76b673320d): 7 of 957 core/built-in patterns flagged, 16 of
22 malicious plugin patterns flagged, 1 self-test miss (`(?<=a{0,200})(?<!b{1,200})x` is only
a warning). Three of the core flags were prototype false positives, now fixed:
`docs-discipline.status-language` (`(?-i:...)` read as recursion) and the two `matching.rs`
lookaround probes `(?:[^()]|\([^()]*\))*` (separated by mandatory parentheses). The b2c rerun
gives the final FP/FN numbers, and the measured malicious runs decide which flags are real.

b2c run (`results/b2c/stats.md`, fixed parser; self-test 25 of 26 as expected):

| Source | Patterns | reject | warn (POLY2 / long lookbehind) | ok | parse error |
|---|---|---|---|---|---|
| core rules: `pattern` | 111 | 3 | 10 | 98 | 0 |
| core rules: `substitution` | 18 | 0 | 0 | 18 | 0 |
| core rules: `tokens` | 17 | 0 | 0 | 17 | 0 |
| engine-built probes | 743 | 0 | 0 | 743 | 0 |
| other built-ins | 13 | 0 | 0 | 13 | 0 |
| `slopvac-context` | 17 | 0 | 2 | 15 | 0 |
| `slopvac-text` built-ins | 38 | 1 | 5 | 32 | 0 |
| malicious plugin set | 22 | 16 | 1 | 5 | 0 |

- Core rejects (4 of 957, 0.4 %): `ai-tells-formatting.title-case-heading` (POLY, NEST),
  `ai-tells-structure.tricolon-abuse-core` (POLY), `ste-procedural.condition-after-command`
  (POLY) and `slopvac-text/src/tokens.rs:76` (units, POLY). Each is a false positive unless the
  timing stages show a blow-up.
- Plugin set: all 12 exponential shapes and the recursion shape are rejected. Of the 7
  polynomial shapes, 3 are rejected (`(?=(a+))+b`, `(.*a){12}`, `\w*\w*\w*!`). The prototype
  passes `(?:(?<=a)a)+$`, `a*\K` and `\s+\S+\s+$` and only warns on
  `(?<=a{0,200})(?<!b{1,200})x`. The two safe shapes `(?>a+)+b` and `(a++)+b` pass. The timing
  stages show whether the 4 polynomial shapes that weren't rejected are false negatives under
  the interpreter limits.
- recheck 4.6.2 (JS translation of each pattern, 10 s per pattern) over the 957: safe 879, linear
  41, polynomial 26 (25 of degree 2, 1 of degree 3:
  `ai-tells-structure.rhetorical-question-transition`), unknown 11 (4 timeouts, 7 parse
  failures of the JS translation). Zero of recheck's 26 overlap the shape check's 4 rejects. 8
  of the 26 get a POLY2 warning. The degree-2 cases are the quadratic cost of an unanchored
  search re-tried at every start position. The shape check doesn't model that, by design,
  because the interpreter's match limit bounds each attempt. The timing stages run recheck's
  attack strings at native length, 64 KiB and 1 MiB to measure each one.

(final FP/FN numbers pending the timing stages)

## 3. End-to-end timing

(pending)

## 4. Proposed contract text

(pending)

## 5. Implementation beads

(pending)
