#!/bin/sh
# SPIKE batch 3 (timing host, under slopvac-heavy). Self-contained: reads the pattern lists
# committed in spike/regex-split/results/b2.
# Usage: sh spike/regex-split/tools/batch3.sh <stage> [modes]
#   prep  : build instrumented slopvac, bench, uninstrumented baseline; inputs; recheck
#   small : 1 MiB classes, nesting, near-miss, recheck and plugin attacks (all modes)
#   large : 8 MiB, then 64 MiB classes
#   e2e   : release `slopvac lint` on slopvac docs and the eval corpus, median of 3
# Timing stages run on dev only (arm64, 16 cores). They refuse to start at 1-minute load
# GATE (default 4) or above and mark the run DISCARD if the load at the end is not under GATE.
set -u
STAGE=$1
MODES=${2:-"interp jit callout rust"}
FULL=$STAGE
# `small-interp` = stage small, mode interp (one mode per stage keeps each stage short).
case $STAGE in *-*) MODES=${STAGE#*-}; STAGE=${STAGE%%-*} ;; esac
WT=$PWD
SHA8=$(git rev-parse --short=8 HEAD)
S=$HOME/tmp/spike-regex-build
D=$S/data
IN=$WT/spike/regex-split/results/b2c
PLUG=$WT/spike/regex-split/results/b2/plugin-patterns.jsonl
TOOLS=$WT/spike/regex-split/tools
OUT=$HOME/personal/dev/slopvac/.git/runs/spike-t-$SHA8-$FULL
BENCH=$S/bin/bench
if [ "$STAGE" != prep ] && [ -d "$OUT" ]; then mv "$OUT" "$OUT.prev-$(date +%s)"; fi
mkdir -p "$OUT" "$S/bin"
echo "toolchain=$(rustc --version)" > "$OUT/toolchain.txt"
echo "host=$(hostname) arch=$(uname -m) nproc=$(nproc)" > "$OUT/host.txt"
load1() { cut -d' ' -f1 /proc/loadavg; }
GATE=${GATE:-4}
over() { awk "BEGIN{exit !($1 >= $GATE)}"; }

if [ "$STAGE" = prep ]; then
  B=$S/tree-$SHA8 T=$S/target
  mkdir -p "$B"
  rsync -a --delete --exclude target --exclude .git "$WT/" "$B/"
  python3 "$TOOLS/apply.py" "$B" || exit 10
  (cd "$B" && CARGO_TARGET_DIR=$T cargo build --release -p slopvac --bin slopvac) > "$OUT/build.log" 2>&1; echo build-slopvac=$?
  cp "$T/release/slopvac" "$S/bin/slopvac-spike" || exit 11
  cp "$B/Cargo.lock" "$B/spike/regex-split/bench/Cargo.lock"
  (cd "$B/spike/regex-split/bench" && CARGO_TARGET_DIR=$T cargo build --release) > "$OUT/bench-build.log" 2>&1; echo build-bench=$?
  cp "$T/release/spike-regex-bench" "$S/bin/bench" || exit 12
  CARGO_TARGET_DIR=$S/target-main cargo build --release -p slopvac --bin slopvac > "$OUT/build-main.log" 2>&1; echo build-main=$?
  cp "$S/target-main/release/slopvac" "$S/bin/slopvac-main" || exit 13
  echo "$SHA8 $(rustc --version)" > "$S/bin/STAMP"
  python3 "$TOOLS/gen_inputs.py" "$WT" "$D"; echo gen=$?
  mkdir -p "$S/recheck" && cd "$S/recheck" &&
    { [ -d node_modules/recheck ] || npm install --silent --no-audit --no-fund recheck@4.6.2 > npm.log 2>&1; }
  node -e 'console.log(require("recheck/package.json").version)' > "$OUT/recheck-version.txt" 2>&1
  cp "$TOOLS/recheck_run.mjs" "$S/recheck/"
  timeout 5400 node "$S/recheck/recheck_run.mjs" "$IN/patterns.jsonl" "$D" > "$OUT/recheck.log" 2>&1; echo recheck=$?
  cp "$D/recheck.jsonl" "$OUT/" 2>/dev/null
  # Lint project: strict profile; slopvac docs and the eval corpus with real extensions.
  P=$S/project
  rm -rf "$P" && mkdir -p "$P"
  printf 'schema_version = 3\nprofile = "strict"\n' > "$P/slopvac.toml"
  cp -R "$WT/docs" "$P/docs"
  python3 "$TOOLS/eval_copy.py" "$WT/evals/corpus" "$P/eval" > "$OUT/eval-ext.txt"
  "${SLOPVAC_RST_CONVERTER:-false}" -c 'import docutils; print("docutils", docutils.__version__)' > "$OUT/docutils.txt" 2>&1
  echo done-prep
  exit 0
fi

echo "bin=$(cat "$S/bin/STAMP")" >> "$OUT/host.txt"
L0=$(load1)
echo "start: $(uptime) nproc=$(nproc)" | tee "$OUT/load.txt"
if over "$L0"; then
  echo "load-gate: load $L0 not under $GATE, nothing measured" | tee -a "$OUT/load.txt"
  exit 75
fi

# run <mode> <class-label> <patterns> <input> [extra bench args]
run() {
  mode=$1; cls=$2; pats=$3; input=$4; shift 4
  label="$mode/$cls"
  echo "$(date -u +%T) $label load=$(load1)" >> "$OUT/load.txt"
  while :; do
    ( ulimit -v 8388608; SLOPVAC_SPIKE_ENGINE=$mode SLOPVAC_SPIKE_ALLOW=$ALLOW \
      timeout 7200 "$BENCH" "$pats" "$input" "$label" "$OUT/bench.jsonl" "$@" ) \
      > /dev/null 2>> "$OUT/bench.err"
    rc=$?
    [ $rc = 0 ] && break
    [ $rc = 3 ] && continue
    if [ -f "$OUT/bench.jsonl.cur" ]; then
      cur=$(cut -f1 "$OUT/bench.jsonl.cur")
      origin=$(cut -f3 "$OUT/bench.jsonl.cur")
      printf '{"id":"%s","label":"%s","mode":"%s","origin":"%s","killed":"exit %s (killed at the 8192 MiB address-space cap, or outer timeout)"}\n' \
        "$cur" "$label" "$mode" "$origin" "$rc" >> "$OUT/bench.jsonl"
      rm -f "$OUT/bench.jsonl.cur"
    else
      echo "$label rc=$rc with no pattern in flight" >> "$OUT/bench.err"; break
    fi
  done
}

# Core: rule and built-in patterns; `rust` mode only for the parity-identical ones.
CORE=$S/patterns-timing.jsonl
# The ~740 `matching.rs` probes are anchored (`\A...\z`) and only ever run on a finding's
# span, so timing them over whole documents says nothing; they stay out of the timing set.
grep -v 'slopvac-engine/src/matching.rs' "$IN/patterns.jsonl" > "$CORE"
RUSTP=$S/patterns-identical.jsonl
# Split allowlist: parity-identical minus every pattern the gap scan names ($ / (?m)^).
cut -d'"' -f4 "$IN/gaps.jsonl" > "$S/gap-ids.txt"
grep -v -F -f "$S/gap-ids.txt" "$IN/allow.txt" > "$S/allow-split.txt"
ALLOW=$S/allow-split.txt
grep '"verdict": "identical"' "$CORE" | grep -v -F -f "$S/gap-ids.txt" > "$RUSTP"
wc -l "$IN/allow.txt" "$S/allow-split.txt" "$RUSTP" >> "$OUT/host.txt"
PLUG=${PLUG}
pats_for() { if [ "$1" = rust ]; then echo "$RUSTP"; else echo "$CORE"; fi; }
case $STAGE in
small)
  for mode in $MODES; do
    for c in block-1m sentence-1m nest-paren-1k nest-paren-10k nest-mixed-1k nest-mixed-10k \
      nest-list-1k nest-list-10k run-ws-1m run-dots-1m run-dash-1m run-a-1m run-word-1m \
      run-caps-1m run-quote-1m run-apos-1m nearmiss-1m; do
      run "$mode" "$c" "$(pats_for "$mode")" "$D/inputs/$c.txt"
    done
    for a in native 64k 1m; do
      run "$mode" "recheck-$a" "$(pats_for "$mode")" /dev/null --pattern-input "$D/attack-core-$a"
    done
    for a in 30 64k 1m; do
      run "$mode" "plugin-$a" "$PLUG" /dev/null --pattern-input "$D/attack-$a"
    done
  done ;;
large8)
  for c in block-8m sentence-8m nearmiss-8m; do
    for mode in $MODES; do
      run "$mode" "$c" "$(pats_for "$mode")" "$D/inputs/$c.txt" --reps 3
    done
  done ;;
large64)
  for c in block-64m sentence-64m; do
    for mode in $MODES; do
      run "$mode" "$c" "$(pats_for "$mode")" "$D/inputs/$c.txt" --reps 3
    done
  done ;;
e2e)
  P=$S/project
  M="python3 $TOOLS/measure.py"
  cd "$P"
  for set in docs eval; do
    for rep in 1 2 3; do
      $M "$OUT/e2e.jsonl" "main-interp/$set" 300 16384 -- "$S/bin/slopvac-main" lint "$set" --format json
      SLOPVAC_SPIKE_ENGINE=interp $M "$OUT/e2e.jsonl" "spike-interp/$set" 300 16384 -- "$S/bin/slopvac-spike" lint "$set" --format json
      SLOPVAC_SPIKE_ENGINE=split SLOPVAC_SPIKE_ALLOW=$ALLOW $M "$OUT/e2e.jsonl" "split/$set" 300 16384 -- "$S/bin/slopvac-spike" lint "$set" --format json
      SLOPVAC_SPIKE_ENGINE=jit $M "$OUT/e2e.jsonl" "jit/$set" 300 16384 -- "$S/bin/slopvac-spike" lint "$set" --format json
      SLOPVAC_SPIKE_ENGINE=callout $M "$OUT/e2e.jsonl" "callout/$set" 300 16384 -- "$S/bin/slopvac-spike" lint "$set" --format json
    done
  done
  for e in interp split jit callout; do
    SLOPVAC_SPIKE_ENGINE=$e SLOPVAC_SPIKE_ALLOW=$ALLOW timeout 300 "$S/bin/slopvac-spike" lint docs eval --format json > "$OUT/report-$e.json" 2>/dev/null
    echo "$e rc=$? sha=$(sha256sum < "$OUT/report-$e.json" | cut -c1-16)" >> "$OUT/reports.txt"
  done
  "$S/bin/slopvac-main" lint docs eval --format json > "$OUT/report-main.json" 2>/dev/null
  echo "main rc=$? sha=$(sha256sum < "$OUT/report-main.json" | cut -c1-16)" >> "$OUT/reports.txt" ;;
esac
L1=$(load1)
echo "end: $(uptime)" | tee -a "$OUT/load.txt"
if over "$L1"; then echo "load-gate: end load $L1 not under $GATE, DISCARD" | tee -a "$OUT/load.txt"; fi
echo done-$STAGE
