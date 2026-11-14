#!/usr/bin/env bash
# Regenerate .github/workflows/release.yml and re-apply the tag-injection hardening.
#
# dist-workspace.toml marks the workflow `allow-dirty` because the hardening
# changes it, and dist refuses to generate a file it was told to leave alone.
# This script lifts that setting for one run, then restores the config.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

config=dist-workspace.toml
backup="$(mktemp)"
cp "$config" "$backup"
trap 'cp "$backup" "$config"; rm -f "$backup"' EXIT

sed -i.bak '/^allow-dirty = /d' "$config" && rm -f "$config.bak"
rm -f .github/workflows/release.yml
dist generate --mode ci
if [[ "${1:-}" == "--save-raw" ]]; then
	# Keep dist's unpatched output: the patcher tests use it as their input.
	cp .github/workflows/release.yml "${2:?--save-raw needs a path}"
fi
python3 scripts/patch_release_workflow.py
