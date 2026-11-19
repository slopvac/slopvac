#!/bin/sh
# SPIKE batch 1 (runs on dev under slopvac-heavy, cwd = detached dev worktree):
# build the instrumented binary in a scratch COPY, fetch the docs corpus, then run the
# parity mode over golden inputs, the eval corpus, the Kubernetes docs and slopvac docs.
set -u
WT=$PWD
SHA8=$(git rev-parse --short=8 HEAD)
OUT=$HOME/personal/dev/slopvac/.git/runs/spike-out-$SHA8
B=$HOME/tmp/spike-regex-build/tree-$SHA8
T=$HOME/tmp/spike-regex-build/target
C=$HOME/tmp/spike-regex-build/corpora
mkdir -p "$OUT" "$B" "$T" "$C"
echo "toolchain=$(rustc --version) cargo=$(cargo --version)" | tee "$OUT/toolchain.txt"
uname -a > "$OUT/host.txt"; nproc >> "$OUT/host.txt"

rsync -a --delete --exclude target --exclude .git "$WT/" "$B/"
python3 "$WT/spike/regex-split/tools/apply.py" "$B" || exit 10
(cd "$B" && CARGO_TARGET_DIR=$T cargo build --release -p slopvac --bin slopvac) > "$OUT/build.log" 2>&1
echo "build=$?"
BIN=$T/release/slopvac
[ -x "$BIN" ] || { tail -40 "$OUT/build.log"; exit 11; }
cp "$BIN" "$OUT/slopvac-spike"
"$BIN" lint --help > "$OUT/lint-help.txt" 2>&1

# Kubernetes website, content/en (CC BY 4.0). Shallow sparse clone, commit recorded.
if [ ! -d "$C/k8s-website/.git" ]; then
  git clone -q --depth 1 --filter=blob:none --sparse https://github.com/kubernetes/website "$C/k8s-website" &&
    git -C "$C/k8s-website" sparse-checkout set content/en
fi
git -C "$C/k8s-website" rev-parse HEAD > "$OUT/k8s-commit.txt"
find "$C/k8s-website/content/en" -type f -name '*.md' | wc -l > "$OUT/k8s-files.txt"
du -sb "$C/k8s-website/content/en" >> "$OUT/k8s-files.txt"

# Neutral project: strict profile so every rule tier runs.
P=$HOME/tmp/spike-regex-build/project
mkdir -p "$P"
printf 'schema_version = 3\nprofile = "strict"\n' > "$P/slopvac.toml"
cp -R "$WT/tests/golden/v3/inputs" "$P/golden"
rm -rf "$P/eval" "$P/docs" "$P/k8s"
cp -R "$WT/evals/corpus" "$P/eval"
cp -R "$WT/docs" "$P/docs"
cp -R "$C/k8s-website/content/en" "$P/k8s"

M="python3 $WT/spike/regex-split/tools/measure.py"
cd "$P"
for set in golden eval docs k8s; do
  for loc in en-US en-GB; do
    rm -f "$OUT/parity-$set-$loc.jsonl"
    SLOPVAC_SPIKE_ENGINE=parity SLOPVAC_SPIKE_DUMP="$OUT/parity-$set-$loc.jsonl" \
      $M "$OUT/parity-runs.jsonl" "parity-$set-$loc" 3600 16384 -- \
      "$BIN" lint "$set" --format json --locale "$loc"
  done
done
# Compile-only dump with every engine-built pattern (rust mode records compile errors).
echo done-batch1
