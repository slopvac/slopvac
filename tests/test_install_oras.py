"""scripts/install_oras.sh checks the archive digest before it extracts or runs anything.

The script runs against a fake ``curl`` that serves a local tarball and a fake
``uname``. The script commits the digest of the real upstream archive, so each test
runs a copy with those digests replaced by the digest of the tarball it serves.
"""

from __future__ import annotations

import hashlib
import io
import os
import re
import shutil
import subprocess
import tarfile
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "install_oras.sh"
FAKE_ORAS = "#!/bin/sh\ntouch \"$ORAS_RAN\"\necho 'oras stub version'\n"
FAKE_CURL = """#!/bin/sh
echo "$*" >> "$CURL_LOG"
while [ "$#" -gt 0 ]; do
    if [ "$1" = -o ]; then out="$2"; fi
    shift
done
cp "$ARCHIVE" "$out"
"""
FAKE_UNAME = """#!/bin/sh
case "$1" in
-s) echo "$FAKE_OS" ;;
-m) echo "$FAKE_ARCH" ;;
esac
"""
# macOS has shasum and no sha256sum.
SHA256SUM_SHIM = """#!/bin/sh
exec shasum -a 256 "$@"
"""


def tarball(path: Path, *, oras: bytes = FAKE_ORAS.encode()) -> str:
    """A tarball with an ``oras`` executable. Returns its sha256."""
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
        info = tarfile.TarInfo("oras")
        info.size, info.mode = len(oras), 0o755
        archive.addfile(info, io.BytesIO(oras))
    path.write_bytes(buffer.getvalue())
    return hashlib.sha256(buffer.getvalue()).hexdigest()


def install_script(tmp_path: Path, digest: str | None, *, drop_check: bool = False):
    """A copy of the installer with the committed digests replaced by ``digest``."""
    text = SCRIPT.read_text(encoding="utf-8")
    if digest is not None:
        text = re.sub(r"SHA256=[0-9a-f]{64}", f"SHA256={digest}", text)
    if drop_check:
        text = "\n".join(
            line for line in text.splitlines() if "sha256sum --check" not in line
        )
    copy = tmp_path / "install_oras.sh"
    copy.write_text(text, encoding="utf-8")
    return copy


class Run:
    def __init__(self, tmp_path: Path) -> None:
        self.tmp = tmp_path
        self.bin = tmp_path / "fake-bin"
        self.bin.mkdir()
        for name, body in {
            "curl": FAKE_CURL,
            "uname": FAKE_UNAME,
            **({} if shutil.which("sha256sum") else {"sha256sum": SHA256SUM_SHIM}),
        }.items():
            (self.bin / name).write_text(body, encoding="utf-8")
            (self.bin / name).chmod(0o755)
        self.archive = tmp_path / "served.tar.gz"
        self.runner_temp = tmp_path / "runner"
        self.runner_temp.mkdir()
        self.github_path = tmp_path / "github_path"
        self.github_path.touch()
        self.curl_log = tmp_path / "curl.log"
        self.ran = tmp_path / "oras-ran"

    def execute(
        self, script: Path, *, os_name: str = "Linux", arch: str = "x86_64"
    ) -> subprocess.CompletedProcess[str]:
        env = {
            **os.environ,
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "RUNNER_TEMP": str(self.runner_temp),
            "GITHUB_PATH": str(self.github_path),
            "ARCHIVE": str(self.archive),
            "CURL_LOG": str(self.curl_log),
            "ORAS_RAN": str(self.ran),
            "FAKE_OS": os_name,
            "FAKE_ARCH": arch,
        }
        return subprocess.run(
            ["bash", str(script)], env=env, capture_output=True, text=True, check=False
        )

    @property
    def installed(self) -> Path:
        return next(self.runner_temp.glob("oras-*/bin"), self.runner_temp) / "oras"


@pytest.mark.parametrize("arch", ["x86_64", "aarch64"])
def test_a_matching_archive_installs_oras_and_adds_it_to_path_once(
    tmp_path: Path, arch: str
) -> None:
    run = Run(tmp_path)
    digest = tarball(run.archive)
    result = run.execute(install_script(tmp_path, digest), arch=arch)
    assert result.returncode == 0, result.stderr
    assert run.installed.is_file()
    assert os.access(run.installed, os.X_OK)
    assert "oras stub version" in result.stdout
    assert run.github_path.read_text().splitlines() == [str(run.installed.parent)]


def test_a_corrupt_archive_fails_before_anything_is_extracted_or_run(
    tmp_path: Path,
) -> None:
    run = Run(tmp_path)
    tarball(run.archive)
    wrong_digest = hashlib.sha256(b"the archive upstream published").hexdigest()
    result = run.execute(install_script(tmp_path, wrong_digest))
    assert result.returncode != 0
    assert not run.installed.exists()
    assert not run.ran.exists()
    assert run.github_path.read_text() == ""


def test_the_digest_check_is_what_stops_a_corrupt_archive(tmp_path: Path) -> None:
    """Without the check the same corrupt archive installs, so the check matters."""
    run = Run(tmp_path)
    tarball(run.archive)
    wrong_digest = hashlib.sha256(b"the archive upstream published").hexdigest()
    result = run.execute(install_script(tmp_path, wrong_digest, drop_check=True))
    assert result.returncode == 0
    assert run.installed.is_file()


@pytest.mark.parametrize(
    ("os_name", "arch"),
    [("Darwin", "x86_64"), ("Linux", "riscv64"), ("Linux", "")],
    ids=["unsupported-os", "unsupported-arch", "no-arch"],
)
def test_an_unsupported_target_fails_without_downloading(
    tmp_path: Path, os_name: str, arch: str
) -> None:
    run = Run(tmp_path)
    tarball(run.archive)
    result = run.execute(install_script(tmp_path, None), os_name=os_name, arch=arch)
    assert result.returncode != 0
    assert not run.curl_log.exists()
    assert run.github_path.read_text() == ""
