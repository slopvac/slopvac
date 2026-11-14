#!/usr/bin/env python3
"""Validate the slopvac 3.0 fork-review artifact for the privileged consumer.

The unprivileged pull request workflow (docs/ci/github-action.md, "Pull
requests from forks") uploads an artifact with exactly three files::

    findings.rdjsonl   reviewdog Diagnostic Format, one JSON object per line
    pr-number          decimal pull request number
    head-sha           40-hex head commit the report was made for

``.github/workflows/reviewdog.yml`` runs this script from the default branch with
a write token. The archive is untrusted input: fork authors control every byte.
The script never executes or checks out anything from it. It checks the archive
against fixed limits, writes only a fixed file name into a fresh directory, and
binds the artifact to the triggering run: the pull request number and the head
commit come from the ``workflow_run`` event and the API, never from the
artifact, and the artifact's own claims must equal them.

Usage: ``fork_review.py extract ARCHIVE OUT_DIR`` with ``EXPECTED_PR`` and
``EXPECTED_SHA`` in the environment. Exit 0 on success, 1 on any rejection.
"""

from __future__ import annotations

import json
import os
import re
import stat
import sys
import zipfile
from collections.abc import Mapping, Sequence
from pathlib import Path, PurePosixPath
from typing import Any

REPORT = "findings.rdjsonl"
PR_NUMBER = "pr-number"
HEAD_SHA = "head-sha"
ENTRIES = frozenset({REPORT, PR_NUMBER, HEAD_SHA})

MAX_ARCHIVE_BYTES = 1_000_000
MAX_REPORT_BYTES = 2_000_000
MAX_SMALL_BYTES = 64
MAX_TOTAL_BYTES = MAX_REPORT_BYTES + 2 * MAX_SMALL_BYTES
MAX_FILES = len(ENTRIES)
MAX_DIAGNOSTICS = 2_000
MAX_STRING = 10_000
MAX_LIST = 50
MAX_COORDINATE = 10_000_000
CHUNK = 8192
# STORED and DEFLATE only: CPython decompresses BZIP2 and LZMA members with no
# output bound before it truncates to the declared size (CWE-409).
ALLOWED_COMPRESSION = (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED)

_SHA = re.compile(r"[0-9a-f]{40}")
_NUMBER = re.compile(r"[1-9][0-9]{0,8}")
_SEVERITIES = {"UNKNOWN_SEVERITY", "ERROR", "WARNING", "INFO"}

# reviewdog's Diagnostic (rdf.proto): every top-level field it defines.
_DIAGNOSTIC_FIELDS = {
    "message",
    "location",
    "severity",
    "source",
    "code",
    "suggestions",
    "original_output",
    "related_locations",
}


class Rejected(Exception):
    """The artifact is not acceptable; the message goes to the job log."""


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise Rejected(message)


def _is_int(value: object, *, minimum: int = 0) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value >= minimum


def _string(value: object, what: str, *, required: bool = True) -> str:
    ok = isinstance(value, str) and len(value) <= MAX_STRING and "\0" not in value
    _require(ok, f"{what} is not a bounded string")
    assert isinstance(value, str)
    _require(bool(value) or not required, f"{what} is empty")
    return value


def _object(value: object, what: str, allowed: set[str]) -> dict[str, object]:
    _require(isinstance(value, dict), f"{what} is not an object")
    assert isinstance(value, dict)
    _require(set(value) <= allowed, f"{what} has unknown fields")
    return value


def _order(point: dict[str, object]) -> tuple[Any, Any]:
    return (point.get("line", 1), point.get("column", 1))


def _check_range(value: object, what: str) -> None:
    fields = _object(value, what, {"start", "end"})
    _require("start" in fields, f"{what} has no start")
    for key, position in fields.items():
        point = _object(position, f"{what}.{key}", {"line", "column"})
        _require(
            all(_is_int(v, minimum=1) and v <= MAX_COORDINATE for v in point.values()),
            f"{what}.{key} is invalid",
        )
    if "end" in fields:
        start, end = fields["start"], fields["end"]
        assert isinstance(start, dict) and isinstance(end, dict)
        _require(_order(end) >= _order(start), f"{what} ends before it starts")


def _check_location(value: object, what: str) -> None:
    fields = _object(value, what, {"path", "range"})
    raw = _string(fields.get("path"), f"{what}.path")
    # The path names a file in the review. Keep it relative and inside the
    # repository so a report cannot point anywhere else.
    path = PurePosixPath(raw)
    _require(
        not path.is_absolute() and ".." not in path.parts and "\\" not in raw,
        f"{what}.path {raw!r} is not a relative repository path",
    )
    if "range" in fields:
        _check_range(fields["range"], f"{what}.range")


def _check_named(value: object, what: str, name_key: str) -> None:
    fields = _object(value, what, {name_key, "url"})
    for key, item in fields.items():
        _string(item, f"{what}.{key}", required=False)


def _check_list(value: object, what: str) -> list[object]:
    _require(isinstance(value, list) and len(value) <= MAX_LIST, f"{what} is invalid")
    assert isinstance(value, list)
    return value


def check_diagnostic(item: object, line_number: int) -> None:
    """Raise Rejected unless ``item`` is a reviewdog Diagnostic."""
    where = f"line {line_number}"
    fields = _object(item, where, _DIAGNOSTIC_FIELDS)
    _string(fields.get("message"), f"{where} message")
    _require("location" in fields, f"{where} has no location")
    _check_location(fields["location"], f"{where} location")
    if "severity" in fields:
        severity = fields["severity"]
        _require(
            isinstance(severity, str) and severity in _SEVERITIES,
            f"{where} severity is invalid",
        )
    if "source" in fields:
        _check_named(fields["source"], f"{where} source", "name")
    if "code" in fields:
        _check_named(fields["code"], f"{where} code", "value")
    if "original_output" in fields:
        _string(fields["original_output"], f"{where} original_output", required=False)
    for suggestion in _check_list(
        fields.get("suggestions", []), f"{where} suggestions"
    ):
        parts = _object(suggestion, f"{where} suggestion", {"range", "text"})
        _check_range(parts.get("range"), f"{where} suggestion range")
        _string(parts.get("text"), f"{where} suggestion text", required=False)
    for entry in _check_list(
        fields.get("related_locations", []), f"{where} related_locations"
    ):
        related = _object(entry, f"{where} related location", {"message", "location"})
        _check_location(related.get("location"), f"{where} related location")
        if "message" in related:
            _string(related["message"], f"{where} related message", required=False)


def check_report(data: bytes) -> int:
    """Validate the rdjsonl bytes and return the diagnostic count."""
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError as err:
        raise Rejected(f"{REPORT} is not UTF-8: {err}") from err
    count = 0
    # Split on "\n" only: str.splitlines also breaks on U+2028 and others, which
    # are legal inside a JSON string.
    for number, raw in enumerate(text.split("\n"), start=1):
        line = raw.removesuffix("\r")
        if not line.strip():
            continue
        count += 1
        _require(
            count <= MAX_DIAGNOSTICS,
            f"{REPORT} has more than {MAX_DIAGNOSTICS} diagnostics",
        )
        try:
            item = json.loads(line)
        except (json.JSONDecodeError, RecursionError) as err:
            raise Rejected(f"line {number} is not valid JSON: {err}") from err
        check_diagnostic(item, number)
    return count


def _entry_limit(name: str) -> int:
    return MAX_REPORT_BYTES if name == REPORT else MAX_SMALL_BYTES


def _read_bounded(zf: zipfile.ZipFile, info: zipfile.ZipInfo, limit: int) -> bytes:
    """Read a member in chunks under a running cap that ignores header sizes."""
    chunks: list[bytes] = []
    total = 0
    with zf.open(info) as handle:
        while chunk := handle.read(CHUNK):
            total += len(chunk)
            _require(
                total <= limit, f"entry {info.filename!r} expands past {limit} bytes"
            )
            chunks.append(chunk)
    return b"".join(chunks)


def read_entries(archive: Path) -> dict[str, bytes]:
    """Read the archive into memory under every size, count and name limit.

    Nothing is extracted by name: the three accepted names are fixed, so a
    traversal path, a symlink or a surprise entry is a rejection, not a write.
    """
    _require(archive.is_file() and not archive.is_symlink(), "archive is not a file")
    size = archive.stat().st_size
    _require(0 < size <= MAX_ARCHIVE_BYTES, f"archive is {size} bytes, over the cap")
    try:
        zf = zipfile.ZipFile(archive)
    except zipfile.BadZipFile as err:
        raise Rejected(f"archive is not a zip file: {err}") from err
    with zf:
        infos = zf.infolist()
        names = [info.filename for info in infos]
        _require(len(infos) <= MAX_FILES, f"archive has {len(infos)} entries")
        _require(len(set(names)) == len(names), "archive has duplicate entries")
        _require(
            set(names) == ENTRIES,
            f"archive entries must be exactly {sorted(ENTRIES)}, got {names!r}",
        )
        total = 0
        for info in infos:
            mode = info.external_attr >> 16
            _require(
                not info.is_dir() and stat.S_IFMT(mode) in (0, stat.S_IFREG),
                f"entry {info.filename!r} is not a regular file",
            )
            _require(
                info.file_size <= _entry_limit(info.filename),
                f"entry {info.filename!r} is {info.file_size} bytes, over the cap",
            )
            total += info.file_size
        _require(total <= MAX_TOTAL_BYTES, f"archive expands to {total} bytes")
        entries: dict[str, bytes] = {}
        for info in infos:
            # Read one byte past the limit so a lying header is still caught.
            _require(
                info.compress_type in ALLOWED_COMPRESSION,
                f"entry {info.filename!r} uses compression method {info.compress_type}",
            )
            try:
                data = _read_bounded(zf, info, _entry_limit(info.filename))
            except (zipfile.BadZipFile, NotImplementedError, RuntimeError) as err:
                raise Rejected(f"entry {info.filename!r} is unreadable: {err}") from err
            _require(
                len(data) == info.file_size,
                f"entry {info.filename!r} size does not match its header",
            )
            entries[info.filename] = data
    return entries


def validate(archive: Path, *, expected_pr: str, expected_sha: str) -> dict[str, bytes]:
    """Return the validated entries or raise Rejected.

    ``expected_pr`` and ``expected_sha`` must come from the workflow_run event
    and the API. The artifact's recorded values only have to agree with them.
    """
    _require(_NUMBER.fullmatch(expected_pr) is not None, "expected PR is invalid")
    _require(_SHA.fullmatch(expected_sha) is not None, "expected head is invalid")
    entries = read_entries(archive)
    claimed_pr = entries[PR_NUMBER].decode("ascii", "replace").strip()
    _require(claimed_pr == expected_pr, "artifact is for a different pull request")
    claimed_sha = entries[HEAD_SHA].decode("ascii", "replace").strip()
    _require(claimed_sha == expected_sha, "artifact was made for a different commit")
    check_report(entries[REPORT])
    return entries


def extract(archive: Path, out: Path, *, expected_pr: str, expected_sha: str) -> None:
    """Write the validated report to ``out/findings.rdjsonl`` and nothing else."""
    entries = validate(archive, expected_pr=expected_pr, expected_sha=expected_sha)
    out.mkdir(parents=True, exist_ok=False)
    # The name is fixed and the directory is new, so there is no path to
    # follow. O_EXCL and O_NOFOLLOW make a planted link fail loudly.
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    with os.fdopen(os.open(out / REPORT, flags, 0o600), "wb") as handle:
        handle.write(entries[REPORT])


def main(argv: Sequence[str], env: Mapping[str, str] = os.environ) -> int:
    if len(argv) != 4 or argv[1] != "extract":
        print("usage: fork_review.py extract ARCHIVE OUT_DIR", file=sys.stderr)
        return 2
    try:
        extract(
            Path(argv[2]),
            Path(argv[3]),
            expected_pr=env.get("EXPECTED_PR", ""),
            expected_sha=env.get("EXPECTED_SHA", ""),
        )
    except (Rejected, OSError) as err:
        print(f"::error title=fork review::{err}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
