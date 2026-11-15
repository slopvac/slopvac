from __future__ import annotations

import importlib.util
import json
import os
import shutil
import subprocess
import sys
import textwrap
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "release_signing.py"
SPEC = importlib.util.spec_from_file_location("release_signing", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
signing = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = signing
SPEC.loader.exec_module(signing)

SECRET = "untrusted comment: secret key\nRWRsecretmaterial\n"
POSIX_ONLY = pytest.mark.skipif(os.name != "posix", reason="uses a POSIX fake binary")

# A stand-in for minisign that records how it was called and writes a signature.
FAKE = textwrap.dedent(
    """\
    #!/usr/bin/env python3
    import json, os, stat, sys
    args = sys.argv[1:]
    log = os.environ["FAKE_LOG"]
    entry = {"args": args}
    if "-S" in args:
        key = args[args.index("-s") + 1]
        entry["key_mode"] = stat.S_IMODE(os.stat(key).st_mode)
        entry["key_text"] = open(key).read()
        entry["key_path"] = key
        entry["stdin_closed"] = sys.stdin.read() == ""
        if os.environ.get("FAKE_FAIL_SIGN"):
            print("sign refused", file=sys.stderr)
            sys.exit(1)
        open(args[args.index("-x") + 1], "w").write("sig")
    with open(log, "a") as handle:
        handle.write(json.dumps(entry) + "\\n")
    """
)


@pytest.fixture
def fake(tmp_path, monkeypatch):
    binary = tmp_path / "fake-minisign"
    binary.write_text(FAKE, encoding="utf-8")
    binary.chmod(0o755)
    log = tmp_path / "calls.jsonl"
    monkeypatch.setenv("MINISIGN", str(binary))
    monkeypatch.setenv("FAKE_LOG", str(log))
    monkeypatch.setenv("RUNNER_TEMP", str(tmp_path))
    return log


@pytest.fixture
def release(tmp_path):
    directory = tmp_path / "artifacts"
    directory.mkdir()
    (directory / "slopvac-x86_64.tar.xz").write_bytes(b"archive")
    (directory / "sha256.sum").write_text("abc  slopvac-x86_64.tar.xz\n")
    pub = tmp_path / "release.pub"
    pub.write_text("untrusted comment: public\nRWTpub\n")
    return directory, pub


def calls(log: Path) -> list[dict]:
    return [json.loads(line) for line in log.read_text().splitlines()]


def run_sign(directory, pub, **env):
    environ = {**os.environ, **env}
    return signing.sign(directory, pub, environ)


@POSIX_ONLY
def test_every_artifact_gets_a_signature_that_is_then_verified(fake, release):
    directory, pub = release
    run_sign(directory, pub, MINISIGN_SECRET_KEY=SECRET)
    assert sorted(p.name for p in directory.glob("*.minisig")) == [
        "sha256.sum.minisig",
        "slopvac-x86_64.tar.xz.minisig",
    ]
    verifications = [c["args"] for c in calls(fake) if "-V" in c["args"]]
    assert len(verifications) == 2
    for args in verifications:
        assert args[args.index("-p") + 1] == str(pub)


@POSIX_ONLY
def test_the_secret_key_file_is_private_and_removed_afterwards(fake, release):
    directory, pub = release
    run_sign(directory, pub, MINISIGN_SECRET_KEY=SECRET)
    signs = [c for c in calls(fake) if "-S" in c["args"]]
    assert {c["key_mode"] for c in signs} == {0o600}
    assert {c["key_text"] for c in signs} == {SECRET}
    for sign_call in signs:
        assert not Path(sign_call["key_path"]).exists()
        assert not Path(sign_call["key_path"]).parent.exists()


@POSIX_ONLY
def test_the_key_file_is_removed_when_signing_fails(fake, release, monkeypatch):
    directory, pub = release
    monkeypatch.setenv("FAKE_FAIL_SIGN", "1")
    with pytest.raises(signing.SigningError, match="sign refused"):
        run_sign(directory, pub, MINISIGN_SECRET_KEY=SECRET)
    assert list(Path(os.environ["RUNNER_TEMP"]).glob("minisign-*")) == []


@POSIX_ONLY
def test_signing_is_non_interactive_for_the_unencrypted_release_key(fake, release):
    directory, pub = release
    run_sign(directory, pub, MINISIGN_SECRET_KEY=SECRET)
    signs = [c for c in calls(fake) if "-S" in c["args"]]
    assert signs
    for call in signs:
        assert "-W" in call["args"]
        assert call["stdin_closed"] is True
        assert not any("RWRsecretmaterial" in arg for arg in call["args"])


@POSIX_ONLY
def test_a_missing_public_key_stops_signing_before_any_signature(fake, release):
    directory, pub = release
    pub.unlink()
    with pytest.raises(signing.SigningError, match="key PR"):
        run_sign(directory, pub, MINISIGN_SECRET_KEY=SECRET)
    assert list(directory.glob("*.minisig")) == []
    assert not fake.exists()


@pytest.mark.parametrize("value", [None, "", "  \n"])
def test_a_missing_secret_stops_signing(release, value):
    directory, pub = release
    env = {} if value is None else {"MINISIGN_SECRET_KEY": value}
    with pytest.raises(signing.SigningError, match="MINISIGN_SECRET_KEY"):
        signing.sign(directory, pub, env)
    assert list(directory.glob("*.minisig")) == []


def test_an_empty_artifact_directory_is_an_error(tmp_path, release):
    _, pub = release
    empty = tmp_path / "empty"
    empty.mkdir()
    with pytest.raises(signing.SigningError, match="no artifacts"):
        signing.sign(empty, pub, {"MINISIGN_SECRET_KEY": SECRET})


def test_a_subdirectory_in_the_artifact_directory_is_an_error(fake, release):
    directory, pub = release
    (directory / "nested").mkdir()
    with pytest.raises(signing.SigningError, match="only files"):
        signing.sign(directory, pub, {"MINISIGN_SECRET_KEY": SECRET})


def test_require_key_exits_nonzero_without_the_public_key(tmp_path):
    result = subprocess.run(
        [
            sys.executable,
            str(SCRIPT),
            "--pubkey",
            str(tmp_path / "none.pub"),
            "require-key",
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 1
    assert "key PR" in result.stderr


# --- the real minisign binary ------------------------------------------------

MINISIGN = shutil.which(os.environ.get("MINISIGN_REAL", "minisign"))
needs_minisign = pytest.mark.skipif(
    MINISIGN is None,
    reason="minisign is not installed (mise install provides it)",
)


def generate_key(tmp_path: Path) -> tuple[Path, Path]:
    """A throwaway unencrypted keypair, like the release key."""
    pub, sec = tmp_path / "gen.pub", tmp_path / "gen.key"
    argv = [MINISIGN, "-G", "-W", "-f", "-p", str(pub), "-s", str(sec)]
    subprocess.run(argv, check=True, capture_output=True, stdin=subprocess.DEVNULL)
    return pub, sec


@pytest.fixture
def real(monkeypatch, tmp_path):
    monkeypatch.setenv("MINISIGN", MINISIGN or "minisign")
    monkeypatch.setenv("RUNNER_TEMP", str(tmp_path))


@needs_minisign
def test_signed_artifacts_verify_and_a_tampered_one_is_rejected(
    real, tmp_path, release
):
    directory, _ = release
    pub, sec = generate_key(tmp_path)
    signing.sign(directory, pub, {"MINISIGN_SECRET_KEY": sec.read_text()})
    files = [p for p in directory.iterdir() if p.suffix != ".minisig"]
    signing.verify(files, pub)

    files[0].write_bytes(files[0].read_bytes() + b"tampered")
    with pytest.raises(signing.SigningError, match=files[0].name):
        signing.verify(files, pub)


@needs_minisign
def test_a_signature_from_another_key_is_rejected(real, tmp_path, release):
    directory, _ = release
    pub, sec = generate_key(tmp_path)
    signing.sign(directory, pub, {"MINISIGN_SECRET_KEY": sec.read_text()})
    other = tmp_path / "other"
    other.mkdir()
    other_pub, _ = generate_key(other)
    files = [p for p in directory.iterdir() if p.suffix != ".minisig"]
    with pytest.raises(signing.SigningError):
        signing.verify(files, other_pub)


@needs_minisign
def test_the_documented_user_command_verifies_a_signed_file(real, tmp_path, release):
    directory, _ = release
    pub, sec = generate_key(tmp_path)
    signing.sign(directory, pub, {"MINISIGN_SECRET_KEY": sec.read_text()})
    artifact = directory / "slopvac-x86_64.tar.xz"
    ok = subprocess.run(
        [MINISIGN, "-Vm", str(artifact), "-p", str(pub)],
        capture_output=True,
        check=False,
    )
    assert ok.returncode == 0
    artifact.write_bytes(b"evil")
    bad = subprocess.run(
        [MINISIGN, "-Vm", str(artifact), "-p", str(pub)],
        capture_output=True,
        check=False,
    )
    assert bad.returncode != 0


def test_the_committed_public_key_satisfies_the_release_gate():
    # The default path is the file the workflows check, so a rename breaks releases.
    assert signing.PUBKEY == ROOT / "keys" / "slopvac-release.pub"
    signing.require_key(signing.PUBKEY)
    first = signing.PUBKEY.read_text(encoding="utf-8").splitlines()[0]
    assert first == "untrusted comment: minisign public key D9914BDB4570AC6D"
