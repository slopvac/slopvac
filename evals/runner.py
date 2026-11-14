"""Engine adapters and result storage for the eval harness.

An engine is any command that takes `lint <paths> --format json` and prints a
lint report. The Python reference and the Rust binary share that contract, so
the harness runs either one by changing the command.

    python -m evals run --engine python --out results.json
    python -m evals run --engine ./target/release/slopvac --out rust.json

Results are plain JSON. The offline tier scores a stored results file and never
starts an engine.
"""

from __future__ import annotations

import json
import shlex
import subprocess
from collections.abc import Callable, Iterable, Mapping
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from . import corpuslib as lib

RESULTS_SCHEMA = 1
BATCH = 50

# STUB(slopvac-8l4.15.6): the Python reference stands in for the Rust binary
# until cutover. Point --engine at the binary then; the command line is the same.
PYTHON_REFERENCE = "uv run --project packages/slopvac-lint --offline slopvac"


class EngineError(Exception):
    pass


class MissingText(EngineError):
    """A document text is neither committed nor in the cache."""


@dataclass(frozen=True)
class Engine:
    name: str
    command: tuple[str, ...]
    cwd: Path | None = None
    extra_args: tuple[str, ...] = field(default=())

    @classmethod
    def parse(cls, spec: str, cwd: Path | None = None) -> Engine:
        if spec == "python":
            return cls("python", tuple(shlex.split(PYTHON_REFERENCE)), cwd)
        return cls(Path(spec).name, tuple(shlex.split(spec)), cwd)

    def lint(self, paths: Iterable[Path]) -> dict[str, Any]:
        argv = [
            *self.command,
            "lint",
            *map(str, paths),
            "--format",
            "json",
            *self.extra_args,
        ]
        proc = subprocess.run(
            argv, capture_output=True, text=True, cwd=self.cwd, check=False
        )
        # Exit 1 means a gate failed, which is normal for slop. Exit 2 is incomplete.
        if proc.returncode not in (0, 1):
            raise EngineError(
                f"{self.name} exited {proc.returncode}: {proc.stderr.strip()[:500]}"
            )
        try:
            return json.loads(proc.stdout)
        except json.JSONDecodeError as exc:
            raise EngineError(f"{self.name} printed invalid JSON: {exc}") from exc


def text_path(doc: Mapping[str, Any], cache: Path) -> Path:
    """Where a document's text lives: the committed file, else the cache."""
    if doc.get("committed"):
        return lib.committed_file(doc)
    path = lib.cache_file(cache, doc["id"])
    if not path.is_file():
        raise MissingText(
            f"{doc['id']} is not cached; run `python -m evals.fetch` "
            "and `python -m evals.build_derived --materialise`"
        )
    return path


def _slim(finding: Mapping[str, Any]) -> dict[str, Any]:
    out = {
        "rule": finding["rule_id"],
        "line": finding["line"],
        "severity": finding.get("severity"),
    }
    if finding.get("end_line") is not None:
        out["end_line"] = finding["end_line"]
    if finding.get("matched_text") is not None:
        out["text"] = finding["matched_text"]
    return out


def default_selector(doc: Mapping[str, Any]) -> bool:
    return doc["origin"] != "model-twin" or doc.get("status") == "generated"


def run_engine(
    engine: Engine,
    manifest: Mapping[str, Any],
    cache: Path,
    *,
    select: Callable[[Mapping[str, Any]], bool] = default_selector,
    batch: int = BATCH,
) -> dict[str, Any]:
    """Lint every selected document and return a results record."""
    docs = [d for d in manifest["documents"] if select(d)]
    # Link each document under a name that carries its id, so a report maps back
    # without trusting path order.
    by_path: dict[str, Mapping[str, Any]] = {}
    for doc in docs:
        by_path[str(text_path(doc, cache).resolve())] = doc
    results: dict[str, Any] = {}
    version: str | None = None
    paths = list(by_path)
    for start in range(0, len(paths), batch):
        report = engine.lint([Path(p) for p in paths[start : start + batch]])
        version = report.get("slopvac_version") or report.get("version")
        for entry in report.get("documents", []):
            doc = by_path.get(str(Path(entry["path"]).resolve()))
            if doc is None:
                raise EngineError(f"report names an unknown path: {entry['path']}")
            results[doc["id"]] = {
                "words": entry.get("words", 0),
                "findings": [_slim(f) for f in entry.get("findings", [])],
            }
    missing = sorted(d["id"] for d in docs if d["id"] not in results)
    if missing:
        raise EngineError(
            f"engine returned no report for {len(missing)} documents: {missing[:3]}"
        )
    return {
        "schema": RESULTS_SCHEMA,
        "engine": {
            "name": engine.name,
            "command": list(engine.command),
            "version": version,
        },
        "corpus": {"manifest_sha256": manifest_digest(manifest)},
        "documents": results,
    }


def manifest_digest(manifest: Mapping[str, Any]) -> str:
    return lib.sha256_text(json.dumps(manifest, sort_keys=True, separators=(",", ":")))


def load_results(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("schema") != RESULTS_SCHEMA:
        raise EngineError(f"{path}: unsupported results schema {data.get('schema')!r}")
    return data


def dump_results(results: Mapping[str, Any], path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(results, indent=1, sort_keys=True) + "\n", encoding="utf-8"
    )
