# shellcheck shell=bash
# Library for the pinned tool installers in this directory. Source it; do not run it.
#
# A release job must not run a download that nothing vouches for. These functions fetch a
# release archive, check it against a sha256 that the calling script commits, and only then
# extract and install one binary. A changed or substituted asset fails the job before
# anything is extracted or run.
#
# Supported runners: Linux and macOS on x86_64 and arm64. 3.0 has no Windows build.

# Print the Rust target triple of this runner, as the upstream archives name it.
release_target() {
	local os arch
	case "$(uname -s)" in
	Linux) os=unknown-linux-gnu ;;
	Darwin) os=apple-darwin ;;
	*)
		echo "pinned installers support Linux and macOS runners only" >&2
		return 1
		;;
	esac
	case "$(uname -m)" in
	x86_64) arch=x86_64 ;;
	aarch64 | arm64) arch=aarch64 ;;
	*)
		echo "unsupported architecture: $(uname -m)" >&2
		return 1
		;;
	esac
	echo "${arch}-${os}"
}

# install_release_archive TOOL URL SHA256 MEMBER
#   TOOL    name of the installed executable
#   URL     https URL of the .tar.xz archive
#   SHA256  committed digest of that archive
#   MEMBER  path of the executable inside the archive
# Installs into "${CARGO_HOME:-$HOME/.cargo}/bin" and adds that directory to GITHUB_PATH
# once, so a repeated install leaves the same state.
install_release_archive() {
	local tool="$1" url="$2" expected="$3" member="$4"
	local work archive actual bin
	if [[ "${member##*/}" != "$tool" ]]; then
		echo "${tool}: the archive member must be named ${tool}, not ${member##*/}" >&2
		return 1
	fi
	work="$(mktemp -d "${RUNNER_TEMP:?RUNNER_TEMP must be set}/pinned-${tool}.XXXXXX")"
	# shellcheck disable=SC2064 # expand $work now: the trap must outlive this function's locals
	trap "rm -rf '$work'" RETURN
	archive="$work/archive.tar.xz"
	curl --proto '=https' --tlsv1.2 -fsSL --retry 3 -o "$archive" "$url"

	if command -v sha256sum >/dev/null 2>&1; then
		actual="$(sha256sum "$archive" | cut -d' ' -f1)"
	else
		actual="$(shasum -a 256 "$archive" | cut -d' ' -f1)"
	fi
	if [[ "$actual" != "$expected" ]]; then
		echo "${tool} digest mismatch for ${url##*/}: expected ${expected}, got ${actual}" >&2
		return 1
	fi

	mkdir "$work/out"
	tar -xJf "$archive" -C "$work/out" "$member"
	# A regular file with one link: a symlink or hard link could point outside the archive.
	if [[ -z "$(find "$work/out/$member" -maxdepth 0 -type f -links 1 2>/dev/null)" ]]; then
		echo "${tool}: the archive member ${member} is not a plain regular file" >&2
		return 1
	fi
	bin="${CARGO_HOME:-$HOME/.cargo}/bin"
	mkdir -p "$bin"
	install -m 0755 "$work/out/$member" "$bin/$tool"
	if ! grep -qxF "$bin" "${GITHUB_PATH:?GITHUB_PATH must be set}"; then
		echo "$bin" >>"$GITHUB_PATH"
	fi
}
