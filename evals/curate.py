"""Build `manifest.json` from `sources.toml` (maintainers only; needs network).

    python -m evals.curate                 # discover, hash, write manifest + files
    python -m evals.curate --dry-run       # print counts only

Selection is deterministic for a given upstream state: every random choice
seeds from `SEED` plus the source identity, and every source is pinned to the
last commit before the cutoff. Re-running does not move a pin. Derived
documents (`seeded-defect`, `clean-control`, `rewrite-pair`, `model-twin`) are
added by `evals.build_derived`, not here.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import random
import re
import sys
import time
import tomllib
import urllib.parse
from concurrent.futures import ThreadPoolExecutor
from typing import Any

from . import corpuslib as lib

SEED = "slopvac-evals-1"
CUTOFF_ISO = lib.HUMAN_CUTOFF + "T00:00:00Z"
API = "https://api.github.com"
SOURCES = lib.EVALS_DIR / "sources.toml"
DIAGRAM_CANDIDATES = lib.EVALS_DIR / "candidates-diagrams.json"
CORPUS = lib.EVALS_DIR / "corpus"
DERIVED_ORIGINS = ("seeded-defect", "clean-control", "rewrite-pair", "model-twin")

TOKEN: str | None = None


def api(endpoint: str, **params: Any) -> Any:
    query = urllib.parse.urlencode(params)
    url = f"{API}/{endpoint}" + (f"?{query}" if query else "")
    return json.loads(lib.http_get(url, TOKEN))


def slug(text: str, limit: int = 48) -> str:
    return re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")[:limit].strip("-")


def doc_id(genre: str, repo: str, key: str) -> str:
    digest = hashlib.sha256(f"{repo}|{key}".encode()).hexdigest()[:6]
    return f"{genre}.{slug(repo.replace('/', '-'), 40)}.{slug(key)}.{digest}"


def spdx_or_none(value: str | None) -> str | None:
    return None if value in (None, "NOASSERTION", "other") else value


def pin_repo(repo: str, repos: dict[str, Any], *, latest: bool = False) -> dict[str, Any]:
    """Resolve the pin, licence and copyright line once per repo.

    Human sources pin the last commit before the cutoff. Stated-LLM sources
    postdate it by design and pin the latest commit.
    """
    if repo in repos:
        return repos[repo]
    params: dict[str, Any] = {"per_page": 1}
    if not latest:
        params["until"] = CUTOFF_ISO
    commits = api(f"repos/{repo}/commits", **params)
    if not commits:
        raise RuntimeError(f"{repo}: no commit before {CUTOFF_ISO}")
    sha = commits[0]["sha"]
    date = commits[0]["commit"]["committer"]["date"][:10]
    try:
        info = api(f"repos/{repo}/license", ref=sha)
    except lib.FetchError:
        info = api(f"repos/{repo}/license")
    licence = spdx_or_none(info["license"]["spdx_id"])
    content = base64.b64decode(info["content"]).decode("utf-8", "replace")
    copyright_line = None
    for line in content.splitlines():
        if re.match(r"\s*(copyright|\(c\))", line, re.I) and len(line) < 200:
            copyright_line = line.strip()
            break
    repos[repo] = {
        "url": f"https://github.com/{repo}",
        "commit": sha,
        "commit_date": date,
        "license": licence,
        "copyright": copyright_line or f"The {repo} authors",
    }
    return repos[repo]


def make_doc(
    *,
    genre: str,
    origin: str,
    fmt: str,
    repo: str,
    licence: str | None,
    key: str,
    source: dict[str, Any],
    text: str,
    date: str,
) -> dict[str, Any]:
    if licence is None:
        raise RuntimeError(f"{repo}: no SPDX licence; set license in sources.toml")
    ident = doc_id(genre, repo, key)
    size = len(lib.normalise(text).encode())
    committed = licence in lib.PERMISSIVE and size <= lib.MAX_COMMITTED_BYTES
    doc: dict[str, Any] = {
        "id": ident,
        "genre": genre,
        "origin": origin,
        "format": fmt,
        "license": licence,
        "date": date,
        "source": source,
        "sha256": lib.sha256_text(text),
        "bytes": size,
        "words": len(text.split()),
        "committed": committed,
    }
    if committed:
        doc["path"] = f"corpus/{genre}/{ident}.txt"
    return doc


def write_committed(doc: dict[str, Any], text: str) -> None:
    if doc["committed"]:
        path = lib.committed_file(doc)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(lib.normalise(text), encoding="utf-8")


Curated = list[tuple[dict[str, Any], str]]


def curate_files(spec: dict[str, Any], repos: dict[str, Any]) -> Curated:
    repo = spec["repo"]
    info = pin_repo(repo, repos)
    tree = api(f"repos/{repo}/git/trees/{info['commit']}", recursive=1)["tree"]
    suffixes = tuple(spec.get("suffix", [".md"]))
    must = re.compile(spec["must"]) if "must" in spec else None
    skip = re.compile(spec["skip"]) if "skip" in spec else None
    if "paths" in spec:
        entries = [
            t for t in tree if t["path"] in spec["paths"] and t["type"] == "blob"
        ]
    else:
        entries = [
            t
            for t in tree
            if t["type"] == "blob"
            and t["path"].startswith(spec.get("prefix", ""))
            and t["path"].endswith(suffixes)
            and spec.get("min_bytes", 2500)
            <= t.get("size", 0)
            <= spec.get("max_bytes", 400_000)
            and (not must or must.search(t["path"]))
            and not (skip and skip.search(t["path"]))
        ]
    entries.sort(key=lambda t: t["path"])
    rng = random.Random(f"{SEED}|{repo}|{spec.get('prefix', '')}|{spec['genre']}")
    rng.shuffle(entries)
    per_dir: dict[str, int] = {}
    out: Curated = []
    for entry in entries:
        if len(out) >= spec.get("count", 5):
            break
        parent = entry["path"].rsplit("/", 1)[0]
        if per_dir.get(parent, 0) >= spec.get("per_dir", 3):
            continue
        url = lib.raw_url(repo, info["commit"], entry["path"])
        try:
            text = lib.normalise(lib.http_get(url).decode("utf-8"))
        except (lib.FetchError, UnicodeDecodeError):
            continue
        lines_range = None
        max_lines = spec.get("max_lines")
        trunc = spec.get("trunc_bytes")
        all_lines = text.split("\n")
        if max_lines and len(all_lines) > max_lines:
            start = rng.randint(1, len(all_lines) - max_lines + 1)
            lines_range = [start, start + max_lines - 1]
        elif trunc and len(text.encode()) > trunc:
            used = n = 0
            for n, line in enumerate(all_lines, 1):
                used += len(line.encode()) + 1
                if used > trunc:
                    n -= 1
                    break
            lines_range = [1, max(n, 1)]
        if lines_range:
            text = lib.extract_lines(text, lines_range)
        if len(text.split()) < spec.get("min_words", 150):
            continue
        per_dir[parent] = per_dir.get(parent, 0) + 1
        source: dict[str, Any] = {
            "type": "github-file",
            "repo": repo,
            "commit": info["commit"],
            "path": entry["path"],
            "url": url,
        }
        if lines_range:
            source["lines"] = lines_range
        key = entry["path"] + (f"-{lines_range[0]}" if lines_range else "")
        doc = make_doc(
            genre=spec["genre"],
            origin="human",
            fmt=spec["format"],
            repo=repo,
            licence=spec.get("license") or info["license"],
            key=key,
            source=source,
            text=text,
            date=info["commit_date"],
        )
        out.append((doc, text))
    return out


def curate_commits(spec: dict[str, Any], repos: dict[str, Any]) -> Curated:
    repo = spec["repo"]
    info = pin_repo(repo, repos)
    pool: list[dict[str, Any]] = []
    for page in range(1, spec.get("pages", 4) + 1):
        batch = api(f"repos/{repo}/commits", until=CUTOFF_ISO, per_page=100, page=page)
        pool.extend(batch)
        if len(batch) < 100:
            break
    keep = []
    for item in pool:
        message = item["commit"]["message"]
        body = message.partition("\n")[2].strip()
        author = (item.get("author") or {}).get("type")
        if (
            len(item["parents"]) == 1
            and author != "Bot"
            and spec.get("min_body", 300) <= len(body) <= spec.get("max_body", 3000)
        ):
            keep.append(item)
    keep.sort(key=lambda i: i["sha"])
    random.Random(f"{SEED}|{repo}|commits").shuffle(keep)
    out: Curated = []
    for item in keep[: spec.get("count", 8)]:
        text = lib.normalise(item["commit"]["message"]).rstrip("\n") + "\n"
        source = {
            "type": "github-commit",
            "repo": repo,
            "commit": item["sha"],
            "url": f"{API}/repos/{repo}/git/commits/{item['sha']}",
            "html_url": f"https://github.com/{repo}/commit/{item['sha']}",
        }
        doc = make_doc(
            genre="commits",
            origin="human",
            fmt="commit-message",
            repo=repo,
            licence=spec.get("license") or info["license"],
            key=item["sha"][:12],
            source=source,
            text=text,
            date=item["commit"]["committer"]["date"][:10],
        )
        out.append((doc, text))
    return out


def curate_prs(spec: dict[str, Any], repos: dict[str, Any]) -> Curated:
    repo = spec["repo"]
    info = pin_repo(repo, repos)
    query = f"repo:{repo} is:pr is:merged created:{spec.get('since', '2021-01-01')}..2022-10-31"
    time.sleep(2.5)  # the search API allows 30 requests a minute
    result = api("search/issues", q=query, sort="comments", order="desc", per_page=100)
    keep = []
    for item in result["items"]:
        body = (item.get("body") or "").strip()
        if item["user"]["type"] != "Bot" and (
            spec.get("min_body", 500) <= len(body) <= spec.get("max_body", 5000)
        ):
            keep.append(item)
    keep.sort(key=lambda i: i["number"])
    random.Random(f"{SEED}|{repo}|prs").shuffle(keep)
    out: Curated = []
    for item in keep[: spec.get("count", 8)]:
        text = (
            lib.normalise(item["title"])
            + "\n\n"
            + lib.normalise(item["body"]).strip()
            + "\n"
        )
        source = {
            "type": "github-pr",
            "repo": repo,
            "number": item["number"],
            "url": f"{API}/repos/{repo}/pulls/{item['number']}",
            "html_url": item["html_url"],
        }
        doc = make_doc(
            genre="prs",
            origin="human",
            fmt="pull-request",
            repo=repo,
            licence=spec.get("license") or info["license"],
            key=f"pr-{item['number']}",
            source=source,
            text=text,
            date=item["created_at"][:10],
        )
        out.append((doc, text))
    return out


def curate_stated_llm(spec: dict[str, Any], repos: dict[str, Any]) -> Curated:
    """Model outputs that their publisher labels with the producing model."""
    repo = spec["repo"]
    info = pin_repo(repo, repos, latest=True)
    out: Curated = []
    for model in spec["models"]:
        path = spec["path"].format(model=model)
        url = lib.raw_url(repo, info["commit"], path)
        data = json.loads(lib.http_get(url))
        idx = [
            i
            for i, row in enumerate(data)
            if len(row["output"].split()) >= spec.get("min_words", 150)
            and len(row["output"].encode()) <= 6000
        ]
        random.Random(f"{SEED}|{repo}|{model}").shuffle(idx)
        for i in sorted(idx[: spec.get("count", 12)]):
            text = lib.normalise(data[i]["output"]).rstrip("\n") + "\n"
            source = {
                "type": "json-field",
                "repo": repo,
                "commit": info["commit"],
                "path": path,
                "url": url,
                "json_path": [i, "output"],
                "model": data[i].get("generator", model),
                "stated_by": "publisher",
            }
            doc = make_doc(
                genre="assistant-answers",
                origin="stated-llm",
                fmt="markdown",
                repo=repo,
                licence=spec.get("license") or info["license"],
                key=f"{model}-{i}",
                source=source,
                text=text,
                date=info["commit_date"],
            )
            out.append((doc, text))
    return out


def curate_diagrams(spec: dict[str, Any], repos: dict[str, Any]) -> Curated:
    """Diagram files from the candidate list, each pinned at its own last pre-cutoff commit."""
    candidates = [
        c for c in json.loads(DIAGRAM_CANDIDATES.read_text()) if c[0] == spec["kind"]
    ]
    candidates.sort()
    random.Random(f"{SEED}|diagrams|{spec['kind']}").shuffle(candidates)
    repo_meta: dict[str, tuple[str | None, bool]] = {}

    def probe(cand: list[str]) -> tuple[dict[str, Any], str] | None:
        _, repo, path = cand
        try:
            if repo not in repo_meta:
                meta = api(f"repos/{repo}")
                repo_meta[repo] = (
                    spdx_or_none((meta.get("license") or {}).get("spdx_id")),
                    meta["fork"],
                )
            licence, fork = repo_meta[repo]
            if fork or licence not in lib.PERMISSIVE:
                return None
            commits = api(
                f"repos/{repo}/commits", path=path, until=CUTOFF_ISO, per_page=1
            )
            if not commits:
                return None
            sha = commits[0]["sha"]
            date = commits[0]["commit"]["committer"]["date"][:10]
            url = lib.raw_url(repo, sha, path)
            text = lib.normalise(lib.http_get(url).decode("utf-8"))
        except (lib.FetchError, UnicodeDecodeError, KeyError, OSError):
            return None
        if len(text) > spec.get("max_bytes", 200_000):
            return None
        if spec["kind"] == "md":
            if "```mermaid" not in text or len(text.split()) < 80:
                return None
        elif "mxfile" not in text and "mxGraphModel" not in text:
            return None
        source = {
            "type": "github-file",
            "repo": repo,
            "commit": sha,
            "path": path,
            "url": url,
        }
        doc = make_doc(
            genre="diagrams",
            origin="human",
            fmt="mermaid-markdown" if spec["kind"] == "md" else "drawio",
            repo=repo,
            licence=licence,
            key=path + "-" + sha[:7],
            source=source,
            text=text,
            date=date,
        )
        return doc, text

    out: Curated = []
    perrepo: dict[str, int] = {}
    with ThreadPoolExecutor(max_workers=6) as pool:
        for result in pool.map(probe, candidates[: spec.get("probe", 260)]):
            if result is None:
                continue
            doc, text = result
            repo = doc["source"]["repo"]
            if (
                perrepo.get(repo, 0) >= spec.get("per_repo", 2)
                or len(out) >= spec["count"]
            ):
                continue
            perrepo[repo] = perrepo.get(repo, 0) + 1
            repos.setdefault(
                repo,
                {
                    "url": f"https://github.com/{repo}",
                    "license": doc["license"],
                    "copyright": f"The {repo} authors",
                },
            )
            out.append((doc, text))
    return out


HANDLERS = {
    "files": curate_files,
    "commits": curate_commits,
    "prs": curate_prs,
    "stated-llm": curate_stated_llm,
    "diagrams": curate_diagrams,
}


def main(argv: list[str] | None = None) -> int:
    global TOKEN
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--only", help="substring of the source name to run")
    args = parser.parse_args(argv)
    TOKEN = lib.github_token(allow_gh_cli=True)
    if not TOKEN:
        print("curation needs GITHUB_TOKEN or an authenticated gh CLI", file=sys.stderr)
        return 2

    spec = tomllib.loads(SOURCES.read_text())
    old = lib.load_manifest() if lib.MANIFEST_PATH.exists() else {"documents": []}
    kept = [d for d in old["documents"] if d["origin"] in DERIVED_ORIGINS]
    repos: dict[str, Any] = {}
    docs: Curated = []
    for item in spec["source"]:
        name = item.get("name") or item.get("repo", "")
        if args.only and args.only not in name:
            continue
        try:
            got = HANDLERS[item.get("type", "files")](item, repos)
        except Exception as exc:  # one bad source must not hide the others
            print(f"SKIP {name}: {exc}", file=sys.stderr)
            continue
        print(f"{name:60s} {len(got)}")
        docs.extend(got)

    seen: set[str] = set()
    final: Curated = []
    for doc, text in docs:
        if doc["id"] not in seen:
            seen.add(doc["id"])
            final.append((doc, text))
    print(f"{len(final)} documents")
    if args.dry_run:
        return 0

    # Rewrite the committed tree wholesale so dropped documents leave no files.
    for stale in CORPUS.rglob("*.txt"):
        stale.unlink()
    for doc, text in final:
        write_committed(doc, text)
    manifest = {
        "schema": lib.SCHEMA_VERSION,
        "human_cutoff": lib.HUMAN_CUTOFF,
        "seed": SEED,
        "repos": dict(sorted(repos.items())),
        "documents": [d for d, _ in final] + kept,
    }
    lib.dump_manifest(manifest)
    errors = lib.validate_manifest(manifest)
    for error in errors[:20]:
        print(error, file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
