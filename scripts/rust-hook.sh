#!/usr/bin/env bash
# Run a Rust tooling hook, or say how to get the tool when it is missing.
# Usage: rust-hook.sh <binary> <args...>   (cargo subcommands: cargo-sort, cargo-shear, cargo-deny)
set -euo pipefail

tool="$1"
shift

if ! command -v "$tool" >/dev/null 2>&1; then
  printf 'slopvac: %s is not installed. Run `mise install` (versions are pinned in mise.toml).\n' "$tool" >&2
  exit 1
fi

if [[ "$tool" == cargo-* ]]; then
  exec cargo "${tool#cargo-}" "$@"
fi
exec "$tool" "$@"
