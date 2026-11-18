#!/usr/bin/env python3
"""Download and install a pinned Vale release without external Unix tools."""

from __future__ import annotations

import argparse
import hashlib
import http.client
import os
import platform
import shutil
import subprocess
import tarfile
import tempfile
import time
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import urlopen

RELEASE_BASE = "https://github.com/vale-cli/vale/releases/download"

# The one Vale pin for CI and the GitHub Action. Dependabot cannot update it; the
# weekly pin-check workflow opens an issue when a newer Vale release exists.
VALE_VERSION = "3.24.0"

# Total download attempts per file, and the wait before the second attempt. Each
# later wait doubles: 2 s, then 4 s.
DOWNLOAD_ATTEMPTS = 3
BACKOFF_SECONDS = 2.0


class InstallError(RuntimeError):
    """Raised when a Vale release cannot be installed safely."""


def _asset_name(version: str) -> str:
    system = platform.system()
    machine = platform.machine().lower()
    systems = {"Linux": "Linux", "Darwin": "macOS"}
    system_name = systems.get(system)
    if system_name is None:
        raise InstallError(f"unsupported platform: {system}")

    if machine in {"x86_64", "amd64"}:
        architecture = "64-bit"
    elif machine in {"arm64", "aarch64"}:
        architecture = "arm64"
    else:
        raise InstallError(f"unsupported architecture: {platform.machine()}")
    return f"vale_{version}_{system_name}_{architecture}.tar.gz"


def _transient(exc: BaseException) -> bool:
    """Return whether a download failure is worth another attempt.

    Server errors (5xx), refused or dropped connections and timeouts are
    transient. Any 4xx answer is final.
    """
    if isinstance(exc, HTTPError):
        return exc.code >= 500
    return isinstance(exc, (OSError, http.client.HTTPException))


def _download(url: str, destination: Path) -> None:
    for attempt in range(1, DOWNLOAD_ATTEMPTS + 1):
        try:
            with (
                urlopen(url, timeout=60) as response,
                destination.open("wb") as output,
            ):
                while chunk := response.read(1024 * 1024):
                    output.write(chunk)
            return
        except (OSError, http.client.HTTPException) as exc:
            if not _transient(exc):
                raise InstallError(f"download failed for {url}: {exc}") from exc
            if attempt == DOWNLOAD_ATTEMPTS:
                raise InstallError(
                    f"download failed for {url} after {attempt} attempts: {exc}"
                ) from exc
            time.sleep(BACKOFF_SECONDS * 2 ** (attempt - 1))


def _checksum(checksums: Path, archive_name: str) -> str:
    for line in checksums.read_text(encoding="utf-8").splitlines():
        fields = line.split()
        if len(fields) >= 2 and fields[-1].lstrip("*") == archive_name:
            return fields[0]
    raise InstallError(f"checksum file has no entry for {archive_name}")


def _verify_checksum(archive: Path, expected: str) -> None:
    digest = hashlib.sha256()
    with archive.open("rb") as input_file:
        for chunk in iter(lambda: input_file.read(1024 * 1024), b""):
            digest.update(chunk)
    actual = digest.hexdigest()
    if actual.lower() != expected.lower():
        raise InstallError(
            f"checksum mismatch for {archive.name}: expected {expected}, got {actual}"
        )


def _install_binary(archive: Path, prefix: Path) -> Path:
    target = prefix / "vale"
    prefix.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive, mode="r:gz") as package:
        try:
            member = package.getmember("vale")
        except KeyError as exc:
            raise InstallError("release archive has no vale member") from exc
        if not member.isfile() or member.name != "vale":
            raise InstallError("release archive's vale member is not a regular file")
        source = package.extractfile(member)
        if source is None:  # pragma: no cover - tarfile contract guard
            raise InstallError("could not read vale member from release archive")
        temporary = target.with_name(f".{target.name}.{os.getpid()}.tmp")
        try:
            with source, temporary.open("wb") as output:
                for chunk in iter(lambda: source.read(1024 * 1024), b""):
                    output.write(chunk)
            temporary.chmod(0o755)
            os.replace(temporary, target)
        finally:
            temporary.unlink(missing_ok=True)
    return target


def _verify_version(binary: Path) -> str:
    try:
        result = subprocess.run(
            [str(binary), "--version"],
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError as exc:
        raise InstallError(f"could not execute {binary}: {exc}") from exc
    output = (result.stdout or result.stderr).strip()
    if result.returncode != 0:
        raise InstallError(f"{binary} --version failed ({result.returncode}): {output}")
    if not output:
        raise InstallError(f"{binary} --version produced no output")
    return output


def install(
    version: str, prefix: Path, cache_dir: Path | None = None
) -> tuple[Path, str]:
    """Install Vale into ``prefix``.

    With ``cache_dir``, a previously stored archive and checksums file are used
    without network access. Either way the archive is checked against the
    checksums file before it is unpacked. Files enter the cache only after that
    check passes.
    """
    archive_name = _asset_name(version)
    checksums_name = f"vale_{version}_checksums.txt"
    base = f"{RELEASE_BASE}/v{version}"
    with tempfile.TemporaryDirectory(prefix="slopvac-vale-") as temporary:
        directory = Path(temporary)
        archive = directory / archive_name
        checksums = directory / checksums_name
        cached = cache_dir is not None and all(
            (cache_dir / name).is_file() for name in (archive_name, checksums_name)
        )
        if cached and cache_dir is not None:
            shutil.copyfile(cache_dir / archive_name, archive)
            shutil.copyfile(cache_dir / checksums_name, checksums)
        else:
            _download(f"{base}/{archive_name}", archive)
            _download(f"{base}/{checksums_name}", checksums)
        _verify_checksum(archive, _checksum(checksums, archive_name))
        if cache_dir is not None and not cached:
            cache_dir.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(archive, cache_dir / archive_name)
            shutil.copyfile(checksums, cache_dir / checksums_name)
        binary = _install_binary(archive, prefix)
    return binary, _verify_version(binary)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "version",
        nargs="?",
        default="",
        help=f"Vale release version; empty installs the pinned {VALE_VERSION}",
    )
    parser.add_argument(
        "--prefix",
        type=Path,
        default=Path("/usr/local/bin"),
        help="directory in which to install vale (default: /usr/local/bin)",
    )
    parser.add_argument(
        "--cache-dir",
        type=Path,
        default=None,
        help="directory that holds the verified archive between runs",
    )
    args = parser.parse_args()
    try:
        binary, version = install(
            args.version or VALE_VERSION,
            args.prefix.expanduser(),
            args.cache_dir.expanduser() if args.cache_dir else None,
        )
    except InstallError as exc:
        parser.error(str(exc))
    print(f"installed {binary}")
    print(version)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
