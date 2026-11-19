#!/usr/bin/env bash
# Regenerate .github/workflows/release.yml and re-apply the tag-injection hardening.
#
# dist-workspace.toml marks the workflow `allow-dirty` because the hardening
# changes it, and dist refuses to generate a file it was told to leave alone.
# This script lifts that setting for one run, then restores the config.
#
# Options:
#   --save-raw PATH   keep dist's unpatched output at PATH (the patcher tests read it)
#   --update-pins     rewrite scripts/release_workflow_pins.json from the result. Run it
#                     only after you review the diff of release.yml.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

save_raw=""
update_pins=0
while [[ $# -gt 0 ]]; do
	case "$1" in
	--save-raw)
		save_raw="${2:?--save-raw needs a path}"
		shift 2
		;;
	--update-pins)
		update_pins=1
		shift
		;;
	*)
		echo "usage: dist_generate.sh [--save-raw PATH] [--update-pins]" >&2
		exit 1
		;;
	esac
done

# --save-raw overwrites its target, so refuse the two files this script owns, whether the
# path names them directly or through a symlink or a relative spelling.
if [[ -n "$save_raw" ]]; then
	resolved="$(python3 -c 'import os, sys; print(os.path.realpath(sys.argv[1]))' "$save_raw")"
	for owned in scripts/release_workflow_pins.json .github/workflows/release.yml; do
		if [[ "$resolved" == "$(python3 -c 'import os, sys; print(os.path.realpath(sys.argv[1]))' "$owned")" ]]; then
			echo "--save-raw must not point at $owned" >&2
			exit 1
		fi
	done
fi

config=dist-workspace.toml
backup="$(mktemp)"
cp "$config" "$backup"
trap 'cp "$backup" "$config"; rm -f "$backup"' EXIT

sed -i.bak '/^allow-dirty = /d' "$config" && rm -f "$config.bak"
rm -f .github/workflows/release.yml
dist generate --mode ci
if [[ -n "$save_raw" ]]; then
	# Keep dist's unpatched output: the patcher tests use it as their input.
	cp .github/workflows/release.yml "$save_raw"
fi
if [[ "$update_pins" == 1 ]]; then
	SLOPVAC_UPDATE_PINS=1 python3 scripts/patch_release_workflow.py
else
	# An ambient SLOPVAC_UPDATE_PINS must not switch a normal run into write mode.
	env -u SLOPVAC_UPDATE_PINS python3 scripts/patch_release_workflow.py
fi
