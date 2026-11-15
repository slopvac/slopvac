#!/usr/bin/env bash
# Install a pinned minisign release binary on a Linux runner and put it on PATH.
#
# The tarball is checked against a sha256 that is committed here, so a changed
# or substituted release asset fails the job. The pinned digest was computed
# from the upstream asset, after its own .minisig checked against the upstream
# minisign public key (published in the jedisct1/minisign README).
#
# To upgrade: change VERSION and SHA256 together, then verify the new asset.
set -euo pipefail

VERSION=0.12
SHA256=9a599b48ba6eb7b1e80f12f36b94ceca7c00b7a5173c95c3efc88d9822957e73
URL="https://github.com/jedisct1/minisign/releases/download/${VERSION}/minisign-${VERSION}-linux.tar.gz"

if [[ "$(uname -s)" != Linux ]]; then
	echo "install_minisign.sh supports Linux runners only" >&2
	exit 1
fi
case "$(uname -m)" in
x86_64) arch=x86_64 ;;
aarch64 | arm64) arch=aarch64 ;;
*)
	echo "unsupported architecture: $(uname -m)" >&2
	exit 1
	;;
esac

dest="${RUNNER_TEMP:?RUNNER_TEMP must be set}/minisign-${VERSION}"
mkdir -p "$dest"
curl --proto '=https' --tlsv1.2 -fsSL --retry 3 -o "$dest/minisign.tar.gz" "$URL"
echo "${SHA256}  ${dest}/minisign.tar.gz" | sha256sum --check --strict -
tar -xzf "$dest/minisign.tar.gz" -C "$dest" "minisign-linux/${arch}/minisign"
mkdir -p "$dest/bin"
install -m 0755 "$dest/minisign-linux/${arch}/minisign" "$dest/bin/minisign"
echo "$dest/bin" >> "${GITHUB_PATH:?GITHUB_PATH must be set}"
"$dest/bin/minisign" -v
