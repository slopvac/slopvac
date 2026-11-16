#!/usr/bin/env bash
# Install the toolchain pinned in rust-toolchain.toml, recovering from a
# partial preinstalled copy.
#
# Some runner images ship a half-installed toolchain, and rustup then fails
# with "detected conflict" while adding components. One failed install triggers
# an uninstall of the pinned channel and a single clean retry.
#
# RUST_TOOLCHAIN_DIR overrides the directory that holds rust-toolchain.toml
# (default: the repository root).
set -euo pipefail

root="${RUST_TOOLCHAIN_DIR:-$(dirname "${BASH_SOURCE[0]}")/../..}"
file="$root/rust-toolchain.toml"

channel="$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$file" | head -n 1)"
if [[ -z "$channel" ]]; then
	echo "no channel found in $file" >&2
	exit 1
fi

# Run inside the repository so rustup reads components and profile from the file.
pushd "$root" >/dev/null
if ! rustup toolchain install; then
	echo "rustup toolchain install failed; reinstalling $channel from scratch" >&2
	rustup toolchain uninstall "$channel"
	rustup toolchain install
fi
rustup show active-toolchain
popd >/dev/null
