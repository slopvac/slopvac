#!/usr/bin/env bash
# Lint the guidance text that `slopvac prime write` and `slopvac prime review` render.
#
# Usage: gate_prime_prose.sh <slopvac-binary> <v3-config> <output-dir>
#
# The binary lints each rendered file with the 3.x ruleset. Exit 0 passes, exit 1 means a
# gate failed and exit 2 means a run was incomplete. The script exits with a failure for
# every nonzero lint exit, so an incomplete run never passes.
set -euo pipefail

bin=$(realpath "$1")
config=$(realpath "$2")
out=$3

rm -rf "$out"
mkdir -p "$out"
out=$(realpath "$out")
cp "$config" "$out/slopvac.toml"
cd "$out"

"$bin" prime write > write.md
"$bin" prime review > review-auto.md
for type in mechanical update rewrite; do
  "$bin" prime review --type "$type" > "review-$type.md"
done

"$bin" lint ./*.md
