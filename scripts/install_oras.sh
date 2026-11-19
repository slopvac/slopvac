#!/usr/bin/env bash
# Install a pinned oras release binary on a Linux runner and put it on PATH.
#
# The tarball is checked against a sha256 that is committed here, so a changed or
# substituted release asset fails the job. The digests were computed from the
# upstream v1.3.6 assets and match oras_1.3.6_checksums.txt in that release.
#
# This follows install_minisign.sh. When scripts/install_release_archive.sh is on
# main, it can take over the download and digest check; oras ships .tar.gz, and that
# library handles .tar.xz.
#
# To upgrade: change VERSION and both digests together, then compare them with the
# upstream checksums file.
set -euo pipefail

VERSION=1.3.6

if [[ "$(uname -s)" != Linux ]]; then
	echo "install_oras.sh supports Linux runners only" >&2
	exit 1
fi
case "$(uname -m)" in
x86_64)
	arch=amd64
	SHA256=66fb0ad7206f3f6fa77ed5b552049d364ccf4b50b1684554f731f76302b52ca0
	;;
aarch64 | arm64)
	arch=arm64
	SHA256=b84269e78dfc2d5329a4aab373da3200de95e6c138338cf6c66c10c0a3f248c1
	;;
*)
	echo "unsupported architecture: $(uname -m)" >&2
	exit 1
	;;
esac

URL="https://github.com/oras-project/oras/releases/download/v${VERSION}/oras_${VERSION}_linux_${arch}.tar.gz"
dest="${RUNNER_TEMP:?RUNNER_TEMP must be set}/oras-${VERSION}"
mkdir -p "$dest/bin"
curl --proto '=https' --tlsv1.2 -fsSL --retry 3 -o "$dest/oras.tar.gz" "$URL"
echo "${SHA256}  ${dest}/oras.tar.gz" | sha256sum --check --strict -
tar -xzf "$dest/oras.tar.gz" -C "$dest/bin" oras
chmod 0755 "$dest/bin/oras"
echo "$dest/bin" >>"${GITHUB_PATH:?GITHUB_PATH must be set}"
"$dest/bin/oras" version
