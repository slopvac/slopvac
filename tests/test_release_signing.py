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

VERSION = "3.1.0"
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
    (directory / "slopvac-x86_64.tar.gz").write_bytes(b"archive")
    (directory / "sha256.sum").write_text("abc  slopvac-x86_64.tar.gz\n")
    pub = tmp_path / "release.pub"
    pub.write_text("untrusted comment: public\nRWTpub\n")
    return directory, pub


def calls(log: Path) -> list[dict]:
    return [json.loads(line) for line in log.read_text().splitlines()]


def run_sign(directory, pub, **env):
    environ = {**os.environ, **env}
    return signing.sign(directory, pub, environ, VERSION)


@POSIX_ONLY
def test_every_artifact_gets_a_signature_that_is_then_verified(fake, release):
    directory, pub = release
    run_sign(directory, pub, MINISIGN_SECRET_KEY=SECRET)
    assert sorted(p.name for p in directory.glob("*.minisig")) == [
        "sha256.sum.minisig",
        "slopvac-x86_64.tar.gz.minisig",
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
        signing.sign(directory, pub, env, VERSION)
    assert list(directory.glob("*.minisig")) == []


def test_an_empty_artifact_directory_is_an_error(tmp_path, release):
    _, pub = release
    empty = tmp_path / "empty"
    empty.mkdir()
    with pytest.raises(signing.SigningError, match="no artifacts"):
        signing.sign(empty, pub, {"MINISIGN_SECRET_KEY": SECRET}, VERSION)


def test_a_subdirectory_in_the_artifact_directory_is_an_error(fake, release):
    directory, pub = release
    (directory / "nested").mkdir()
    with pytest.raises(signing.SigningError, match="only files"):
        signing.sign(directory, pub, {"MINISIGN_SECRET_KEY": SECRET}, VERSION)


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


def generate_key(tmp_path: Path) -> tuple[Path, Path]:
    """A throwaway unencrypted keypair, like the release key."""
    pub, sec = tmp_path / "gen.pub", tmp_path / "gen.key"
    argv = [MINISIGN, "-G", "-W", "-f", "-p", str(pub), "-s", str(sec)]
    subprocess.run(argv, check=True, capture_output=True, stdin=subprocess.DEVNULL)
    return pub, sec


@pytest.fixture
def real(monkeypatch, tmp_path):
    if MINISIGN is None:
        message = "minisign is not installed (mise install provides it)"
        # CI installs the pinned binary, so a missing one is a broken install, not a skip.
        if os.environ.get("CI"):
            pytest.fail(message)
        pytest.skip(message)
    monkeypatch.setenv("MINISIGN", MINISIGN)
    monkeypatch.setenv("RUNNER_TEMP", str(tmp_path))


def test_signed_artifacts_verify_and_a_tampered_one_is_rejected(
    real, tmp_path, release
):
    directory, _ = release
    pub, sec = generate_key(tmp_path)
    signing.sign(directory, pub, {"MINISIGN_SECRET_KEY": sec.read_text()}, VERSION)
    files = [p for p in directory.iterdir() if p.suffix != ".minisig"]
    signing.verify(files, pub)

    files[0].write_bytes(files[0].read_bytes() + b"tampered")
    with pytest.raises(signing.SigningError, match=files[0].name):
        signing.verify(files, pub)


def test_a_signature_from_another_key_is_rejected(real, tmp_path, release):
    directory, _ = release
    pub, sec = generate_key(tmp_path)
    signing.sign(directory, pub, {"MINISIGN_SECRET_KEY": sec.read_text()}, VERSION)
    other = tmp_path / "other"
    other.mkdir()
    other_pub, _ = generate_key(other)
    files = [p for p in directory.iterdir() if p.suffix != ".minisig"]
    with pytest.raises(signing.SigningError):
        signing.verify(files, other_pub)


def test_the_documented_user_command_verifies_a_signed_file(real, tmp_path, release):
    directory, _ = release
    pub, sec = generate_key(tmp_path)
    signing.sign(directory, pub, {"MINISIGN_SECRET_KEY": sec.read_text()}, VERSION)
    artifact = directory / "slopvac-x86_64.tar.gz"
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


def read_signature(artifact: Path) -> list[str]:
    return artifact.with_name(artifact.name + ".minisig").read_text().splitlines()


def test_every_signature_carries_the_trusted_comment_for_its_file(
    real, tmp_path, release
):
    directory, _ = release
    pub, sec = generate_key(tmp_path)
    signing.sign(directory, pub, {"MINISIGN_SECRET_KEY": sec.read_text()}, "3.1.0-rc.2")
    for name in ("slopvac-x86_64.tar.gz", "sha256.sum"):
        trusted = read_signature(directory / name)[2]
        assert (
            trusted
            == f"trusted comment: slopvac-release file:{name} version:3.1.0-rc.2"
        )
        # minisign prints the trusted comment only after the global signature verified.
        out = subprocess.run(
            [MINISIGN, "-Vm", str(directory / name), "-p", str(pub)],
            capture_output=True,
            text=True,
            check=True,
        )
        assert (
            f"Trusted comment: slopvac-release file:{name} version:3.1.0-rc.2"
            in out.stdout
        )


def test_a_trusted_comment_edited_after_signing_fails_verification(
    real, tmp_path, release
):
    directory, _ = release
    pub, sec = generate_key(tmp_path)
    signing.sign(directory, pub, {"MINISIGN_SECRET_KEY": sec.read_text()}, "3.1.0")
    artifact = directory / "slopvac-x86_64.tar.gz"
    signature = artifact.with_name(artifact.name + ".minisig")
    signature.write_text(
        signature.read_text().replace("version:3.1.0", "version:3.0.0")
    )
    with pytest.raises(signing.SigningError, match=artifact.name):
        signing.verify([artifact], pub)


@pytest.mark.parametrize(
    "version",
    ["", "v3.1.0", "3.1", "3.1.0 file:x", "3.1.0-rc.0", "3.1.0\nx", "3.1.0+b"],
)
def test_a_version_that_is_not_a_release_version_is_refused(release, version):
    directory, pub = release
    with pytest.raises(signing.SigningError, match="release version"):
        signing.sign(directory, pub, {"MINISIGN_SECRET_KEY": SECRET}, version)
    assert list(directory.glob("*.minisig")) == []


def test_the_command_line_requires_a_version(release):
    directory, pub = release
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--pubkey", str(pub), "sign", str(directory)],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 2
    assert "--version" in result.stderr


@pytest.mark.parametrize(
    "name",
    [
        "slopvac linux.tar.gz",
        "slopvac\tlinux.tar.gz",
        "slopvac\nlinux.tar.gz",
        "slopvac\rlinux.tar.gz",
        "slopvac-\u00e9.tar.gz",
        "slopvac-\u4e2d.tar.gz",
        "slopvac\u00a0linux.tar.gz",
    ],
)
def test_a_file_name_that_cannot_go_in_a_trusted_comment_stops_signing_first(
    fake, release, name
):
    directory, pub = release
    (directory / name).write_bytes(b"x")
    with pytest.raises(signing.SigningError, match="trusted comment"):
        run_sign(directory, pub, MINISIGN_SECRET_KEY=SECRET)
    # Nothing was signed, not even the valid files that sort before the bad one.
    assert list(directory.glob("*.minisig")) == []
    # The fake minisign logs every call; no log means it never ran.
    assert not fake.exists()


# --- verify-dir: the upload list is exactly the verified payloads and their sidecars ---


@pytest.fixture
def signed(fake, release):
    directory, pub = release
    for payload in list(directory.iterdir()):
        (directory / (payload.name + ".minisig")).write_text("sig")
    return directory, pub


@POSIX_ONLY
def test_the_upload_list_holds_each_verified_payload_and_its_sidecar(signed):
    directory, pub = signed
    listed = signing.verify_directory(directory, pub)
    assert [p.name for p in listed] == [
        "sha256.sum",
        "sha256.sum.minisig",
        "slopvac-x86_64.tar.gz",
        "slopvac-x86_64.tar.gz.minisig",
    ]


@POSIX_ONLY
def test_an_orphan_signature_fails_and_lists_nothing(signed, tmp_path):
    directory, pub = signed
    (directory / "extra.minisig").write_text("sig")
    out = tmp_path / "upload-list"
    code = signing.main(
        ["--pubkey", str(pub), "verify-dir", str(directory), "--list", str(out)]
    )
    assert code == 1
    assert not out.exists()


@POSIX_ONLY
def test_a_payload_without_a_signature_fails(signed):
    directory, pub = signed
    (directory / "slopvac-x86_64.tar.gz.minisig").unlink()
    with pytest.raises(signing.SigningError, match="slopvac-x86_64.tar.gz"):
        signing.verify_directory(directory, pub)


@POSIX_ONLY
def test_an_entry_that_is_not_a_regular_file_fails(signed):
    directory, pub = signed
    (directory / "nested").mkdir()
    with pytest.raises(signing.SigningError, match="nested"):
        signing.verify_directory(directory, pub)


@POSIX_ONLY
def test_the_list_file_is_nul_separated_for_xargs(signed, tmp_path):
    directory, pub = signed
    out = tmp_path / "upload-list"
    code = signing.main(
        ["--pubkey", str(pub), "verify-dir", str(directory), "--list", str(out)]
    )
    assert code == 0
    names = out.read_bytes().split(b"\0")
    assert names[-1] == b""
    assert len(names) == 5
    assert all(n.startswith(str(directory).encode()) for n in names[:-1])
