#!/usr/bin/env bash
# Install a pinned cargo-delta release binary and put it on PATH.
#
# cargo-delta maps a pull request's changed files to the workspace crates they affect (see
# scripts/ci/rust_scope.py). This script installs one fixed release instead of the latest,
# checked against a sha256 committed here (see install_release_archive.sh).
#
# The digests are the asset digests that the GitHub API reports for the v0.5.0 release
# (https://github.com/tekian/cargo-delta/releases/tag/v0.5.0). The macOS arm64 and Linux x64
# assets were also downloaded and hashed by hand, and both hashes matched. The release has no
# GitHub attestations (`gh attestation verify -R tekian/cargo-delta` finds none), so the
# committed digest is the only check.
#
# To upgrade: change VERSION and every digest together, then verify the new assets.
set -euo pipefail

# shellcheck source=scripts/install_release_archive.sh
source "$(dirname "${BASH_SOURCE[0]}")/install_release_archive.sh"

VERSION=0.5.0
target="$(release_target)"
case "$target" in
aarch64-apple-darwin) sha256=67ca698cb6bf6cdac9f2d087182a3ce60012973e2b968d49f7081555cf510279 ;;
x86_64-apple-darwin) sha256=1fefc4536c55ca8bea45fd3c51eacf348a00e82b946b6d1eed1bb06a43f3c235 ;;
aarch64-unknown-linux-gnu) sha256=61e648999b954036ea64bc74baeaddb9ecd0641f800094248d540f182adb67ff ;;
x86_64-unknown-linux-gnu) sha256=d1d330bf209a494145bd19a62fe59d97767b593415e1c41d160307dad2dde75f ;;
*)
	echo "no pinned cargo-delta digest for ${target}" >&2
	exit 1
	;;
esac

install_release_archive cargo-delta \
	"https://github.com/tekian/cargo-delta/releases/download/v${VERSION}/cargo-delta-${target}.tar.gz" \
	"$sha256" cargo-delta
