"""Engine adapters and result storage for the eval harness.

An engine is any command that takes `lint <paths> --format json` and prints a
lint report. The Python reference and the Rust binary share that contract, so
the harness runs either one by changing the command.

    python -m evals run --engine python --out results.json
    python -m evals run --engine ./target/release/slopvac --out rust.json

The runner stages every document under a name with the right extension, in a
temporary directory outside the repository. Committed corpus files end in
`.corpus` so prose gates skip them, and the repository config excludes their
directory, so linting them in place would find nothing or fail.

Results are plain JSON. The offline tier scores a stored results file and never
starts an engine.
"""

from __future__ import annotations

import importlib.util
import json
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
from collections.abc import Callable, Iterable, Mapping
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from . import corpuslib as lib
from . import procs

RESULTS_SCHEMA = 1
BATCH = 50
REPO_ROOT = lib.EVALS_DIR.parent

# The Python reference is the engine until cutover; pass the Rust binary to
# --engine afterwards, since the command line is the same.
PYTHON_FALLBACK = f"uv run --project {shlex.quote(str(REPO_ROOT / 'packages/slopvac-lint'))} --offline slopvac"


def python_reference() -> tuple[str, ...]:
    """Command for the Python reference.

    Use the interpreter that runs the harness when it can import slopvac, so CI
    and `uv run` need no second tool. Otherwise ask uv for the package project.
    """
    if importlib.util.find_spec("slopvac"):
        return (sys.executable, "-c", "from slopvac.cli import main; main()")
    return tuple(shlex.split(PYTHON_FALLBACK))


# The reference reads prose and source comments only. Diagram and structured
# string extractors arrive with the Rust engine.
PYTHON_UNSUPPORTED = frozenset({"drawio", "json", "ini"})

PROSE_SUFFIX = {
    "markdown": ".md",
    "mermaid-markdown": ".md",
    "pull-request": ".md",
    "rst": ".rst",
    "commit-message": ".txt",
}
SOURCE_SUFFIX = {
    "rust": ".rs",
    "python": ".py",
    "go": ".go",
    "c": ".c",
    "javascript": ".js",
    "typescript": ".ts",
}


class EngineError(Exception):
    pass


class MissingText(EngineError):
    """A document text is neither committed nor in the cache."""


def _resolve_executable(name: str) -> str:
    """Make a relative path absolute now, because the engine runs from another directory.

    A bare name stays as it is and is found on PATH at run time.
    """
    if os.sep in name or name.startswith("."):
        return str(Path(name).resolve())
    return name


@dataclass(frozen=True)
class Engine:
    name: str
    command: tuple[str, ...]
    cwd: Path | None = None
    unsupported: frozenset[str] = frozenset()
    extra_args: tuple[str, ...] = field(default=())

    @classmethod
    def parse(cls, spec: str, cwd: Path | None = None) -> Engine:
        if spec == "python":
            return cls("python", python_reference(), cwd, PYTHON_UNSUPPORTED)
        command = shlex.split(spec)
        return cls(
            Path(command[0]).name, (_resolve_executable(command[0]), *command[1:]), cwd
        )

    def lint(self, paths: Iterable[Path], extra: Iterable[str] = ()) -> dict[str, Any]:
        argv = [
            *self.command,
            "lint",
            *map(str, paths),
            "--format",
            "json",
            *self.extra_args,
            *extra,
        ]
        proc = subprocess.run(
            argv, capture_output=True, text=True, cwd=self.cwd, check=False
        )
        # Exit 1 means a gate failed, which is normal for slop. Exit 2 means the run
        # was incomplete, for example rules that cannot run on code comments. That
        # still yields findings, so keep them and record the gap per document.
        # Anything else, or exit 2 without a report, is an engine failure.
        stderr = procs.redact(proc.stderr.strip())[:500]
        try:
            report = json.loads(proc.stdout)
        except json.JSONDecodeError as exc:
            raise EngineError(
                f"{self.name} exited {proc.returncode} with no report: {stderr or exc}"
            ) from exc
        if proc.returncode not in (0, 1, 2) or "documents" not in report:
            raise EngineError(f"{self.name} exited {proc.returncode}: {stderr}")
        return report


def text_path(doc: Mapping[str, Any], cache: Path) -> Path:
    """Where a document's text lives: committed, derived in the cache, or a twin."""
    if doc.get("committed"):
        return lib.committed_file(doc)
    folder = "twins" if doc["origin"] == "model-twin" else "docs"
    path = cache / folder / f"{doc['id']}.txt"
    if not path.is_file():
        hint = (
            "run `python -m evals.twins generate` and `record`"
            if folder == "twins"
            else "run `python -m evals.fetch` and `python -m evals.build_derived --materialise`"
        )
        raise MissingText(f"{doc['id']} is not in the cache; {hint}")
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


def available_selector(cache: Path) -> Callable[[Mapping[str, Any]], bool]:
    """Select documents whose text is committed or cached, so CI can run a subset."""

    def select(doc: Mapping[str, Any]) -> bool:
        if not default_selector(doc):
            return False
        if doc.get("committed"):
            return True
        try:
            text_path(doc, cache)
        except MissingText:
            return False
        return True

    return select


def _rst_available() -> bool:
    return bool(shutil.which("rst2html") or shutil.which("rst2html.py"))


def classify(doc: Mapping[str, Any], engine: Engine) -> tuple[str, str] | str:
    """Return (kind, suffix) for a lintable document, else the reason it is skipped."""
    fmt = doc["format"]
    if fmt in engine.unsupported:
        return f"format {fmt!r} has no extractor in this engine"
    if fmt in SOURCE_SUFFIX:
        return "code", SOURCE_SUFFIX[fmt]
    if fmt == "rst" and not _rst_available():
        return "rst converter (docutils) is not installed"
    return "prose", PROSE_SUFFIX.get(fmt, ".txt")


def run_engine(
    engine: Engine,
    manifest: Mapping[str, Any],
    cache: Path,
    *,
    select: Callable[[Mapping[str, Any]], bool] = default_selector,
    batch: int = BATCH,
    stage_root: Path | None = None,
) -> dict[str, Any]:
    """Lint every selected document and return a results record."""
    docs = [d for d in manifest["documents"] if select(d)]
    skipped: dict[str, str] = {}
    results: dict[str, Any] = {}
    version: str | None = None
    with tempfile.TemporaryDirectory(prefix="slopvac-evals-", dir=stage_root) as tmp:
        stage = Path(tmp)
        groups: dict[str, dict[str, Mapping[str, Any]]] = {"prose": {}, "code": {}}
        for doc in docs:
            plan = classify(doc, engine)
            if isinstance(plan, str):
                skipped[doc["id"]] = plan
                continue
            kind, suffix = plan
            target = stage / f"{doc['id']}{suffix}"
            shutil.copyfile(text_path(doc, cache), target)
            groups[kind][str(target.resolve())] = doc
        # Run from the staging directory so the repository's own slopvac.toml,
        # which excludes the corpus, never applies to eval documents.
        staged_engine = Engine(
            engine.name,
            engine.command,
            engine.cwd or stage,
            engine.unsupported,
            engine.extra_args,
        )
        for kind, by_path in groups.items():
            extra = ("--mode", "code-comments") if kind == "code" else ()
            paths = list(by_path)
            for start in range(0, len(paths), batch):
                chunk = paths[start : start + batch]
                report = staged_engine.lint([Path(p) for p in chunk], extra)
                version = (
                    report.get("slopvac_version") or report.get("version") or version
                )
                for entry in report.get("documents", []):
                    # Adapters may report paths relative to their working directory.
                    reported = (staged_engine.cwd or Path.cwd()) / entry["path"]
                    doc = by_path.get(str(reported.resolve()))
                    if doc is None:
                        raise EngineError(
                            f"report names an unknown path: {entry['path']}"
                        )
                    results[doc["id"]] = {
                        "words": entry.get("words", 0),
                        "findings": [_slim(f) for f in entry.get("findings", [])],
                    }
                    if entry.get("unchecked"):
                        results[doc["id"]]["unchecked"] = len(entry["unchecked"])
    missing = sorted(
        d["id"] for d in docs if d["id"] not in results and d["id"] not in skipped
    )
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
        "skipped": dict(sorted(skipped.items())),
        "documents": results,
    }


# Fields that record when something was checked, not what was found. A later
# re-check must not invalidate results. Every other field, including the
# licence, date and per-document `provenance`, is part of corpus identity.
BOOKKEEPING_REPO_FIELDS = ("public_checked_at",)


def manifest_digest(manifest: Mapping[str, Any]) -> str:
    """Corpus identity: the manifest minus re-check dates.

    Licence, date, licence revision, licence basis and date source all count,
    so the gate rejects a baseline recorded against different evidence.
    """
    identity = {
        **manifest,
        "repos": {
            name: {k: v for k, v in repo.items() if k not in BOOKKEEPING_REPO_FIELDS}
            for name, repo in manifest.get("repos", {}).items()
        },
    }
    return lib.sha256_text(json.dumps(identity, sort_keys=True, separators=(",", ":")))


def check_corpus(results: Mapping[str, Any], manifest: Mapping[str, Any]) -> None:
    """Reject results recorded against a different manifest."""
    recorded = (results.get("corpus") or {}).get("manifest_sha256")
    if recorded != manifest_digest(manifest):
        raise EngineError(
            "results were recorded against a different manifest "
            f"({str(recorded)[:12]} vs {manifest_digest(manifest)[:12]}); run the engine again"
        )


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
