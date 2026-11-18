"""The pinned tool installers verify a download before they extract or run it."""

from __future__ import annotations

import hashlib
import io
import os
import platform
import stat
import subprocess
import tarfile
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / "scripts"
LIBRARY = SCRIPTS / "install_release_archive.sh"

# A stand-in for `curl`: records its arguments and copies a prepared payload to `-o FILE`.
FAKE_CURL = """#!/usr/bin/env bash
printf '%s\\n' "$@" >> "$FAKE_CURL_ARGS"
while [[ $# -gt 0 ]]; do
	if [[ "$1" == -o ]]; then
		cp "$FAKE_CURL_PAYLOAD" "$2"
	fi
	shift
done
"""

# script -> (URL prefix before the target triple, tool it installs)
PINNED = {
    "install_cargo_dist.sh": (
        "https://github.com/axodotdev/cargo-dist/releases/download/v0.34.0/cargo-dist-",
        "dist",
    ),
    "install_cargo_cyclonedx.sh": (
        "https://github.com/CycloneDX/cyclonedx-rust-cargo/releases/download/"
        "cargo-cyclonedx-0.5.5/cargo-cyclonedx-",
        "cargo-cyclonedx",
    ),
    "install_cargo_auditable.sh": (
        "https://github.com/rust-secure-code/cargo-auditable/releases/download/"
        "v0.7.7/cargo-auditable-",
        "cargo-auditable",
    ),
}


def _target() -> str:
    arch = {"x86_64": "x86_64", "arm64": "aarch64", "aarch64": "aarch64"}.get(
        platform.machine()
    )
    system = {"Linux": "unknown-linux-gnu", "Darwin": "apple-darwin"}.get(
        platform.system()
    )
    if arch is None or system is None:
        pytest.skip("the installers support Linux and macOS on x86_64 and arm64")
    return f"{arch}-{system}"


# Tool/platform pairs the upstream release does not build; the script exits before curl.
UNSUPPORTED = {("install_cargo_cyclonedx.sh", "aarch64-unknown-linux-gnu")}


def _skip_if_unsupported(script: str) -> None:
    if (script, _target()) in UNSUPPORTED:
        pytest.skip(f"{script} has no upstream build for {_target()}")


@pytest.fixture
def sandbox(tmp_path):
    """An environment whose `curl` serves `payload` and whose installs stay in tmp_path."""
    bin_dir = tmp_path / "fakebin"
    bin_dir.mkdir()
    curl = bin_dir / "curl"
    curl.write_text(FAKE_CURL)
    curl.chmod(curl.stat().st_mode | stat.S_IEXEC)
    (tmp_path / "runner").mkdir()
    (tmp_path / "github_path").write_text("")
    payload = tmp_path / "payload"
    env = {
        **os.environ,
        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
        "RUNNER_TEMP": str(tmp_path / "runner"),
        "GITHUB_PATH": str(tmp_path / "github_path"),
        "CARGO_HOME": str(tmp_path / "cargo"),
        "FAKE_CURL_ARGS": str(tmp_path / "curl_args"),
        "FAKE_CURL_PAYLOAD": str(payload),
    }

    class Sandbox:
        root = tmp_path
        installed = tmp_path / "cargo" / "bin"
        github_path = tmp_path / "github_path"
        curl_args = tmp_path / "curl_args"

        @staticmethod
        def serve(data: bytes) -> None:
            payload.write_bytes(data)

        @staticmethod
        def run(*command: str) -> subprocess.CompletedProcess[str]:
            return subprocess.run(
                command, env=env, capture_output=True, text=True, check=False
            )

    return Sandbox


def _archive(member: str, body: str) -> tuple[bytes, str]:
    """A .tar.xz holding one executable `member`, and its sha256."""
    data = body.encode()
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:xz") as tar:
        info = tarfile.TarInfo(member)
        info.size = len(data)
        info.mode = 0o755
        tar.addfile(info, io.BytesIO(data))
    blob = buffer.getvalue()
    return blob, hashlib.sha256(blob).hexdigest()


@pytest.mark.parametrize("script", PINNED)
def test_a_downloaded_archive_with_another_digest_fails_the_install(sandbox, script):
    _, tool = PINNED[script]
    _skip_if_unsupported(script)
    sandbox.serve(b"not the pinned archive")
    result = sandbox.run("bash", str(SCRIPTS / script))
    assert result.returncode != 0
    assert "digest mismatch" in result.stderr
    assert not (sandbox.installed / tool).exists()
    assert sandbox.github_path.read_text() == ""


@pytest.mark.parametrize("script", PINNED)
def test_the_download_is_the_pinned_release_asset_for_this_runner(sandbox, script):
    prefix, _ = PINNED[script]
    _skip_if_unsupported(script)
    sandbox.serve(b"not the pinned archive")
    sandbox.run("bash", str(SCRIPTS / script))
    args = sandbox.curl_args.read_text().split()
    assert f"{prefix}{_target()}.tar.xz" in args
    assert "--proto" in args


def _link_archive(kind: str) -> tuple[bytes, str]:
    """A .tar.xz whose `tool-x/tool` member is a symlink or hard link, and its sha256."""
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:xz") as tar:
        target = tarfile.TarInfo("tool-x/real")
        target.size = 5
        target.mode = 0o755
        tar.addfile(target, io.BytesIO(b"real\n"))
        link = tarfile.TarInfo("tool-x/tool")
        link.type = tarfile.SYMTYPE if kind == "symlink" else tarfile.LNKTYPE
        link.linkname = "real" if kind == "symlink" else "tool-x/real"
        tar.addfile(link)
    blob = buffer.getvalue()
    return blob, hashlib.sha256(blob).hexdigest()


INSTALL_TOOL = (
    'source "$1" && install_release_archive tool https://example.test/t.tar.xz "$2" "$3"'
)


@pytest.mark.parametrize("kind", ["symlink", "hardlink"])
def test_an_archive_member_that_is_a_link_is_refused(sandbox, kind):
    blob, digest = _link_archive(kind)
    sandbox.serve(blob)
    result = sandbox.run(
        "bash", "-c", INSTALL_TOOL, "bash", str(LIBRARY), digest, "tool-x/tool"
    )
    assert result.returncode != 0
    assert "not a plain regular file" in result.stderr
    assert not (sandbox.installed / "tool").exists()
    assert sandbox.github_path.read_text() == ""


def test_a_member_not_named_for_the_tool_is_refused(sandbox):
    blob, digest = _archive("tool-x/other", "#!/bin/sh\n")
    sandbox.serve(blob)
    result = sandbox.run(
        "bash", "-c", INSTALL_TOOL, "bash", str(LIBRARY), digest, "tool-x/other"
    )
    assert result.returncode != 0
    assert "must be named tool" in result.stderr
    assert not (sandbox.installed / "tool").exists()


def test_the_scratch_directory_is_removed_after_an_install(sandbox):
    blob, digest = _archive("tool-x/tool", "#!/bin/sh\n")
    sandbox.serve(blob)
    result = sandbox.run(
        "bash", "-c", INSTALL_TOOL, "bash", str(LIBRARY), digest, "tool-x/tool"
    )
    assert result.returncode == 0, result.stderr
    assert list((sandbox.root / "runner").iterdir()) == []


def test_a_matching_digest_installs_the_executable_and_exports_the_path(sandbox):
    blob, digest = _archive("tool-x/tool", "#!/bin/sh\necho stub-ran\n")
    sandbox.serve(blob)
    result = sandbox.run(
        "bash",
        "-c",
        'source "$1" && install_release_archive tool https://example.test/t.tar.xz "$2" tool-x/tool',
        "bash",
        str(LIBRARY),
        digest,
    )
    assert result.returncode == 0, result.stderr
    ran = sandbox.run(str(sandbox.installed / "tool"))
    assert ran.stdout == "stub-ran\n"
    assert sandbox.github_path.read_text().splitlines() == [str(sandbox.installed)]


def test_installing_twice_leaves_one_path_entry_and_a_working_executable(sandbox):
    blob, digest = _archive("tool-x/tool", "#!/bin/sh\necho stub-ran\n")
    sandbox.serve(blob)
    command = (
        "bash",
        "-c",
        'source "$1" && install_release_archive tool https://example.test/t.tar.xz "$2" tool-x/tool',
        "bash",
        str(LIBRARY),
        digest,
    )
    assert sandbox.run(*command).returncode == 0
    second = sandbox.run(*command)
    assert second.returncode == 0, second.stderr
    assert sandbox.run(str(sandbox.installed / "tool")).stdout == "stub-ran\n"
    assert sandbox.github_path.read_text().splitlines() == [str(sandbox.installed)]
