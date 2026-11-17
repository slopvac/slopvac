"""Shared helpers for the eval corpus: manifest I/O, hashing, schema, fetching.

Standard library only, so the offline tests and the fetch script run anywhere
Python 3.11 does.
"""

from __future__ import annotations

import hashlib
import http.client
import json
import os
import re
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable
from pathlib import Path
from typing import Any

EVALS_DIR = Path(__file__).resolve().parent
MANIFEST_PATH = EVALS_DIR / "manifest.json"
DEFAULT_CACHE = EVALS_DIR / ".cache"

SCHEMA_VERSION = 1
# Human text must predate this instant. The first public ChatGPT release was on
# 2022-11-30; the cutoff leaves a month of margin.
HUMAN_CUTOFF = "2022-11-01"

GENRES = (
    "product",
    "reference",
    "tutorials",
    "changelogs",
    "readmes",
    "adrs",
    "prs",
    "commits",
    "ui-strings",
    "code-comments",
    "diagrams",
    # Chat answers have no documentation genre; only stated-llm uses this one.
    "assistant-answers",
)
ORIGINS = (
    "human",
    "stated-llm",
    "model-twin",
    "seeded-defect",
    "clean-control",
    "rewrite-pair",
)
# Licences whose terms allow redistribution in this Apache-2.0 repository with
# an attribution notice. Anything else stays in the cache.
PERMISSIVE = frozenset(
    {
        "MIT",
        "Apache-2.0",
        "BSD-2-Clause",
        "BSD-3-Clause",
        "ISC",
        "0BSD",
        "Unlicense",
        "CC0-1.0",
        "CC-BY-4.0",
        "MIT OR Apache-2.0",
        "MIT OR Unlicense",
        "Apache-2.0 OR MIT",
    }
)
# Committed copies stay small so the repository does not grow without bound.
MAX_COMMITTED_BYTES = 32 * 1024

SOURCE_TYPES = (
    "github-file",
    "github-commit",
    "github-pr",
    "json-field",
    "derived",
    "model-output",
)
FORMATS = (
    "markdown",
    "rst",
    "text",
    "json",
    "ini",
    "po",
    "yaml",
    "mermaid-markdown",
    "drawio",
    "python",
    "rust",
    "go",
    "javascript",
    "typescript",
    "c",
    "commit-message",
    "pull-request",
)
TWIN_STATUS = ("pending", "generated")

SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
ID_RE = re.compile(r"^[a-z0-9][a-z0-9._-]*$")
DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")


def normalise(text: str) -> str:
    """Line endings are the only change between upstream bytes and corpus bytes."""
    return text.replace("\r\n", "\n").replace("\r", "\n")


def sha256_text(text: str) -> str:
    return hashlib.sha256(normalise(text).encode("utf-8")).hexdigest()


def load_manifest(path: Path = MANIFEST_PATH) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def dump_manifest(manifest: dict[str, Any], path: Path = MANIFEST_PATH) -> None:
    manifest["documents"].sort(key=lambda d: d["id"])
    path.write_text(
        json.dumps(manifest, indent=1, ensure_ascii=False) + "\n", encoding="utf-8"
    )


def committed_file(doc: dict[str, Any]) -> Path:
    return EVALS_DIR / doc["path"]


def cache_file(cache: Path, doc_id: str) -> Path:
    return cache / "docs" / f"{doc_id}.txt"


# --- schema ----------------------------------------------------------------


def validate_manifest(manifest: dict[str, Any]) -> list[str]:
    """Return every schema violation; an empty list means the manifest is valid."""
    errors: list[str] = []
    if manifest.get("schema") != SCHEMA_VERSION:
        errors.append(f"schema must be {SCHEMA_VERSION}")
    repos = manifest.get("repos", {})
    docs = manifest.get("documents", [])
    seen: set[str] = set()
    for doc in docs:
        errors.extend(_validate_doc(doc, repos, seen))
    return errors


def _need(
    doc: dict[str, Any], key: str, errors: list[str], kind: type | tuple = str
) -> Any:
    value = doc.get(key)
    if value is None or not isinstance(value, kind):
        errors.append(f"{doc.get('id', '?')}: missing or mistyped {key!r}")
        return None
    return value


def _validate_doc(
    doc: dict[str, Any], repos: dict[str, Any], seen: set[str]
) -> list[str]:
    errors: list[str] = []
    ident = _need(doc, "id", errors)
    if ident is None:
        return errors
    if not ID_RE.match(ident):
        errors.append(f"{ident}: id must match {ID_RE.pattern}")
    if ident in seen:
        errors.append(f"{ident}: duplicate id")
    seen.add(ident)

    genre = _need(doc, "genre", errors)
    if genre is not None and genre not in GENRES:
        errors.append(f"{ident}: unknown genre {genre!r}")
    origin = _need(doc, "origin", errors)
    if origin is not None and origin not in ORIGINS:
        errors.append(f"{ident}: unknown origin {origin!r}")
    fmt = _need(doc, "format", errors)
    if fmt is not None and fmt not in FORMATS:
        errors.append(f"{ident}: unknown format {fmt!r}")
    _need(doc, "license", errors)
    pending = origin == "model-twin" and doc.get("status") == "pending"
    date = doc.get("date") if pending else _need(doc, "date", errors)
    if date is not None:
        if not DATE_RE.match(date):
            errors.append(f"{ident}: date must be YYYY-MM-DD")
        elif origin == "human" and date >= HUMAN_CUTOFF:
            errors.append(f"{ident}: human text must predate {HUMAN_CUTOFF}")
    committed = _need(doc, "committed", errors, bool)

    source = _need(doc, "source", errors, dict)
    if source is not None:
        errors.extend(_validate_source(ident, origin, doc, source, repos))

    sha = doc.get("sha256")
    if pending:
        if sha is not None or committed:
            errors.append(f"{ident}: a pending twin has no hash and is not committed")
    elif not isinstance(sha, str) or not SHA256_RE.match(sha):
        errors.append(f"{ident}: sha256 must be 64 lowercase hex digits")

    if committed:
        path = doc.get("path")
        if not isinstance(path, str) or ".." in path or path.startswith("/"):
            errors.append(f"{ident}: committed documents need a relative path")
        if doc.get("license") not in PERMISSIVE:
            errors.append(
                f"{ident}: licence {doc.get('license')!r} may not be committed"
            )
        if origin not in ("human", "stated-llm"):
            errors.append(f"{ident}: only upstream documents are committed")
        if isinstance(doc.get("bytes"), int) and doc["bytes"] > MAX_COMMITTED_BYTES:
            errors.append(
                f"{ident}: committed copy exceeds {MAX_COMMITTED_BYTES} bytes"
            )
    return errors


def _validate_source(
    ident: str,
    origin: Any,
    doc: dict[str, Any],
    source: dict[str, Any],
    repos: dict[str, Any],
) -> list[str]:
    errors: list[str] = []
    stype = source.get("type")
    if stype not in SOURCE_TYPES:
        return [f"{ident}: unknown source type {stype!r}"]
    if stype in ("github-file", "github-commit", "github-pr", "json-field"):
        repo = source.get("repo")
        if repo not in repos:
            errors.append(f"{ident}: repo {repo!r} missing from the repos table")
        if not isinstance(source.get("url"), str) or not source["url"].startswith(
            "https://"
        ):
            errors.append(f"{ident}: source.url must be an https URL")
        commit = source.get("commit")
        if stype != "github-pr" and not (
            isinstance(commit, str) and COMMIT_RE.match(commit)
        ):
            errors.append(f"{ident}: source.commit must be a 40-digit commit id")
        if stype == "github-file":
            lines = source.get("lines")
            if lines is not None and not (
                isinstance(lines, list)
                and len(lines) == 2
                and all(isinstance(n, int) for n in lines)
                and 1 <= lines[0] <= lines[1]
            ):
                errors.append(f"{ident}: source.lines must be [first, last], 1-based")
        if stype == "github-pr" and not isinstance(source.get("number"), int):
            errors.append(f"{ident}: source.number must be an integer")
        if stype == "json-field" and not isinstance(source.get("json_path"), list):
            errors.append(f"{ident}: source.json_path must be a list")
    if stype == "derived":
        parents = source.get("parents")
        if not (isinstance(parents, list) and parents):
            errors.append(f"{ident}: derived documents need parents")
        for key in ("generator", "seed"):
            if key not in source:
                errors.append(f"{ident}: derived source needs {key!r}")
        if origin not in ("seeded-defect", "clean-control", "rewrite-pair"):
            errors.append(f"{ident}: only generated origins may be derived")
    if (
        origin in ("seeded-defect", "clean-control", "rewrite-pair")
        and stype != "derived"
    ):
        errors.append(f"{ident}: {origin} documents are derived")
    if origin == "model-twin":
        if stype != "model-output":
            errors.append(f"{ident}: model twins use source type 'model-output'")
        for key in ("family", "outline_of", "outline_sha256"):
            if not isinstance(source.get(key), str):
                errors.append(f"{ident}: twin source needs {key!r}")
        if doc.get("status") not in TWIN_STATUS:
            errors.append(f"{ident}: twin status must be one of {TWIN_STATUS}")
    elif origin == "human" and stype in ("derived", "model-output"):
        errors.append(f"{ident}: human documents come from an upstream source")
    return errors


# --- fetching --------------------------------------------------------------

USER_AGENT = "slopvac-evals-fetch/1"


class FetchError(Exception):
    pass


def github_token(allow_gh_cli: bool = False) -> str | None:
    token = os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN")
    if token or not allow_gh_cli:
        return token
    try:
        out = subprocess.run(
            ["gh", "auth", "token"],
            capture_output=True,
            text=True,
            timeout=20,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    return out.stdout.strip() or None


def http_get(url: str, token: str | None = None, retries: int = 4) -> bytes:
    headers = {"User-Agent": USER_AGENT}
    if url.startswith("https://api.github.com/"):
        headers["Accept"] = "application/vnd.github+json"
        if token:
            headers["Authorization"] = f"Bearer {token}"
    last: Exception | None = None
    for attempt in range(retries):
        request = urllib.request.Request(url, headers=headers)
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                return response.read()
        except urllib.error.HTTPError as exc:
            last = exc
            if exc.code in (429, 502, 503, 504) or (
                exc.code == 403 and exc.headers.get("X-RateLimit-Remaining") == "0"
            ):
                time.sleep(min(60, 5 * 2**attempt))
                continue
            break
        except (
            urllib.error.URLError,
            TimeoutError,
            http.client.HTTPException,
            ConnectionError,
        ) as exc:
            last = exc
            time.sleep(2**attempt)
    hint = ""
    if isinstance(last, urllib.error.HTTPError) and last.code == 403:
        hint = " (set GITHUB_TOKEN to raise the GitHub API rate limit)"
    raise FetchError(f"GET {url} failed: {last}{hint}")


def raw_url(repo: str, commit: str, path: str) -> str:
    return (
        f"https://raw.githubusercontent.com/{repo}/{commit}/{urllib.parse.quote(path)}"
    )


def extract_lines(text: str, lines: list[int] | None) -> str:
    if not lines:
        return text
    chunk = text.split("\n")[lines[0] - 1 : lines[1]]
    return "\n".join(chunk) + "\n"


def walk_json(data: Any, path: list[Any]) -> Any:
    for step in path:
        data = data[step]
    return data


def fetch_text(doc: dict[str, Any], token: str | None, raw_cache: Path) -> str:
    """Download one upstream document and return its normalised text.

    Whole-file downloads are kept in `raw_cache` by URL, so documents that cut
    different ranges from one file cost one request.
    """
    source = doc["source"]
    stype = source["type"]
    if stype == "github-file":
        body = _cached_get(source["url"], token, raw_cache)
        return extract_lines(normalise(body.decode("utf-8")), source.get("lines"))
    if stype == "github-commit":
        data = json.loads(_cached_get(source["url"], token, raw_cache))
        return normalise(data["message"]).rstrip("\n") + "\n"
    if stype == "github-pr":
        data = json.loads(http_get(source["url"], token))
        body = normalise(data.get("body") or "").strip()
        return normalise(data["title"]) + "\n\n" + body + "\n"
    if stype == "json-field":
        data = json.loads(_cached_get(source["url"], token, raw_cache))
        return normalise(str(walk_json(data, source["json_path"]))).rstrip("\n") + "\n"
    raise FetchError(f"{doc['id']}: cannot fetch source type {stype!r}")


def _cached_get(
    url: str,
    token: str | None,
    raw_cache: Path,
    getter: Callable[[str, str | None], bytes] = http_get,
) -> bytes:
    """Download once per URL; safe when several threads miss the same entry.

    Each download writes its own temporary file in the cache directory and then
    renames it into place, so racing writers never share a file and a reader
    never sees a partial one. Last rename wins; the bytes are identical.
    """
    key = hashlib.sha256(url.encode()).hexdigest()
    path = raw_cache / key
    try:
        return path.read_bytes()
    except FileNotFoundError:
        pass
    body = getter(url, token)
    raw_cache.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        dir=raw_cache, prefix=key[:12] + ".", suffix=".tmp", delete=False
    ) as handle:
        handle.write(body)
        tmp = Path(handle.name)
    os.replace(tmp, path)
    return body
