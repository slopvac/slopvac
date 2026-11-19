#!/bin/sh
# SPIKE batch 2b (dev, under slopvac-heavy): no timings.
# Rebuild the bench (directed fuzz); parity again for the RST inputs, now that the helper puts
# docutils on PATH (golden sample.rst and the eval corpus, whose 18 .rst files b1/b2 skipped);
# directed differential fuzz; summary over every dump; gap scan; shape check; recheck.
# Usage: sh spike/regex-split/tools/batch2b.sh <b1-out-dir> <b2-out-dir>
set -u
B1=$1
B2=$2
WT=$PWD
SHA8=$(git rev-parse --short=8 HEAD)
OUT=$HOME/personal/dev/slopvac/.git/runs/spike-out-$SHA8
S=$HOME/tmp/spike-regex-build
B=$S/tree-$SHA8
T=$S/target
P=$S/project
TOOLS=$WT/spike/regex-split/tools
mkdir -p "$OUT" "$B"
echo "toolchain=$(rustc --version)" | tee "$OUT/toolchain.txt"
echo "host=$(hostname) $(uname -m) nproc=$(nproc) $(uptime)" | tee "$OUT/host.txt"
python3 -c 'import docutils; print("docutils", docutils.__version__)' 2>&1 | tee "$OUT/docutils.txt"

rsync -a --delete --exclude target --exclude .git "$WT/" "$B/"
python3 "$TOOLS/apply.py" "$B" || exit 10
(cd "$B" && CARGO_TARGET_DIR=$T cargo build --release -p slopvac --bin slopvac) > "$OUT/build.log" 2>&1
echo build-slopvac=$?
cp "$B/Cargo.lock" "$B/spike/regex-split/bench/Cargo.lock"
(cd "$B/spike/regex-split/bench" && CARGO_TARGET_DIR=$T cargo build --release) > "$OUT/bench-build.log" 2>&1
echo build-bench=$?
BIN=$T/release/slopvac
BENCH=$T/release/spike-regex-bench
[ -x "$BENCH" ] || { tail -30 "$OUT/bench-build.log"; exit 12; }

rm -rf "$P/eval" "$P/golden-rst"
python3 "$TOOLS/eval_copy.py" "$WT/evals/corpus" "$P/eval" > "$OUT/eval-ext.txt"
mkdir -p "$P/golden-rst" && cp "$WT/tests/golden/v3/inputs/formats/sample.rst" "$P/golden-rst/"
M="python3 $TOOLS/measure.py"
cd "$P"
for set in eval golden-rst; do
  for loc in en-US en-GB; do
    rm -f "$OUT/parity-$set-$loc.jsonl"
    SLOPVAC_SPIKE_ENGINE=parity SLOPVAC_SPIKE_DUMP="$OUT/parity-$set-$loc.jsonl" \
      $M "$OUT/parity-runs.jsonl" "parity-$set-$loc" 3600 16384 -- "$BIN" lint "$set" --format json --locale "$loc"
  done
done
cd "$WT"

python3 "$TOOLS/gen_match.py" "$B2/patterns.jsonl" "$OUT/directed.jsonl" 150
echo gen-match=$?
rm -f "$OUT/parity-fuzz-directed.jsonl" "$OUT/fuzz-directed.jsonl"
SLOPVAC_SPIKE_ENGINE=parity SLOPVAC_SPIKE_DUMP="$OUT/parity-fuzz-directed.jsonl" timeout 3600 \
  "$BENCH" "$B2/patterns.jsonl" /dev/null fuzz-directed "$OUT/fuzz-directed.jsonl" \
  --fuzz 3000 --fuzz-file "$OUT/directed.jsonl" --timeout 300
echo fuzz=$?

DUMPS="$B1/parity-golden-en-US.jsonl $B1/parity-golden-en-GB.jsonl $B1/parity-docs-en-US.jsonl $B1/parity-docs-en-GB.jsonl $B1/parity-k8s-en-US.jsonl $B1/parity-k8s-en-GB.jsonl $OUT/parity-eval-en-US.jsonl $OUT/parity-eval-en-GB.jsonl $OUT/parity-golden-rst-en-US.jsonl $OUT/parity-golden-rst-en-GB.jsonl"
python3 "$TOOLS/summarize.py" "$WT/crates/slopvac-rules/tests/fixtures/core-v3" "$OUT/split.md" $DUMPS "$B2/parity-fuzz.jsonl" "$OUT/parity-fuzz-directed.jsonl"
echo summarize=$?
python3 "$TOOLS/gap_scan.py" "$OUT/patterns.jsonl" > "$OUT/gaps.jsonl"; echo gaps=$?
cat "$OUT/patterns.jsonl" "$B2/plugin-patterns.jsonl" > "$OUT/all-patterns.jsonl"
python3 "$TOOLS/shape_check.py" "$OUT/all-patterns.jsonl" --json > "$OUT/shape.jsonl"; echo shape=$?
(cd "$TOOLS" && python3 shape_selftest.py) > "$OUT/shape-selftest.txt" 2>&1; echo selftest=$?

mkdir -p "$S/recheck" && cd "$S/recheck" &&
  { [ -d node_modules/recheck ] || npm install --silent --no-audit --no-fund recheck@4.6.2 > npm.log 2>&1; }
node -e 'console.log(require("recheck/package.json").version)' > "$OUT/recheck-version.txt" 2>&1
cp "$TOOLS/recheck_run.mjs" "$S/recheck/"
timeout 5400 node "$S/recheck/recheck_run.mjs" "$OUT/patterns.jsonl" "$S/data" > "$OUT/recheck.log" 2>&1
echo recheck=$?
cp "$S/data/recheck.jsonl" "$OUT/" 2>/dev/null
echo done-batch2b
