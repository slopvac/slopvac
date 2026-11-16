"""Download the eval corpus into a cache directory and verify every hash.

    python -m evals.fetch                    # upstream documents not committed
    python -m evals.fetch --all              # also re-download committed ones
    python -m evals.fetch --only 'genre=diagrams' --only 'origin=human'

A download whose SHA-256 differs from the manifest is discarded, never written
to the cache, and makes the run exit 1. Committed documents are checked
against the manifest on every run without network access.
"""

from __future__ import annotations

import argparse
import fnmatch
import os
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import Any

from . import corpuslib as lib

FETCHABLE = ("github-file", "github-commit", "github-pr", "json-field")


def selected(doc: dict[str, Any], filters: list[str]) -> bool:
    """`--only` takes `field=glob`; filters on one field OR, across fields AND."""
    by_field: dict[str, list[str]] = {}
    for item in filters:
        field, _, pattern = item.partition("=")
        by_field.setdefault(field, []).append(pattern)
    return all(
        any(fnmatch.fnmatch(str(doc.get(field, "")), p) for p in patterns)
        for field, patterns in by_field.items()
    )


def verify_committed(doc: dict[str, Any]) -> str | None:
    path = lib.committed_file(doc)
    if not path.is_file():
        return f"{doc['id']}: committed file {doc['path']} is missing"
    actual = lib.sha256_text(path.read_text(encoding="utf-8"))
    if actual != doc["sha256"]:
        return f"{doc['id']}: committed file hash {actual[:12]} != manifest {doc['sha256'][:12]}"
    return None


def fetch_one(
    doc: dict[str, Any], cache: Path, token: str | None, force: bool
) -> tuple[str, str]:
    """Return (status, message). Status is one of ok, cached, mismatch, error."""
    target = lib.cache_file(cache, doc["id"])
    if target.exists() and not force:
        if lib.sha256_text(target.read_text(encoding="utf-8")) == doc["sha256"]:
            return "cached", doc["id"]
        target.unlink()  # a stale or damaged cache entry is refetched
    try:
        text = lib.fetch_text(doc, token, cache / "raw")
    except (lib.FetchError, KeyError, IndexError, ValueError) as exc:
        return "error", f"{doc['id']}: {exc}"
    actual = lib.sha256_text(text)
    if actual != doc["sha256"]:
        return (
            "mismatch",
            f"{doc['id']}: upstream hash {actual[:12]} != manifest {doc['sha256'][:12]}",
        )
    target.parent.mkdir(parents=True, exist_ok=True)
    tmp = target.with_suffix(".tmp")
    tmp.write_text(lib.normalise(text), encoding="utf-8")
    tmp.replace(target)
    return "ok", doc["id"]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument(
        "--cache",
        type=Path,
        default=Path(os.environ.get("SLOPVAC_EVALS_CACHE", lib.DEFAULT_CACHE)),
        help="cache directory (default: evals/.cache or $SLOPVAC_EVALS_CACHE)",
    )
    parser.add_argument(
        "--all", action="store_true", help="re-download committed documents too"
    )
    parser.add_argument("--force", action="store_true", help="ignore cache hits")
    parser.add_argument("--only", action="append", default=[], metavar="FIELD=GLOB")
    parser.add_argument("--jobs", type=int, default=8)
    args = parser.parse_args(argv)

    manifest = lib.load_manifest()
    errors = lib.validate_manifest(manifest)
    if errors:
        print("\n".join(errors[:20]), file=sys.stderr)
        return 1

    docs = [d for d in manifest["documents"] if selected(d, args.only)]
    failures: list[str] = []
    for doc in docs:
        if doc["committed"]:
            problem = verify_committed(doc)
            if problem:
                failures.append(problem)

    todo = [
        d
        for d in docs
        if d["source"]["type"] in FETCHABLE and (args.all or not d["committed"])
    ]
    token = lib.github_token()
    counts = {"ok": 0, "cached": 0, "mismatch": 0, "error": 0}
    with ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        for status, message in pool.map(
            lambda d: fetch_one(d, args.cache, token, args.force), todo
        ):
            counts[status] += 1
            if status in ("mismatch", "error"):
                failures.append(message)
    print(
        f"{len(docs)} selected, {len(todo)} fetched: "
        f"{counts['ok']} downloaded, {counts['cached']} cached, "
        f"{counts['mismatch']} mismatched, {counts['error']} failed"
    )
    for failure in failures:
        print(f"FAIL {failure}", file=sys.stderr)
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
