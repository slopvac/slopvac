#!/usr/bin/env bash
# Install a pinned cargo-cyclonedx release binary and put it on PATH.
#
# dist's generated workflow piped the upstream installer script into `sh`. This script
# installs the release archive for the runner instead, checked against a sha256 committed
# here (see install_release_archive.sh).
#
# The digests were computed from the upstream cargo-cyclonedx-0.5.5 assets. Each matches the
# `.sha256` file that the release publishes, and each asset passed
# `gh attestation verify -R CycloneDX/cyclonedx-rust-cargo`.
# The release has no aarch64 Linux build; the global job runs on x86_64.
#
# To upgrade: change VERSION and every digest together, then verify the new assets.
set -euo pipefail

# shellcheck source=scripts/install_release_archive.sh
source "$(dirname "${BASH_SOURCE[0]}")/install_release_archive.sh"

VERSION=0.5.5
target="$(release_target)"
case "$target" in
aarch64-apple-darwin) sha256=c296b0821c02ad13cbea6a7173ba6c37fa777acc7d066c5ff8570a600aeb62a7 ;;
x86_64-apple-darwin) sha256=98beb669323396428b5c6c7003adc4235207442c7b025743366ee98759b2c3a6 ;;
x86_64-unknown-linux-gnu) sha256=9136714bd6e1ac703f5c2c889af578d9783852886d6ebfe18d471555637ad2b3 ;;
*)
	echo "no pinned cargo-cyclonedx digest for ${target}" >&2
	exit 1
	;;
esac

name="cargo-cyclonedx-${target}"
install_release_archive cargo-cyclonedx \
	"https://github.com/CycloneDX/cyclonedx-rust-cargo/releases/download/cargo-cyclonedx-${VERSION}/${name}.tar.xz" \
	"$sha256" "${name}/cargo-cyclonedx"
