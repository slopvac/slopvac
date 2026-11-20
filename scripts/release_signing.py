#!/usr/bin/env python3
"""Sign and verify slopvac release artifacts with minisign.

    release_signing.py require-key
    release_signing.py sign DIR --version VERSION
    release_signing.py verify FILE [FILE ...]

``sign`` writes ``<file>.minisig`` next to every regular file in DIR, then checks
each signature against the committed public key before it returns. The workflows
run it before they upload to the release, so a release never ships unsigned.

Every signature carries the trusted comment ``slopvac-release file:<name>
version:<VERSION>``. Minisign's global signature covers the trusted comment, so a
verifier that checks it knows which file of which release the key signed.
``slopvac self update`` requires it to match the archive it asked for
(``crates/slopvac-update/src/verify.rs`` defines the same format). VERSION is
``X.Y.Z`` or ``X.Y.Z-rc.N`` without the ``v``.

Environment of ``sign`` (set only on the signing step):

* ``MINISIGN_SECRET_KEY``: contents of the minisign secret key file. The release key
  has no password, so signing never prompts; an encrypted key fails instead.

The secret key goes to a mode 600 file under ``RUNNER_TEMP`` that is removed
afterwards. Nothing here prints it.

``MINISIGN`` names the minisign binary (default ``minisign`` on PATH). ``--pubkey``
overrides ``keys/slopvac-release.pub``. Users verify a download with
``minisign -Vm <file> -p keys/slopvac-release.pub``; ``verify`` does the same for
many files.
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
from collections.abc import Iterator, Sequence
from contextlib import contextmanager
from pathlib import Path

PUBKEY = Path(__file__).resolve().parents[1] / "keys" / "slopvac-release.pub"
SIGNATURE_SUFFIX = ".minisig"
_NUM = r"(?:0|[1-9][0-9]*)"
_VERSION = re.compile(rf"{_NUM}\.{_NUM}\.{_NUM}(?:-rc\.[1-9][0-9]*)?")


class SigningError(Exception):
    """A signing or verification step failed; the release must not go on."""


def trusted_comment(name: str, version: str) -> str:
    """The authenticated identity of one release file."""
    if _VERSION.fullmatch(version) is None:
        raise SigningError(
            f"not a release version (expected X.Y.Z or X.Y.Z-rc.N): {version}"
        )
    if not re.fullmatch(r"[A-Za-z0-9._+-]+", name):
        raise SigningError(f"file name cannot go in a trusted comment: {name!r}")
    return f"slopvac-release file:{name} version:{version}"


def minisign_binary() -> str:
    binary = os.environ.get("MINISIGN") or "minisign"
    if shutil.which(binary) is None:
        raise SigningError(f"minisign binary not found: {binary}")
    return binary


def require_key(pubkey: Path) -> None:
    if not pubkey.is_file():
        raise SigningError(
            f"public key {pubkey} is missing; merge the key PR before releasing "
            "(docs/release/maintainer-setup.md)"
        )


def _run(argv: list[str]) -> None:
    # stdin is closed so minisign can never wait on a terminal prompt.
    result = subprocess.run(
        argv,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip()
        raise SigningError(f"{' '.join(argv[:2])} failed: {detail}")


def verify(files: Sequence[Path], pubkey: Path) -> None:
    require_key(pubkey)
    binary = minisign_binary()
    for file in files:
        signature = file.with_name(file.name + SIGNATURE_SUFFIX)
        if not signature.is_file():
            raise SigningError(f"no signature for {file.name}")
        try:
            _run(
                [binary, "-V", "-q", "-p", str(pubkey), "-m", str(file)]
                + ["-x", str(signature)]
            )
        except SigningError as err:
            raise SigningError(f"signature check failed for {file.name}: {err}")


def artifacts(directory: Path) -> list[Path]:
    if not directory.is_dir():
        raise SigningError(f"{directory} is not a directory")
    entries = sorted(directory.iterdir())
    nested = [e.name for e in entries if not e.is_file()]
    if nested:
        raise SigningError(f"expected only files in {directory}, found: {nested}")
    files = [e for e in entries if not e.name.endswith(SIGNATURE_SUFFIX)]
    if not files:
        raise SigningError(f"no artifacts to sign in {directory}")
    return files


@contextmanager
def secret_key_file(secret: str, runner_temp: str | None) -> Iterator[Path]:
    """The secret key in a mode 600 file that is removed on exit, even on failure."""
    key_dir = Path(tempfile.mkdtemp(prefix="minisign-", dir=runner_temp))
    try:
        key_file = key_dir / "release.key"
        descriptor = os.open(key_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            handle.write(secret.rstrip("\n") + "\n")
        yield key_file
    finally:
        shutil.rmtree(key_dir, ignore_errors=True)


def sign(
    directory: Path, pubkey: Path, environ: dict[str, str], version: str
) -> list[Path]:
    secret = environ.get("MINISIGN_SECRET_KEY", "")
    if not secret.strip():
        raise SigningError("MINISIGN_SECRET_KEY is not set")
    require_key(pubkey)
    files = artifacts(directory)
    comments = {f: trusted_comment(f.name, version) for f in files}
    binary = minisign_binary()

    with secret_key_file(secret, environ.get("RUNNER_TEMP")) as key_file:
        for file in files:
            signature = file.with_name(file.name + SIGNATURE_SUFFIX)
            argv = [binary, "-S", "-s", str(key_file), "-m", str(file)]
            argv += ["-x", str(signature), "-t", comments[file]]
            # -W reads the key as unencrypted; an encrypted key then fails instead of prompting.
            _run([*argv, "-W"])

    verify(files, pubkey)
    return [f.with_name(f.name + SIGNATURE_SUFFIX) for f in files]


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--pubkey", type=Path, default=PUBKEY)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("require-key")
    sign_parser = commands.add_parser("sign")
    sign_parser.add_argument("directory", type=Path)
    sign_parser.add_argument("--version", required=True)
    commands.add_parser("verify").add_argument("files", type=Path, nargs="+")
    args = parser.parse_args(argv)
    try:
        if args.command == "require-key":
            require_key(args.pubkey)
        elif args.command == "sign":
            for signature in sign(
                args.directory, args.pubkey, dict(os.environ), args.version
            ):
                print(f"signed {signature.name}")
        else:
            verify(args.files, args.pubkey)
            print(f"verified {len(args.files)} file(s)")
    except SigningError as err:
        print(f"error: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
