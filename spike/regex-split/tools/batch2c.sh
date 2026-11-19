#!/bin/sh
# SPIKE batch 2c (no timings): shape check from this head; RST parity reruns (the 18 eval
# .rst files and the golden sample.rst) with the helper's SLOPVAC_RST_CONVERTER venv; final
# summary; recheck and shape stats.
# Usage: sh spike/regex-split/tools/batch2c.sh <b1-out> <b2-out> <b2b-out>
set -u
B1=$1 B2=$2 B2B=$3
WT=$PWD
SHA8=$(git rev-parse --short=8 HEAD)
OUT=$HOME/personal/dev/slopvac/.git/runs/spike-out-$SHA8
S=$HOME/tmp/spike-regex-build
P=$S/project
TOOLS=$WT/spike/regex-split/tools
BIN=$S/target/release/slopvac
mkdir -p "$OUT"
echo "toolchain=$(rustc --version)" | tee "$OUT/toolchain.txt"
echo "host=$(hostname) $(uname -m) nproc=$(nproc) $(uptime)" | tee "$OUT/host.txt"
echo "SLOPVAC_RST_CONVERTER=${SLOPVAC_RST_CONVERTER:-unset}" | tee "$OUT/docutils.txt"
"${SLOPVAC_RST_CONVERTER:-false}" -c 'import docutils; print("docutils", docutils.__version__)' 2>&1 | tee -a "$OUT/docutils.txt"
# The spike binary built by b2b (same overlay; nothing in it changed since).
[ -x "$BIN" ] || { echo "no spike binary"; exit 11; }

rm -rf "$P/eval-rst" "$P/golden-rst"
mkdir -p "$P/eval-rst" "$P/golden-rst"
find "$P/eval" -name '*.rst' -exec cp {} "$P/eval-rst/" \;
ls "$P/eval-rst" | wc -l | tee "$OUT/eval-rst-count.txt"
cp "$WT/tests/golden/v3/inputs/formats/sample.rst" "$P/golden-rst/"
cd "$P"
for set in eval-rst golden-rst; do
  for loc in en-US en-GB; do
    rm -f "$OUT/parity-$set-$loc.jsonl"
    SLOPVAC_SPIKE_ENGINE=parity SLOPVAC_SPIKE_DUMP="$OUT/parity-$set-$loc.jsonl" timeout 600 \
      "$BIN" lint "$set" --format json --locale "$loc" > "$OUT/report-$set-$loc.json" 2> "$OUT/stderr-$set-$loc.txt"
    rc=$?
    echo "$set $loc rc=$rc stderr_lines=$(wc -l < "$OUT/stderr-$set-$loc.txt") docutils_mentions=$(grep -ci docutils "$OUT/stderr-$set-$loc.txt")" | tee -a "$OUT/rst-runs.txt"
  done
done
cd "$WT"

DUMPS="$B1/parity-golden-en-US.jsonl $B1/parity-golden-en-GB.jsonl $B1/parity-docs-en-US.jsonl $B1/parity-docs-en-GB.jsonl $B1/parity-k8s-en-US.jsonl $B1/parity-k8s-en-GB.jsonl $B2B/parity-eval-en-US.jsonl $B2B/parity-eval-en-GB.jsonl $OUT/parity-eval-rst-en-US.jsonl $OUT/parity-eval-rst-en-GB.jsonl $OUT/parity-golden-rst-en-US.jsonl $OUT/parity-golden-rst-en-GB.jsonl"
python3 "$TOOLS/summarize.py" "$WT/crates/slopvac-rules/tests/fixtures/core-v3" "$OUT/split.md" $DUMPS "$B2/parity-fuzz.jsonl" "$B2B/parity-fuzz-directed.jsonl"
echo summarize=$?
python3 "$TOOLS/gap_scan.py" "$OUT/patterns.jsonl" > "$OUT/gaps.jsonl"; echo gaps=$?
cat "$OUT/patterns.jsonl" "$B2/plugin-patterns.jsonl" > "$OUT/all-patterns.jsonl"
python3 "$TOOLS/shape_check.py" "$OUT/all-patterns.jsonl" --json > "$OUT/shape.jsonl"; echo shape=$?
(cd "$TOOLS" && python3 shape_selftest.py) > "$OUT/shape-selftest.txt" 2>&1; echo selftest=$?
python3 "$TOOLS/stats.py" "$OUT/patterns.jsonl" "$B2/plugin-patterns.jsonl" "$OUT/shape.jsonl" "$B2B/recheck.jsonl" > "$OUT/stats.md"
echo stats=$?
echo done-batch2c
