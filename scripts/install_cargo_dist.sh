#!/usr/bin/env bash
# Install a pinned cargo-dist release binary and put it on PATH.
#
# The release workflow used to pipe cargo-dist's installer script into `sh`. This
# script installs the release archive for the runner instead, checked against a sha256
# committed here (see install_release_archive.sh).
#
# The digests were computed from the upstream v0.34.0 assets. Each matches the
# `.sha256` file and `sha256.sum` that the release publishes, and each asset passed
# `gh attestation verify -R axodotdev/cargo-dist`.
#
# To upgrade: change VERSION and every digest together, then verify the new assets.
set -euo pipefail

# shellcheck source=scripts/install_release_archive.sh
source "$(dirname "${BASH_SOURCE[0]}")/install_release_archive.sh"

VERSION=0.34.0
target="$(release_target)"
case "$target" in
aarch64-apple-darwin) sha256=d1eec30e63eed23e5a5bd48fe96eae314799811bffca254ec1d2264638ba1890 ;;
x86_64-apple-darwin) sha256=6aa89bb1e7c18f51ae8b148ce2fc6ab00ac46ed8173d4f2ed9afa12101218773 ;;
aarch64-unknown-linux-gnu) sha256=58efb33edf01342875bfd0c3ce2befa9d73075579ca1b3e5dd75b98c88f92e83 ;;
x86_64-unknown-linux-gnu) sha256=7fee44ff3ff71ed0064b32d4ec62fee6b58bd37c5887b12a86aedddc5af0c7b5 ;;
*)
	echo "no pinned cargo-dist digest for ${target}" >&2
	exit 1
	;;
esac

name="cargo-dist-${target}"
install_release_archive dist \
	"https://github.com/axodotdev/cargo-dist/releases/download/v${VERSION}/${name}.tar.xz" \
	"$sha256" "${name}/dist"
"${CARGO_HOME:-$HOME/.cargo}/bin/dist" --version
