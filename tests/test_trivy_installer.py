"""Run the real Install Trivy step against local fixture archives, offline."""

from __future__ import annotations

import hashlib
import io
import os
import shutil
import stat
import subprocess
import tarfile
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/security.yml"
JOBS = ["trivy", "trivy-secret"]
ARCHES = [("X64", "Linux-64bit"), ("ARM64", "Linux-ARM64")]

pytestmark = pytest.mark.skipif(
    shutil.which("sha256sum") is None or shutil.which("bash") is None,
    reason="needs bash and GNU sha256sum",
)

# Stands in for curl: logs the URL, then copies the fixture named by its basename.
CURL_STUB = """#!/bin/sh
out=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out=$2; shift 2 ;;
    -*) shift ;;
    *) url=$1; shift ;;
  esac
done
echo "$url" >> "$FIXTURE_DIR/urls"
cp "$FIXTURE_DIR/$(basename "$url")" "$out"
"""


def install_step(job: str) -> dict:
    steps = yaml.safe_load(WORKFLOW.read_text())["jobs"][job]["steps"]
    return next(s for s in steps if s.get("name") == "Install Trivy")


def archive(asset: str) -> bytes:
    """A tarball like the release asset: trivy plus files the step must not extract."""
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        for name, data in {
            "trivy": f"#!/bin/sh\necho {asset}\n".encode(),
            "LICENSE": b"license\n",
            "contrib/gitlab.tpl": b"template\n",
        }.items():
            info = tarfile.TarInfo(name)
            info.size = len(data)
            info.mode = 0o755
            tar.addfile(info, io.BytesIO(data))
    return buf.getvalue()


def run_step(tmp_path: Path, job: str, arch: str, asset: str, *, good: bool):
    step = install_step(job)
    version = step["env"]["VERSION"]
    name = f"trivy_{version}_{asset}.tar.gz"
    fixtures, bin_dir = tmp_path / "fixtures", tmp_path / "stubs"
    work, runner_temp = tmp_path / "workspace", tmp_path / "runner"
    for d in (fixtures, bin_dir, work, runner_temp):
        d.mkdir()
    # Both arches are served, so a swapped map downloads the wrong archive.
    digests = {}
    for _, other in ARCHES:
        data = archive(other)
        (fixtures / f"trivy_{version}_{other}.tar.gz").write_bytes(data)
        digests[other] = hashlib.sha256(data).hexdigest()
    digest = digests[asset] if good else "0" * 64
    curl = bin_dir / "curl"
    curl.write_text(CURL_STUB)
    curl.chmod(curl.stat().st_mode | stat.S_IXUSR)
    github_path = tmp_path / "github_path"
    github_path.write_text("")
    env = {
        **os.environ,
        **{k: str(v) for k, v in step["env"].items()},
        "SHA256_X64": digests["Linux-64bit"] if arch == "X64" else "0" * 64,
        "SHA256_ARM64": digests["Linux-ARM64"] if arch == "ARM64" else "0" * 64,
        "RUNNER_ARCH": arch,
        "RUNNER_TEMP": str(runner_temp),
        "GITHUB_PATH": str(github_path),
        "FIXTURE_DIR": str(fixtures),
        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
    }
    if not good:
        env["SHA256_X64" if arch == "X64" else "SHA256_ARM64"] = digest
    proc = subprocess.run(
        ["bash", "-c", step["run"]],
        cwd=work,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    urls = (fixtures / "urls").read_text().split() if (fixtures / "urls").exists() else []
    return proc, work, runner_temp, github_path, name, urls


@pytest.mark.parametrize("job", JOBS)
@pytest.mark.parametrize(("arch", "asset"), ARCHES)
def test_good_digest_installs_only_trivy(tmp_path: Path, job: str, arch: str, asset: str) -> None:
    proc, work, runner_temp, github_path, name, urls = run_step(
        tmp_path, job, arch, asset, good=True
    )
    assert proc.returncode == 0, proc.stderr
    assert len(urls) == 1
    assert urls[0].startswith("https://github.com/aquasecurity/trivy/releases/download/v")
    assert urls[0].endswith(f"/{name}")
    dest = next(runner_temp.glob("trivy-*"))
    assert sorted(p.name for p in (dest / "bin").iterdir()) == ["trivy"]
    assert (dest / "bin" / "trivy").read_text().strip().endswith(f"echo {asset}")
    assert github_path.read_text() == f"{dest / 'bin'}\n"
    assert list(work.iterdir()) == []


@pytest.mark.parametrize("job", JOBS)
@pytest.mark.parametrize(("arch", "asset"), ARCHES)
def test_bad_digest_installs_nothing(tmp_path: Path, job: str, arch: str, asset: str) -> None:
    proc, work, runner_temp, github_path, name, urls = run_step(
        tmp_path, job, arch, asset, good=False
    )
    assert proc.returncode != 0
    assert urls and urls[0].endswith(f"/{name}")
    dest = next(runner_temp.glob("trivy-*"))
    assert list((dest / "bin").iterdir()) == []
    assert not any(p.name == "trivy" for p in runner_temp.rglob("*"))
    assert github_path.read_text() == ""
    assert list(work.iterdir()) == []
