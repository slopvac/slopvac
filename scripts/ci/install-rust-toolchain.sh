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

channel="$(python3 -c '
import sys, tomllib

path = sys.argv[1]
try:
    with open(path, "rb") as handle:
        print(tomllib.load(handle)["toolchain"]["channel"])
except (OSError, tomllib.TOMLDecodeError, KeyError, TypeError) as error:
    sys.exit(f"cannot read the toolchain channel from {path}: {error!r}")
' "$file")"
if [[ -z "$channel" ]]; then
	echo "empty channel in $file" >&2
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
