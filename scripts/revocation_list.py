#!/usr/bin/env python3
"""Build, sign and check the empty first-party revocation list.

    revocation_list.py render [--days N] [--now TIME]
    revocation_list.py build DIR [--days N] [--now TIME]
    revocation_list.py verify DIR

``render`` prints the canonical list. ``build`` writes ``revocations.json`` and
``revocations.json.minisig`` into DIR, signs with ``MINISIGN_SECRET_KEY`` and checks
the result as ``verify`` does. ``verify`` is the only gate before a push: it checks
the list and its signature against ``keys/slopvac-release.pub`` and exits 1 on any
mismatch.

The checks mirror what the CLI accepts (``crates/slopvac-plugins/src/revoke.rs``,
``RevocationList::verify``): strict canonical JSON (sorted keys, no white space),
at most 1 MiB, ``schema_version`` 1, ``type`` ``revocations``, and ``expires`` later
than ``issued`` by at most 30 days. The signature must be minisign's prehashed
mode. A Rust test in ``crates/slopvac-plugins/tests/revoke.rs`` runs ``render`` and
verifies the output with the real verifier, so the two cannot drift.

``issued`` is the current UTC time to the second, so each run orders after the last
one: the CLI refuses an older ``issued`` as a rollback. The list is valid for 30
days by default and the workflow signs a new one every week.

Environment of ``build``: ``MINISIGN_SECRET_KEY`` holds the secret key file text
(no password). It goes to a mode 600 file that is removed afterwards and is never
printed. ``MINISIGN`` names the minisign binary.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import re
import shutil
import sys
import tempfile
from collections.abc import Sequence
from datetime import UTC, datetime, timedelta
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from release_signing import (
    PUBKEY,
    SIGNATURE_SUFFIX,
    SigningError,
    _run,
    minisign_binary,
)
from release_signing import (
    verify as verify_signatures,
)

LIST_NAME = "revocations.json"
MAX_LIST_BYTES = 1 << 20
MAX_VALID_DAYS = 30
TIME_FORMAT = "%Y-%m-%dT%H:%M:%SZ"
_TIME = re.compile(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z")
_KEY_ID = re.compile(r"[0-9A-F]{16}")
_DIGEST = re.compile(r"sha256:[0-9a-f]{64}")
_FIELDS = {"schema_version", "type", "issued", "expires", "keys", "digests"}
# The two-byte signature algorithm of a minisign signature: "ED" is prehashed,
# "Ed" is the legacy mode that the CLI refuses.
PREHASHED = b"ED"


def _format(moment: datetime) -> str:
    return moment.astimezone(UTC).strftime(TIME_FORMAT)


def _parse(text: str) -> datetime:
    if _TIME.fullmatch(text) is None:
        raise SigningError(f"not a YYYY-MM-DDTHH:MM:SSZ time: {text!r}")
    try:
        return datetime.strptime(text, TIME_FORMAT).replace(tzinfo=UTC)
    except ValueError as err:
        raise SigningError(f"not a calendar time: {text!r}") from err


def canonical(document: dict[str, object]) -> bytes:
    """Sorted keys, no white space, as the verifier re-encodes it."""
    return json.dumps(
        document, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode("utf-8")


def render(now: datetime, days: int = MAX_VALID_DAYS) -> bytes:
    """The empty list issued at ``now``, valid for ``days`` days."""
    if not 1 <= days <= MAX_VALID_DAYS:
        raise SigningError(f"--days must be 1 to {MAX_VALID_DAYS}, got {days}")
    issued = now.astimezone(UTC).replace(microsecond=0)
    data = canonical(
        {
            "schema_version": 1,
            "type": "revocations",
            "issued": _format(issued),
            "expires": _format(issued + timedelta(days=days)),
            "keys": [],
            "digests": [],
        }
    )
    check_list(data, now)
    return data


def _sorted_unique(values: object, pattern: re.Pattern[str], what: str) -> None:
    if not isinstance(values, list) or not all(
        isinstance(v, str) and pattern.fullmatch(v) for v in values
    ):
        raise SigningError(f"{what} must be a list of well-formed entries")
    if values != sorted(set(values)):
        raise SigningError(f"{what} must be sorted and unique")


def check_list(data: bytes, now: datetime) -> None:
    """Refuse anything the CLI would refuse, or a list that is already expired."""
    if len(data) > MAX_LIST_BYTES:
        raise SigningError(f"the list is larger than {MAX_LIST_BYTES} bytes")
    try:
        document = json.loads(data)
    except ValueError as err:
        raise SigningError(f"the list is not JSON: {err}") from err
    if not isinstance(document, dict) or set(document) != _FIELDS:
        raise SigningError(f"the list must have exactly the fields {sorted(_FIELDS)}")
    if canonical(document) != data:
        raise SigningError("the list is not canonical JSON")
    if document["schema_version"] != 1 or document["type"] != "revocations":
        raise SigningError("not a version 1 revocation list")
    issued, expires = _parse(str(document["issued"])), _parse(str(document["expires"]))
    if expires <= issued or expires - issued > timedelta(days=MAX_VALID_DAYS):
        raise SigningError(
            f"expires must be after issued by at most {MAX_VALID_DAYS} days"
        )
    if expires <= now:
        raise SigningError("the list has already expired")
    _sorted_unique(document["keys"], _KEY_ID, "keys")
    _sorted_unique(document["digests"], _DIGEST, "digests")


def check_prehashed(signature: Path) -> None:
    lines = signature.read_text(encoding="utf-8").splitlines()
    try:
        raw = base64.b64decode(lines[1], validate=True)
    except (IndexError, ValueError) as err:
        raise SigningError(f"{signature.name} is not a minisign signature") from err
    if raw[:2] != PREHASHED:
        raise SigningError(f"{signature.name} is not a prehashed signature")


def verify(directory: Path, pubkey: Path, now: datetime) -> None:
    """The gate before a push: list, mode and signature."""
    file = directory / LIST_NAME
    if not file.is_file():
        raise SigningError(f"{file} is missing")
    check_list(file.read_bytes(), now)
    signature = file.with_name(file.name + SIGNATURE_SUFFIX)
    if not signature.is_file():
        raise SigningError(f"no signature for {file.name}")
    check_prehashed(signature)
    verify_signatures([file], pubkey)


def build(
    directory: Path,
    pubkey: Path,
    environ: dict[str, str],
    now: datetime,
    days: int,
) -> None:
    secret = environ.get("MINISIGN_SECRET_KEY", "")
    if not secret.strip():
        raise SigningError("MINISIGN_SECRET_KEY is not set")
    data = render(now, days)
    binary = minisign_binary()
    directory.mkdir(parents=True, exist_ok=True)
    file = directory / LIST_NAME
    file.write_bytes(data)
    signature = file.with_name(file.name + SIGNATURE_SUFFIX)

    key_dir = Path(tempfile.mkdtemp(prefix="minisign-", dir=environ.get("RUNNER_TEMP")))
    try:
        key_file = key_dir / "release.key"
        descriptor = os.open(key_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            handle.write(secret.rstrip("\n") + "\n")
        # Default mode is prehashed. -W reads the key as unencrypted, so an
        # encrypted key fails instead of prompting.
        argv = [binary, "-S", "-s", str(key_file), "-m", str(file)]
        _run([*argv, "-x", str(signature), "-W"])
    finally:
        shutil.rmtree(key_dir, ignore_errors=True)
    verify(directory, pubkey, now)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--pubkey", type=Path, default=PUBKEY)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("render", "build"):
        sub = commands.add_parser(name)
        sub.add_argument("--days", type=int, default=MAX_VALID_DAYS)
        sub.add_argument("--now", help="override the clock (YYYY-MM-DDTHH:MM:SSZ)")
        if name == "build":
            sub.add_argument("directory", type=Path)
    commands.add_parser("verify").add_argument("directory", type=Path)
    args = parser.parse_args(argv)
    try:
        now = _parse(args.now) if getattr(args, "now", None) else datetime.now(UTC)
        if args.command == "render":
            sys.stdout.buffer.write(render(now, args.days))
        elif args.command == "build":
            build(args.directory, args.pubkey, dict(os.environ), now, args.days)
            print(f"signed and verified {LIST_NAME}")
        else:
            verify(args.directory, args.pubkey, now)
            print(f"verified {LIST_NAME}")
    except SigningError as err:
        print(f"error: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
