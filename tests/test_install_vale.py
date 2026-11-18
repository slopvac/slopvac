"""The Vale installer retries transient download failures and verifies checksums.

The HTTP boundary is faked: ``urlopen`` is replaced by a scripted opener, so no
test touches the network or waits for a real backoff.
"""

from __future__ import annotations

import hashlib
import http.client
import importlib.util
import io
import platform
import ssl
import sys
import tarfile
from pathlib import Path
from typing import Self
from urllib.error import HTTPError, URLError

import pytest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "install_vale", ROOT / "scripts" / "install_vale.py"
)
assert SPEC is not None and SPEC.loader is not None
install_vale = importlib.util.module_from_spec(SPEC)
sys.modules["install_vale"] = install_vale
SPEC.loader.exec_module(install_vale)

VERSION = "9.9.9"
ASSET = f"vale_{VERSION}_Linux_64-bit.tar.gz"
FAKE_VALE = b"#!/bin/sh\necho vale version 9.9.9\n"


def _archive() -> bytes:
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w:gz") as package:
        info = tarfile.TarInfo("vale")
        info.size = len(FAKE_VALE)
        info.mode = 0o755
        package.addfile(info, io.BytesIO(FAKE_VALE))
    return buffer.getvalue()


class _Response(io.BytesIO):
    def __enter__(self) -> Self:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()


class _BrokenResponse(_Response):
    """A response whose body fails part-way, like a dropped connection."""

    def __init__(self, error: BaseException) -> None:
        super().__init__(b"partial")
        self.error = error

    def read(self, size: int | None = -1) -> bytes:
        raise self.error


class MidRead:
    """Fault marker: open succeeds, then reading the body raises ``error``."""

    def __init__(self, error: BaseException) -> None:
        self.error = error


class FakeNetwork:
    """Serve scripted responses per URL suffix and record every request."""

    def __init__(self, archive: bytes, checksum: str | None = None) -> None:
        self.archive = archive
        self.checksum = checksum or hashlib.sha256(archive).hexdigest()
        self.faults: dict[str, list[BaseException | MidRead]] = {}
        self.requests: list[str] = []

    def fail(self, suffix: str, *errors: BaseException | MidRead) -> None:
        self.faults[suffix] = list(errors)

    def __call__(self, url: str, timeout: float = 0) -> _Response:
        self.requests.append(url)
        for suffix, errors in self.faults.items():
            if url.endswith(suffix) and errors:
                fault = errors.pop(0)
                if isinstance(fault, MidRead):
                    return _BrokenResponse(fault.error)
                raise fault
        if url.endswith(".tar.gz"):
            return _Response(self.archive)
        return _Response(f"{self.checksum}  {ASSET}\n".encode())


def _http(code: int) -> HTTPError:
    return HTTPError("https://example.invalid", code, "boom", {}, None)  # type: ignore[arg-type]


@pytest.fixture
def network(monkeypatch: pytest.MonkeyPatch) -> FakeNetwork:
    fake = FakeNetwork(_archive())
    monkeypatch.setattr(install_vale, "urlopen", fake)
    monkeypatch.setattr(platform, "system", lambda: "Linux")
    monkeypatch.setattr(platform, "machine", lambda: "x86_64")
    sleeps: list[float] = []
    monkeypatch.setattr(install_vale.time, "sleep", sleeps.append)
    fake.sleeps = sleeps  # type: ignore[attr-defined]
    return fake


def test_retries_a_server_error_then_succeeds(network: FakeNetwork, tmp_path: Path):
    network.fail(".tar.gz", _http(500))

    binary, version = install_vale.install(VERSION, tmp_path / "bin")

    assert binary.read_bytes() == FAKE_VALE
    assert "9.9.9" in version
    assert [u for u in network.requests if u.endswith(".tar.gz")] == [
        f"{install_vale.RELEASE_BASE}/v{VERSION}/{ASSET}"
    ] * 2
    assert network.sleeps == [2.0]  # type: ignore[attr-defined]


@pytest.mark.parametrize(
    "error",
    [URLError("connection refused"), TimeoutError("timed out"), _http(503)],
    ids=["connection", "timeout", "503"],
)
def test_retries_connection_failures_and_timeouts(
    network: FakeNetwork, tmp_path: Path, error: BaseException
):
    network.fail(".tar.gz", error)

    install_vale.install(VERSION, tmp_path / "bin")

    assert sum(u.endswith(".tar.gz") for u in network.requests) == 2


def test_backoff_doubles_between_attempts(network: FakeNetwork, tmp_path: Path):
    network.fail(".tar.gz", _http(500), _http(502))

    install_vale.install(VERSION, tmp_path / "bin")

    assert network.sleeps == [2.0, 4.0]  # type: ignore[attr-defined]


def test_does_not_retry_a_client_error(network: FakeNetwork, tmp_path: Path):
    network.fail(".tar.gz", _http(404))

    with pytest.raises(install_vale.InstallError, match="download failed.*404"):
        install_vale.install(VERSION, tmp_path / "bin")

    assert len(network.requests) == 1
    assert network.sleeps == []  # type: ignore[attr-defined]


def test_gives_up_after_three_attempts(network: FakeNetwork, tmp_path: Path):
    network.fail(".tar.gz", _http(500), _http(500), _http(500), _http(500))

    with pytest.raises(install_vale.InstallError, match="after 3 attempts"):
        install_vale.install(VERSION, tmp_path / "bin")

    assert len(network.requests) == 3
    assert network.sleeps == [2.0, 4.0]  # type: ignore[attr-defined]
    assert not (tmp_path / "bin" / "vale").exists()


@pytest.mark.parametrize(
    "error",
    [
        http.client.IncompleteRead(b"par", 100),
        ConnectionResetError("reset by peer"),
    ],
    ids=["incomplete-read", "connection-reset"],
)
def test_retries_a_body_that_fails_mid_read(
    network: FakeNetwork, tmp_path: Path, error: BaseException
):
    network.fail(".tar.gz", MidRead(error))
    cache = tmp_path / "cache"

    binary, _ = install_vale.install(VERSION, tmp_path / "bin", cache)

    assert binary.read_bytes() == FAKE_VALE
    assert (cache / ASSET).read_bytes() == network.archive
    assert sum(u.endswith(".tar.gz") for u in network.requests) == 2


def test_local_write_failure_is_not_retried(
    network: FakeNetwork, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    def deny(self: Path, data: bytes) -> int:
        raise PermissionError(13, "Permission denied", str(self))

    monkeypatch.setattr(Path, "write_bytes", deny)

    with pytest.raises(install_vale.InstallError, match="could not write"):
        install_vale.install(VERSION, tmp_path / "bin")

    assert len(network.requests) == 1
    assert network.sleeps == []  # type: ignore[attr-defined]
    assert not (tmp_path / "bin" / "vale").exists()


def test_tls_certificate_failure_is_not_retried(network: FakeNetwork, tmp_path: Path):
    network.fail(
        ".tar.gz",
        URLError(ssl.SSLCertVerificationError("certificate verify failed")),
    )

    with pytest.raises(install_vale.InstallError, match="download failed"):
        install_vale.install(VERSION, tmp_path / "bin")

    assert len(network.requests) == 1
    assert network.sleeps == []  # type: ignore[attr-defined]
    assert not (tmp_path / "bin" / "vale").exists()


def test_checksum_mismatch_fails_without_retry(network: FakeNetwork, tmp_path: Path):
    network.checksum = "0" * 64

    with pytest.raises(install_vale.InstallError, match="checksum mismatch"):
        install_vale.install(VERSION, tmp_path / "bin")

    assert len(network.requests) == 2  # archive and checksums, once each
    assert not (tmp_path / "bin" / "vale").exists()


def test_cached_archive_skips_the_network(network: FakeNetwork, tmp_path: Path):
    cache = tmp_path / "cache"
    install_vale.install(VERSION, tmp_path / "first", cache)
    network.requests.clear()

    binary, _ = install_vale.install(VERSION, tmp_path / "second", cache)

    assert network.requests == []
    assert binary.read_bytes() == FAKE_VALE


def test_cached_archive_is_still_verified(network: FakeNetwork, tmp_path: Path):
    cache = tmp_path / "cache"
    install_vale.install(VERSION, tmp_path / "first", cache)
    tampered = _archive() + b"tampered"
    (cache / f"{ASSET}").write_bytes(tampered)
    network.requests.clear()

    with pytest.raises(install_vale.InstallError, match="checksum mismatch"):
        install_vale.install(VERSION, tmp_path / "second", cache)

    assert network.requests == []


def test_unverified_download_never_enters_the_cache(
    network: FakeNetwork, tmp_path: Path
):
    network.checksum = "0" * 64
    cache = tmp_path / "cache"

    with pytest.raises(install_vale.InstallError):
        install_vale.install(VERSION, tmp_path / "bin", cache)

    assert not cache.exists() or not any(cache.iterdir())
