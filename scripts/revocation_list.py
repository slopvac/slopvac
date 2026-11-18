#!/usr/bin/env python3
"""Build, sign and check the empty first-party revocation list.

    revocation_list.py render [--days N] [--now TIME]
    revocation_list.py build DIR [--days N] [--now TIME]
    revocation_list.py verify DIR
    revocation_list.py check-newer CANDIDATE CURRENT [--first-publication]
    revocation_list.py fetch-published IMAGE DIR [--first-publication]

``fetch-published`` pulls the published list into DIR. Only a clear "manifest not
found" means nothing is published, and only ``--first-publication`` accepts that
(an existing list is judged by check-newer). Every other failure exits 1.

``check-newer`` stops a push unless the candidate's ``issued`` is strictly later
than the published list's (CURRENT, which may not exist on the first run). A
published list that does not verify is replaced only with ``--first-publication``,
which logs a warning; that flag never replaces a list that verifies.
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
import subprocess
import sys
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
    secret_key_file,
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


def check_list(data: bytes, now: datetime, *, require_fresh: bool = True) -> datetime:
    """Refuse anything the CLI would refuse, or a list that is already expired.

    Returns ``issued``.
    """
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
    # `type(..) is int` rejects true (a bool) and 1.0 (a float), which compare equal to 1.
    if type(document["schema_version"]) is not int or document["schema_version"] != 1:
        raise SigningError("not a version 1 revocation list")
    if document["type"] != "revocations":
        raise SigningError("not a version 1 revocation list")
    issued, expires = _parse(str(document["issued"])), _parse(str(document["expires"]))
    if expires <= issued or expires - issued > timedelta(days=MAX_VALID_DAYS):
        raise SigningError(
            f"expires must be after issued by at most {MAX_VALID_DAYS} days"
        )
    if require_fresh and expires <= now:
        raise SigningError("the list has already expired")
    _sorted_unique(document["keys"], _KEY_ID, "keys")
    _sorted_unique(document["digests"], _DIGEST, "digests")
    return issued


def check_prehashed(signature: Path) -> None:
    # A signature that is not UTF-8 is a verification failure, not a crash. A read
    # error (OSError) is not caught: that is a filesystem fault and must fail closed.
    try:
        lines = signature.read_bytes().decode("utf-8").splitlines()
        raw = base64.b64decode(lines[1], validate=True)
    except (IndexError, ValueError) as err:
        raise SigningError(f"{signature.name} is not a minisign signature") from err
    if raw[:2] != PREHASHED:
        raise SigningError(f"{signature.name} is not a prehashed signature")


def verify(
    directory: Path, pubkey: Path, now: datetime, *, require_fresh: bool = True
) -> datetime:
    """The gate before a push: list, mode and signature. Returns ``issued``.

    ``require_fresh=False`` accepts a list past its ``expires``; ``check-newer``
    uses it for the published list, which a lapsed list must not block.
    """
    file = directory / LIST_NAME
    if not file.is_file():
        raise SigningError(f"{file} is missing")
    issued = check_list(file.read_bytes(), now, require_fresh=require_fresh)
    signature = file.with_name(file.name + SIGNATURE_SUFFIX)
    if not signature.is_file():
        raise SigningError(f"no signature for {file.name}")
    check_prehashed(signature)
    verify_signatures([file], pubkey)
    return issued


# Registry answers that mean the tag has no manifest. Nothing else does: a bare
# 404 can also come from a layer, and auth, network and 5xx errors never mean absent.
_MANIFEST_ABSENT = re.compile(
    r"manifest[_ ]unknown|name[_ ]unknown|^Error: \S+: not found$",
    re.IGNORECASE | re.MULTILINE,
)


def _oras(args: list[str]) -> subprocess.CompletedProcess[str]:
    binary = os.environ.get("ORAS") or "oras"
    if shutil.which(binary) is None:
        raise SigningError(f"oras binary not found: {binary}")
    return subprocess.run(
        [binary, *args],
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        check=False,
    )


def fetch_published(image: str, directory: Path, *, first_publication: bool) -> None:
    """Pull the published list into ``directory``, or confirm that none exists.

    The manifest is resolved first, apart from its layers. Only a clear
    "manifest not found" for the tag means absent, and then only
    ``first_publication`` may go on: ``directory`` is left uncreated. A list that
    exists is pulled and judged by ``check_newer``. Every other failure raises, so
    the push stops: an auth or network error, a 5xx, a layer that cannot be
    fetched, or a manifest that is not JSON.
    """
    manifest = _oras(["manifest", "fetch", image])
    if manifest.returncode != 0:
        if _MANIFEST_ABSENT.search(manifest.stderr):
            if first_publication:
                return
            raise SigningError(
                f"nothing is published at {image}; "
                "run by hand with first_publication to publish the first list"
            )
        raise SigningError(
            f"could not resolve the manifest of {image}: {manifest.stderr.strip()}"
        )
    try:
        document = json.loads(manifest.stdout)
    except ValueError as err:
        raise SigningError(f"the manifest of {image} is not JSON") from err
    if not isinstance(document, dict) or not isinstance(document.get("layers"), list):
        raise SigningError(f"the manifest of {image} has no layers")
    pulled = _oras(["pull", image, "--output", str(directory)])
    if pulled.returncode != 0:
        raise SigningError(
            f"could not pull the layers of {image}: {pulled.stderr.strip()}"
        )


# Keys that may have signed the *published* list, besides the current committed key:
# `keys/slopvac-release-<KEYID>.pub`, the previous or next key of a rotation. The
# candidate is always signed with the current key (`keys/slopvac-release.pub`), and a
# rotation release is signed with the current key too (docs/release/distribution.md).
# To trust a previous key through a rotation, keep its file in `keys/`; delete it to
# stop trusting it. Clients verify only against the compiled-in key, so a published
# list from another key is never usable and the next push replaces it.
PUBLISHED_KEY_GLOB = "slopvac-release-[0-9A-F]*.pub"


def published_keys(pubkey: Path) -> list[Path]:
    """The keys that may have signed the published list: current first."""
    others = sorted(p for p in pubkey.parent.glob(PUBLISHED_KEY_GLOB) if p != pubkey)
    return [pubkey, *others]


def _verify_published(current: Path, pubkey: Path, now: datetime) -> datetime:
    failures = []
    for key in published_keys(pubkey):
        try:
            return verify(current, key, now, require_fresh=False)
        except SigningError as err:
            failures.append(f"{key.name}: {err}")
    raise SigningError("; ".join(failures))


def _workflow_text(text: str) -> str:
    """One line, safe inside a ``::warning::`` workflow command."""
    return " ".join(text.replace("%", "%25").split())[:500]


def check_newer(
    candidate: Path,
    current: Path,
    pubkey: Path,
    now: datetime,
    *,
    first_publication: bool = False,
) -> None:
    """Refuse to replace the published list with one that is not strictly newer.

    The CLI refuses an older ``issued`` than the one it cached, so an older
    artifact pushed late (a retried job) would break clients. The candidate must
    verify with the current key. The published list may verify with the current
    key or a trusted previous or next key (``published_keys``). A ``current``
    directory that does not exist means nothing is published yet.

    A published list that does not verify is not overwritten, except in a first
    publication: that run logs a warning and replaces it. A first publication
    never replaces a list that verifies.
    """
    issued = verify(candidate, pubkey, now)
    if not current.exists():
        return
    try:
        published = _verify_published(current, pubkey, now)
    except SigningError as err:
        if not first_publication:
            raise SigningError(
                f"the published list does not verify, so it is not replaced: {err}"
            ) from err
        print(
            "::warning::replacing an existing list that failed verification: "
            + _workflow_text(str(err))
        )
        return
    if first_publication:
        raise SigningError(
            f"a valid list (issued {_format(published)}) is already published; "
            "first_publication is only for the first run or a list that fails verification"
        )
    if issued <= published:
        raise SigningError(
            f"the candidate (issued {_format(issued)}) is not newer than the "
            f"published list (issued {_format(published)})"
        )


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

    with secret_key_file(secret, environ.get("RUNNER_TEMP")) as key_file:
        # Default mode is prehashed. -W reads the key as unencrypted, so an
        # encrypted key fails instead of prompting.
        argv = [binary, "-S", "-s", str(key_file), "-m", str(file)]
        _run([*argv, "-x", str(signature), "-W"])
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
    newer = commands.add_parser("check-newer")
    newer.add_argument("--first-publication", action="store_true")
    newer.add_argument("candidate", type=Path)
    newer.add_argument("current", type=Path, help="may not exist: nothing published")
    fetch = commands.add_parser("fetch-published")
    fetch.add_argument("image")
    fetch.add_argument("directory", type=Path)
    fetch.add_argument("--first-publication", action="store_true")
    args = parser.parse_args(argv)
    try:
        now = _parse(args.now) if getattr(args, "now", None) else datetime.now(UTC)
        if args.command == "render":
            sys.stdout.buffer.write(render(now, args.days))
        elif args.command == "fetch-published":
            fetch_published(
                args.image, args.directory, first_publication=args.first_publication
            )
            print("fetched the published list" if args.directory.exists() else "none")
        elif args.command == "build":
            build(args.directory, args.pubkey, dict(os.environ), now, args.days)
            print(f"signed and verified {LIST_NAME}")
        elif args.command == "check-newer":
            check_newer(
                args.candidate,
                args.current,
                args.pubkey,
                now,
                first_publication=args.first_publication,
            )
            print("the candidate is newer than the published list")
        else:
            verify(args.directory, args.pubkey, now)
            print(f"verified {LIST_NAME}")
    except SigningError as err:
        print(f"error: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
