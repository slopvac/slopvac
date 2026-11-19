#!/bin/sh
# SPIKE batch 2 (dev, under slopvac-heavy): no timings.
# 1. Build the instrumented slopvac, the bench and the uninstrumented baseline.
# 2. Eval corpus parity again: its files end in `.corpus`, which b1's lint skipped, so this
#    copies them to their real extension first.
# 3. Summarise corpus dumps -> patterns.jsonl; differential fuzz in parity mode (3000
#    strings per pattern); summarise again with the fuzz dump.
# 4. Shape check, recheck, malicious inputs.
# Usage: sh spike/regex-split/tools/batch2.sh <b1-out-dir>
set -u
B1=$1
WT=$PWD
SHA8=$(git rev-parse --short=8 HEAD)
OUT=$HOME/personal/dev/slopvac/.git/runs/spike-out-$SHA8
S=$HOME/tmp/spike-regex-build
B=$S/tree-$SHA8
T=$S/target
P=$S/project
mkdir -p "$OUT" "$B"
echo "toolchain=$(rustc --version)" | tee "$OUT/toolchain.txt"
echo "host=$(hostname) $(uname -m) nproc=$(nproc) $(uptime)" | tee "$OUT/host.txt"
TOOLS=$WT/spike/regex-split/tools

rsync -a --delete --exclude target --exclude .git "$WT/" "$B/"
python3 "$TOOLS/apply.py" "$B" || exit 10
(cd "$B" && CARGO_TARGET_DIR=$T cargo build --release -p slopvac --bin slopvac) > "$OUT/build.log" 2>&1
echo build-slopvac=$?
cp "$T/release/slopvac" "$S/slopvac-spike-$SHA8" || exit 11
cp "$B/Cargo.lock" "$B/spike/regex-split/bench/Cargo.lock"
(cd "$B/spike/regex-split/bench" && CARGO_TARGET_DIR=$T cargo build --release) > "$OUT/bench-build.log" 2>&1
echo build-bench=$?
cp "$T/release/spike-regex-bench" "$S/bench-$SHA8" || { tail -30 "$OUT/bench-build.log"; exit 12; }
CARGO_TARGET_DIR=$S/target-main cargo build --release -p slopvac --bin slopvac > "$OUT/build-main.log" 2>&1
echo build-main=$?
cp "$S/target-main/release/slopvac" "$S/slopvac-main-$SHA8"
BIN=$S/slopvac-spike-$SHA8

# Eval corpus with real extensions: <name>-<ext>.<hash>.corpus -> <name>-<ext>.<hash>.<ext>.
rm -rf "$P/eval"
python3 - "$WT/evals/corpus" "$P/eval" > "$OUT/eval-ext.txt" <<'EOF'
import collections, pathlib, shutil, sys
src, dst = map(pathlib.Path, sys.argv[1:])
known = {"md", "mdx", "rst", "txt", "py", "js", "ts", "tsx", "jsx", "rs", "go", "html", "htm",
         "adoc", "yaml", "yml", "toml", "sh", "java", "c", "h", "cpp", "rb", "mermaid", "mmd",
         "puml", "dot", "d2", "json", "svg", "kt", "swift", "cs", "php", "tex"}
c = collections.Counter()
for f in sorted(src.rglob("*.corpus")):
    stem = f.name[: -len(".corpus")]
    base = stem.rsplit(".", 1)[0]
    ext = base.rsplit("-", 1)[-1].lower()
    if ext not in known:
        ext = "md"
    c[ext] += 1
    out = dst / f.relative_to(src).parent / f"{stem}.{ext}"
    out.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(f, out)
for k, v in c.most_common():
    print(k, v)
EOF
cat "$OUT/eval-ext.txt"
M="python3 $TOOLS/measure.py"
cd "$P"
for loc in en-US en-GB; do
  rm -f "$OUT/parity-eval-$loc.jsonl"
  SLOPVAC_SPIKE_ENGINE=parity SLOPVAC_SPIKE_DUMP="$OUT/parity-eval-$loc.jsonl" \
    $M "$OUT/parity-runs.jsonl" "parity-eval-$loc" 3600 16384 -- "$BIN" lint eval --format json --locale "$loc"
done
cd "$WT"

DUMPS="$B1/parity-golden-en-US.jsonl $B1/parity-golden-en-GB.jsonl $B1/parity-docs-en-US.jsonl $B1/parity-docs-en-GB.jsonl $B1/parity-k8s-en-US.jsonl $B1/parity-k8s-en-GB.jsonl $OUT/parity-eval-en-US.jsonl $OUT/parity-eval-en-GB.jsonl"
python3 "$TOOLS/summarize.py" "$WT/crates/slopvac-rules/tests/fixtures/core-v3" "$OUT/split-corpus.md" $DUMPS
echo summarize1=$?
cp "$OUT/patterns.jsonl" "$OUT/patterns-corpus.jsonl"
rm -f "$OUT/parity-fuzz.jsonl" "$OUT/fuzz.jsonl"
SLOPVAC_SPIKE_ENGINE=parity SLOPVAC_SPIKE_DUMP="$OUT/parity-fuzz.jsonl" timeout 3600 \
  "$S/bench-$SHA8" "$OUT/patterns-corpus.jsonl" /dev/null fuzz "$OUT/fuzz.jsonl" --fuzz 3000 --timeout 300
echo fuzz=$?
python3 "$TOOLS/summarize.py" "$WT/crates/slopvac-rules/tests/fixtures/core-v3" "$OUT/split.md" $DUMPS "$OUT/parity-fuzz.jsonl"
echo summarize2=$?

python3 "$TOOLS/gen_inputs.py" "$WT" "$S/data"
echo gen=$?
cp "$S/data/plugin-patterns.jsonl" "$OUT/"
cat "$OUT/patterns.jsonl" "$S/data/plugin-patterns.jsonl" > "$OUT/all-patterns.jsonl"
python3 "$TOOLS/shape_check.py" "$OUT/all-patterns.jsonl" --json > "$OUT/shape.jsonl"
echo shape=$?
(cd "$TOOLS" && python3 shape_selftest.py) > "$OUT/shape-selftest.txt" 2>&1; echo selftest=$?

mkdir -p "$S/recheck" && cd "$S/recheck" &&
  { [ -d node_modules/recheck ] || npm install --silent --no-audit --no-fund recheck@4.6.2 > npm.log 2>&1; } &&
  node -e 'console.log(require("recheck/package.json").version)' > "$OUT/recheck-version.txt" 2>&1
cp "$TOOLS/recheck_run.mjs" "$S/recheck/"
timeout 5400 node "$S/recheck/recheck_run.mjs" "$OUT/patterns.jsonl" "$S/data" > "$OUT/recheck.log" 2>&1
echo recheck=$?
cp "$S/data/recheck.jsonl" "$OUT/" 2>/dev/null
cd "$WT"
ls -la "$S"/*-$SHA8 "$S/data" | head -40
echo done-batch2
