#!/usr/bin/env bash
# Install a pinned cargo-auditable release binary and put it on PATH.
#
# dist's generated workflow piped the installer script of the *latest* upstream release
# into `sh`. This script installs one fixed release instead, checked against a sha256
# committed here (see install_release_archive.sh).
#
# The digests were computed from the upstream v0.7.7 assets. Each matches the `.sha256`
# file that the release publishes. That release has no GitHub attestations
# (`gh attestation verify -R rust-secure-code/cargo-auditable` finds none), so the
# committed digest is the only check; compare any new digest with the upstream `.sha256`
# and a local download before you commit it.
#
# To upgrade: change VERSION and every digest together.
set -euo pipefail

# shellcheck source=scripts/install_release_archive.sh
source "$(dirname "${BASH_SOURCE[0]}")/install_release_archive.sh"

VERSION=0.7.7
target="$(release_target)"
case "$target" in
aarch64-apple-darwin) sha256=a66ad6392861ca1abfe0b02b522622d91edf63e5d7b60f4d3088b5c23d240c72 ;;
x86_64-apple-darwin) sha256=4b87f06ea13d32466bbc68b1f5715ac11753a052d9935245380bddce8a3f0e50 ;;
aarch64-unknown-linux-gnu) sha256=bcb2d46002a8dd8e94d0d669cb39d38c9651db2ae5d1f3f1f1bab516ee1e212c ;;
x86_64-unknown-linux-gnu) sha256=4c8d3b08c3c34e9f2333367224dbbfb4aab68067698b43de59fe19aded3416dd ;;
*)
	echo "no pinned cargo-auditable digest for ${target}" >&2
	exit 1
	;;
esac

name="cargo-auditable-${target}"
install_release_archive cargo-auditable \
	"https://github.com/rust-secure-code/cargo-auditable/releases/download/v${VERSION}/${name}.tar.xz" \
	"$sha256" "${name}/cargo-auditable"
