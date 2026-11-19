#!/bin/sh
# SPIKE batch 2 (dev, under slopvac-heavy): no timings. Summarise the b1 parity dumps,
# run the shape check and recheck, generate inputs, build the bench.
# Usage: sh spike/regex-split/tools/batch2.sh <b1-out-dir>
set -u
B1=$1
WT=$PWD
SHA8=$(git rev-parse --short=8 HEAD)
OUT=$HOME/personal/dev/slopvac/.git/runs/spike-out-$SHA8
S=$HOME/tmp/spike-regex-build
B=$S/tree-$SHA8
T=$S/target
mkdir -p "$OUT" "$B"
echo "toolchain=$(rustc --version)" | tee "$OUT/toolchain.txt"
TOOLS=$WT/spike/regex-split/tools

python3 "$TOOLS/summarize.py" "$WT/crates/slopvac-rules/tests/fixtures/core-v3" "$OUT/split.md" "$B1"/parity-*.jsonl
echo summarize=$?
python3 "$TOOLS/gen_inputs.py" "$WT" "$S/data"
echo gen=$?
cp "$S/data/plugin-patterns.jsonl" "$OUT/"
cat "$OUT/patterns.jsonl" "$S/data/plugin-patterns.jsonl" > "$OUT/all-patterns.jsonl"
python3 "$TOOLS/shape_check.py" "$OUT/all-patterns.jsonl" --json > "$OUT/shape.jsonl"
echo shape=$?
python3 "$TOOLS/shape_selftest.py" > "$OUT/shape-selftest.txt" 2>&1; echo selftest=$?

mkdir -p "$S/recheck" && cd "$S/recheck" &&
  { [ -d node_modules/recheck ] || npm install --silent --no-audit --no-fund recheck@4.6.2 > npm.log 2>&1; } &&
  node -e 'console.log(require("recheck/package.json").version)' > "$OUT/recheck-version.txt" 2>&1
cp "$TOOLS/recheck_run.mjs" "$S/recheck/"
timeout 3000 node "$S/recheck/recheck_run.mjs" "$OUT/patterns.jsonl" "$S/data" > "$OUT/recheck.log" 2>&1
echo recheck=$?
cp "$S/data/recheck.jsonl" "$OUT/" 2>/dev/null
cd "$WT"

rsync -a --delete --exclude target --exclude .git "$WT/" "$B/"
python3 "$TOOLS/apply.py" "$B" || exit 10
cp "$B/Cargo.lock" "$B/spike/regex-split/bench/Cargo.lock"
(cd "$B" && CARGO_TARGET_DIR=$T cargo build --release -p slopvac --bin slopvac) > "$OUT/build.log" 2>&1
echo build-slopvac=$?
cp "$T/release/slopvac" "$S/slopvac-spike-$SHA8"
(cd "$B/spike/regex-split/bench" && CARGO_TARGET_DIR=$T cargo build --release) > "$OUT/bench-build.log" 2>&1
echo build-bench=$?
cp "$T/release/spike-regex-bench" "$S/bench-$SHA8"
# Uninstrumented baseline binary from the dev worktree itself (shipped code).
CARGO_TARGET_DIR=$S/target-main cargo build --release -p slopvac --bin slopvac > "$OUT/build-main.log" 2>&1
echo build-main=$?
cp "$S/target-main/release/slopvac" "$S/slopvac-main-$SHA8"
ls -la "$S"/*-$SHA8 "$S/data" | head -40
