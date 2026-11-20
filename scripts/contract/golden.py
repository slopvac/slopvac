"""Record and check the slopvac 3.0 golden suite.

The suite fixes what a 3.0 binary must reproduce. Three modes share one set of
inputs and one projection of the lint report:

- ``parity``: a 3.0 binary, run with the reference-equivalent legacy configuration
  (the generated ``core:legacy-<profile>`` style), matches the Python 2.x reference
  output, recorded under ``tests/golden/v3/parity/<profile>/<input>.json`` with the
  run records in ``runs/<profile>.json``.
- ``preserve``: the same expectations, reached through the config that
  ``migrate --preserve-behavior`` writes.
- ``policy``: the 3.0 default styles. The expectation is the parity output plus the
  reviewed deltas in ``tests/golden/v3/policy/deltas.toml``.

    python scripts/contract/golden.py record --reference <command>
    python scripts/contract/golden.py check  --candidate <command> [--mode parity]
    python scripts/contract/golden.py diff   --a <command> --b <command> [--mode parity]
    python scripts/contract/golden.py inputs        # rewrite the generated rule-example inputs
    python scripts/contract/golden.py pin           # rewrite pin.toml for the current reference

A ``<command>`` is a shell-split program prefix. The runner appends
``lint <file>... --config <config> --format json --mode <surface>``
for each surface and language (the format comes from the real file extension; 2.x
accepts ``--language`` only with ``--stdin``), and ``--profile`` only with ``--flavor 2x`` (a 3.0
binary takes the profile from its config). Exit codes 0, 1 and 2 all carry a report.
The run's exit status and summary are compared apart from the document gates. A golden file changes only with an approved reason (see
``tests/golden/v3/README.md``).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GOLDEN = ROOT / "tests" / "golden" / "v3"
INPUTS_TOML = GOLDEN / "inputs.toml"
PIN_TOML = GOLDEN / "pin.toml"
DELTAS_TOML = GOLDEN / "policy" / "deltas.toml"
RULE_EXAMPLES = GOLDEN / "inputs" / "rule-examples"
LEGACY_STYLES = ROOT / "docs" / "contract" / "compat" / "profiles"
PROFILES = ("strict", "normal", "relaxed")
MODES = ("parity", "preserve", "policy")
LINE_BREAK = re.compile(r"\r\n|\r|\n")

# Fixture inputs that live outside tests/golden. Each is pinned by sha256 in inputs.toml.
FIXTURE_ROOT = "packages/slopvac-lint/tests/fixtures/regression"
FIXTURE_FILES = {
    "stacklok-before": f"{FIXTURE_ROOT}/stacklok/permissions-and-posture.before.md",
    "stacklok-after": f"{FIXTURE_ROOT}/stacklok/permissions-and-posture.after.md",
    "issue-252": f"{FIXTURE_ROOT}/issues/252-judgement-checklist-config.md",
    "issue-253": f"{FIXTURE_ROOT}/issues/253-diff-base-symlink.md",
    "issue-254": f"{FIXTURE_ROOT}/issues/254-condition-after-command.md",
    "issue-255": f"{FIXTURE_ROOT}/issues/255-history-narration-current-state.md",
    "issue-256": f"{FIXTURE_ROOT}/issues/256-versioned-change-note.md",
    "issue-257": f"{FIXTURE_ROOT}/issues/257-title-case-proper-nouns.md",
}

# Format fixtures staged with their real extension. `surface` picks `--mode`, and
# `language` names the format that the extension selects.
FORMAT_ROOT = "tests/golden/v3/inputs/formats"
FORMAT_FILES = {
    "format-mdx": ("sample.mdx", "prose", "mdx"),
    "format-html": ("sample.html", "prose", "html"),
    "format-rst": ("sample.rst", "prose", "rst"),
    "format-python-comments": ("sample.py", "code-comments", "python"),
    "format-javascript-comments": ("sample.js", "code-comments", "javascript"),
}

# Eval-corpus tier: a deterministic subset of the committed human Markdown documents
# in evals/manifest.json, linted in place. The files end in `.corpus`, so the runner
# stages them as `.md`. The subset rule is in tests/golden/v3/README.md.
EVALS_MANIFEST = ROOT / "evals" / "manifest.json"
CORPUS_MAX_BYTES = 4096
CORPUS_STRIDE = 3


# --- inputs ----------------------------------------------------------------


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def rule_example_documents() -> dict[str, str]:
    """One document per category: every rule's examples, one paragraph each."""
    from slopvac.rules import load_ruleset

    docs: dict[str, str] = {}
    for rule in load_ruleset().rules:
        paragraphs = docs.setdefault(rule.category, [])
        for example in rule.examples:
            paragraphs.append(example.bad.strip())
            if example.good:
                paragraphs.append(example.good.strip())
    return {
        f"rule-examples-{category}": "\n\n".join(paras) + "\n"
        for category, paras in sorted(docs.items())
        if paras
    }


def write_rule_examples() -> None:
    RULE_EXAMPLES.mkdir(parents=True, exist_ok=True)
    for old in RULE_EXAMPLES.glob("*.md"):
        old.unlink()
    for doc_id, text in rule_example_documents().items():
        (RULE_EXAMPLES / f"{doc_id.removeprefix('rule-examples-')}.md").write_text(
            text, encoding="utf-8"
        )


def corpus_documents() -> list[dict]:
    """The manifest documents of the corpus tier.

    Human Markdown documents that the repository commits, at most 4096 bytes,
    sorted by manifest id, taking every third one from the first.
    """
    documents = json.loads(EVALS_MANIFEST.read_text(encoding="utf-8"))["documents"]
    eligible = sorted(
        (
            d
            for d in documents
            if d["origin"] == "human"
            and d["format"] == "markdown"
            and d["committed"]
            and d["bytes"] <= CORPUS_MAX_BYTES
        ),
        key=lambda d: d["id"],
    )
    return eligible[::CORPUS_STRIDE]


def staged_name(entry: dict) -> str:
    """The file name a run stages an input under: the format comes from the extension."""
    suffix = Path(entry["path"]).suffix
    return f"{entry['id']}{'.md' if suffix == '.corpus' else suffix}"


def input_entries() -> list[dict]:
    entries = []
    for path in sorted(RULE_EXAMPLES.glob("*.md")):
        entries.append(
            {
                "id": f"rule-examples-{path.stem}",
                "tier": "rule-examples",
                "path": path.relative_to(ROOT).as_posix(),
                "sha256": sha256(path),
                "surface": "prose",
                "language": "markdown",
            }
        )
    for doc_id, rel in FIXTURE_FILES.items():
        entries.append(
            {
                "id": doc_id,
                "tier": "stacklok" if doc_id.startswith("stacklok") else "issues",
                "path": rel,
                "sha256": sha256(ROOT / rel),
                "surface": "prose",
                "language": "markdown",
            }
        )
    for doc_id, (name, surface, language) in FORMAT_FILES.items():
        rel = f"{FORMAT_ROOT}/{name}"
        entries.append(
            {
                "id": doc_id,
                "tier": "formats",
                "path": rel,
                "sha256": sha256(ROOT / rel),
                "surface": surface,
                "language": language,
            }
        )
    for doc in corpus_documents():
        rel = f"evals/{doc['path']}"
        entries.append(
            {
                "id": f"corpus-{doc['id']}",
                "tier": "corpus",
                "path": rel,
                "sha256": sha256(ROOT / rel),
                "surface": "prose",
                "language": "markdown",
            }
        )
    return entries


def render_inputs(entries: list[dict]) -> str:
    lines = [
        "# Generated by scripts/contract/golden.py. Do not edit.",
        "# Each input is linted once per profile. `path` is relative to the repository root.",
        "# `surface` selects --mode. `language` names the format that the file extension selects.",
        "",
    ]
    for e in entries:
        lines += [
            "[[input]]",
            f'id = "{e["id"]}"',
            f'tier = "{e["tier"]}"',
            f'path = "{e["path"]}"',
            f'sha256 = "{e["sha256"]}"',
            f'surface = "{e["surface"]}"',
            f'language = "{e["language"]}"',
            "",
        ]
    return "\n".join(lines)


def load_inputs() -> list[dict]:
    return tomllib.loads(INPUTS_TOML.read_text(encoding="utf-8"))["input"]


# --- projection ------------------------------------------------------------


def split_lines(data: bytes) -> list[bytes]:
    """Lines by the position contract: LF, CRLF and a lone CR end a line."""
    text = data.decode("utf-8")
    parts = LINE_BREAK.split(text)
    return [p.encode("utf-8") for p in parts]


def line_starts(data: bytes) -> list[int]:
    starts = [0]
    for m in re.finditer(rb"\r\n|\r|\n", data):
        starts.append(m.end())
    return starts


def derive_span(
    data: bytes, line: int, column: int, end_line: int, end_column: int
) -> dict | None:
    """Byte span for code-point columns, or None when the position leaves the file."""
    starts = line_starts(data)
    text = data.decode("utf-8")
    lines_text = LINE_BREAK.split(text)
    try:
        start = starts[line - 1] + len(
            lines_text[line - 1][: column - 1].encode("utf-8")
        )
        end = starts[end_line - 1] + len(
            lines_text[end_line - 1][: end_column - 1].encode("utf-8")
        )
    except IndexError:
        return None
    return {"start": start, "end": end}


def _finding_2x(f: dict, data: bytes) -> dict:
    matched = f["matched_text"]
    extra = matched.count("\n")
    end_line = f["line"] + extra
    if extra:
        end_column = len(matched.rsplit("\n", 1)[1]) + 1
    else:
        end_column = f.get("end_column") or f["column"] + len(matched)
    span = derive_span(data, f["line"], f["column"], end_line, end_column)
    if span is None or data[span["start"] : span["end"]].decode("utf-8") != matched:
        # 2.x masks code spans and folds whitespace in matched_text, so the end of
        # this match cannot be proved from the source. Keep line and column only.
        span, end_line, end_column = None, None, None
    return {
        "rule_id": "core:" + f["rule_id"],
        "category": f["category"],
        "severity": f["severity"],
        "line": f["line"],
        "column": f["column"],
        "end_line": end_line,
        "end_column": end_column,
        "span": span,
        "matched_text": matched,
        "message": f["message"],
        "ai_signal": f["ai_signal"],
        "replacement": f.get("replacement"),
    }


def _finding_3(f: dict) -> dict:
    fix = f.get("fix")
    replacement = None
    if fix:
        edits = fix["edits"]
        if len(edits) != 1:
            raise ValueError(f"{f['rule_id']}: a multi-edit fix has no 2.x replacement")
        replacement = edits[0]["text"]
    return {
        "rule_id": f["rule_id"],
        "category": f["category"],
        "severity": f["severity"],
        "line": f["line"],
        "column": f["column"],
        "end_line": f.get("end_line", f["line"])
        if f.get("end_column") is not None
        else None,
        "end_column": f.get("end_column"),
        "span": f.get("span"),
        "matched_text": f["matched_text"],
        "message": f["message"],
        "ai_signal": f["ai_signal"],
        "replacement": replacement,
    }


def _sort_key(f: dict) -> tuple:
    return (
        f["line"],
        f["column"],
        f["end_line"] or 0,
        f["end_column"] or 0,
        f["rule_id"],
    )


def _unchecked(items: list) -> list[str]:
    return [i if isinstance(i, str) else i["message"] for i in items]


def project_document(doc: dict, data: bytes, schema_version: int) -> dict:
    if schema_version == 3:
        findings = [_finding_3(f) for f in doc["findings"]]
        gate = {
            "passed": doc["gate"]["passed"],
            "reasons": list(doc["gate"]["reasons"]),
        }
    else:
        findings = [_finding_2x(f, data) for f in doc["findings"]]
        gate = {"passed": doc["passed"], "reasons": list(doc["failure_reasons"])}
    findings.sort(key=_sort_key)
    return {
        "path": doc["path"],
        "profile": doc["profile"],
        "words": doc["words"],
        "sentences": doc["sentences"],
        "paragraphs": doc["paragraphs"],
        "total_findings": doc["total_findings"],
        "errors": doc["errors"],
        "warnings": doc["warnings"],
        "suggestions": doc["suggestions"],
        "per_100_words": doc["per_100_words"],
        "score": doc["score"],
        "gate": gate,
        "categories": sorted(doc["categories"], key=lambda c: c["category"]),
        "unchecked": _unchecked(doc["unchecked"]),
        "ai_register": doc["ai_register"],
        "prose": doc["prose"],
        "findings": findings,
    }


def project_report(report: dict, exit_code: int, files: dict[str, bytes]) -> dict:
    """Reduce a 2.x or v3 report to the fields both can state.

    Keys that only 3.0 produces (genre, contexts, config sources, provenance, pack)
    are outside the projection: the 2.x reference cannot ground them.
    """
    version = report["schema_version"]
    documents = sorted(
        (project_document(d, files[d["path"]], version) for d in report["documents"]),
        key=lambda d: d["path"],
    )
    return {
        "exit_code": exit_code,
        "summary": report["summary"],
        "documents": documents,
    }


# --- running a binary ------------------------------------------------------

FLAVORS = ("2x", "3x")


def config_text(mode: str, profile: str) -> str:
    """The 3.0 config a candidate runs with in each mode.

    ``parity`` and ``preserve`` run the reference-equivalent legacy configuration:
    the profile and the generated ``core:legacy-<profile>`` style, which carries the
    2.x category dials, rule defaults and STE tiers (docs/contract/profiles.md).
    ``preserve`` is the config that ``migrate --preserve-behavior`` writes for
    ``profile = "<profile>"``. ``policy`` runs the 3.0 default styles.
    """
    if mode in ("parity", "preserve"):
        return (
            f'schema_version = 3\nprofile = "{profile}"\n'
            f'styles = ["core:legacy-{profile}"]\n'
        )
    return f'schema_version = 3\nprofile = "{profile}"\n'


def config_for(mode: str, profile: str, flavor: str, scratch: Path) -> Path:
    """Write the run's config. A 2.x binary takes an empty config and ``--profile``."""
    path = scratch / f"{flavor}-{mode}-{profile}.toml"
    path.write_text(
        config_text(mode, profile) if flavor == "3x" else "", encoding="utf-8"
    )
    return path


def exit_for(doc: dict) -> int:
    """Exit status of a document alone: 2 when a check did not run, else its gate."""
    if doc["unchecked"]:
        return 2
    return 0 if doc["gate"]["passed"] else 1


def group_key(entry: dict) -> str:
    return f"{entry['surface']}:{entry['language']}"


def run_binary(
    command: str, mode: str, profile: str, inputs: list[dict], flavor: str = "3x"
) -> tuple[dict[str, dict], dict[str, dict]]:
    """Lint the inputs, one process for each surface and language.

    Returns the projection per input id and the run record per group: the process
    exit status and the run summary, which the document gates do not determine.
    """
    groups: dict[str, list[dict]] = {}
    for entry in inputs:
        groups.setdefault(group_key(entry), []).append(entry)
    results: dict[str, dict] = {}
    runs: dict[str, dict] = {}
    for key, entries in sorted(groups.items()):
        with tempfile.TemporaryDirectory(prefix="golden-") as tmp:
            scratch = Path(tmp)
            files: dict[str, bytes] = {}
            names: dict[str, str] = {}
            for entry in entries:
                name = staged_name(entry)
                shutil.copyfile(ROOT / entry["path"], scratch / name)
                files[name] = (ROOT / entry["path"]).read_bytes()
                names[name] = entry["id"]
            config = config_for(mode, profile, flavor, scratch)
            argv = [
                *shlex.split(command),
                "lint",
                *names,
                "--config",
                config.name,
                "--format",
                "json",
                "--mode",
                entries[0]["surface"],
            ]
            if flavor == "2x":
                argv += ["--profile", profile]
            proc = subprocess.run(
                argv,
                cwd=scratch,
                capture_output=True,
                text=True,
                check=False,
                env=execution_env(),
            )
            if proc.returncode not in (0, 1, 2) or not proc.stdout.strip():
                raise SystemExit(f"{command}: exit {proc.returncode}\n{proc.stderr}")
            report = json.loads(proc.stdout)
            projected = project_report(report, proc.returncode, files)
            for doc in projected["documents"]:
                results[names[doc["path"]]] = {
                    "exit_code": exit_for(doc),
                    "document": doc,
                }
            runs[key] = {"exit_code": proc.returncode, "summary": report["summary"]}
    return results, runs


# --- golden files ----------------------------------------------------------


def golden_path(profile: str, input_id: str) -> Path:
    return GOLDEN / "parity" / profile / f"{input_id}.json"


def runs_path(profile: str) -> Path:
    return GOLDEN / "runs" / f"{profile}.json"


def render_golden(value: dict) -> str:
    return json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"


def load_deltas() -> list[dict]:
    if not DELTAS_TOML.is_file():
        return []
    return tomllib.loads(DELTAS_TOML.read_text(encoding="utf-8")).get("delta", [])


DEFAULT_DELTA_MODES = ["policy"]


def deltas_for(mode: str, flavor: str) -> list[dict]:
    """The reviewed deltas that move the expectation for ``mode``.

    A delta lists the modes it applies to in ``modes`` (default ``["policy"]``).
    The 2.x reference never gets a delta: it produces the parity output by
    definition, and a delta describes how a 3.0 binary differs from it.
    """
    if flavor == "2x":
        return []
    return [
        d for d in load_deltas() if mode in d.get("modes", DEFAULT_DELTA_MODES)
    ]


def expected(mode: str, profile: str, input_id: str, flavor: str = "3x") -> dict:
    base = json.loads(golden_path(profile, input_id).read_text(encoding="utf-8"))
    return apply_deltas(base, profile, input_id, deltas_for(mode, flavor))


def expected_runs(mode: str, profile: str, flavor: str = "3x") -> dict:
    base = json.loads(runs_path(profile).read_text(encoding="utf-8"))
    return apply_run_deltas(base, profile, deltas_for(mode, flavor))


def apply_deltas(base: dict, profile: str, input_id: str, deltas: list[dict]) -> dict:
    """Policy expectation: parity output with reviewed per-finding deltas applied.

    A delta names one finding by rule id, line and column. ``removed`` drops it,
    ``added`` inserts ``finding``, and ``changed`` merges ``set`` into it. Counts,
    scores and gates are not patched here: a delta set that moves them records the
    new document-level values under ``document``. The expected exit status follows
    the resulting gate, so a delta that flips a gate flips the exit status too.
    """
    result = json.loads(json.dumps(base))
    doc = result["document"]
    for d in deltas:
        if d["input"] != input_id or d["profile"] != profile:
            continue
        findings = doc["findings"]
        if d["kind"] == "added":
            findings.append(d["finding"])
        else:
            if "column" not in d:
                raise ValueError(f"delta needs a column: {d}")
            hit = [
                f
                for f in findings
                if f["rule_id"] == d["rule_id"]
                and f["line"] == d["line"]
                and f["column"] == d["column"]
            ]
            if len(hit) != 1:
                raise ValueError(f"delta does not match one finding: {d}")
            if d["kind"] == "removed":
                findings.remove(hit[0])
            else:
                hit[0].update(d["set"])
        doc.update(d.get("document", {}))
        findings.sort(key=_sort_key)
    result["exit_code"] = exit_for(doc)
    return result


def apply_run_deltas(base: dict, profile: str, deltas: list[dict]) -> dict:
    """Patch the run record of a group with the ``run`` table of a delta."""
    result = json.loads(json.dumps(base))
    for d in deltas:
        if d["profile"] != profile or "run" not in d:
            continue
        patch = d["run"]
        group = result[patch["group"]]
        if "exit_code" in patch:
            group["exit_code"] = patch["exit_code"]
        group["summary"].update(patch.get("summary", {}))
    return result


# --- python source pin -----------------------------------------------------


def reference_tree_hash(src: Path) -> str:
    """Hash of the Python reference: its source and rule data files, by content.

    The file set is every ``.py``, ``.yml``, ``.yaml``, ``.toml`` and ``.json`` file
    under ``src``, outside ``__pycache__``. Files are visited in order of their
    path parts relative to ``src``. Each path enters the digest in POSIX form, so the
    host separator does not matter. Git state, timestamps and file modes do not.
    """
    digest = hashlib.sha256()
    for path in sorted(src.rglob("*"), key=lambda p: p.relative_to(src).parts):
        if (
            path.is_file()
            and path.suffix in {".py", ".yml", ".yaml", ".toml", ".json"}
            and "__pycache__" not in path.parts
        ):
            digest.update(path.relative_to(src).as_posix().encode())
            digest.update(b"\0")
            digest.update(path.read_bytes())
            digest.update(b"\0")
    return digest.hexdigest()


def execution_env() -> dict[str, str]:
    """The environment every reference and candidate run uses.

    The Python directory comes first, so the entry points of the reference (and
    ``rst2html`` from docutils) resolve there before anything else on ``PATH``.
    """
    path = os.pathsep.join([str(Path(sys.executable).parent), os.environ["PATH"]])
    return {**os.environ, "PATH": path}


def vale_version() -> str:
    """The version of the ``vale`` that a run would execute, found on the run PATH."""
    env = execution_env()
    binary = shutil.which("vale", path=env["PATH"])
    if binary is None:
        return ""
    proc = subprocess.run(
        [binary, "--version"], capture_output=True, text=True, check=False, env=env
    )
    return proc.stdout.strip().removeprefix("vale version ")


def current_pin(src: Path) -> dict:
    from importlib.metadata import version

    return {
        "slopvac_version": version("slopvac"),
        "python_src_sha256": reference_tree_hash(src),
        "vale": vale_version(),
    }


REFERENCE_SRC = ROOT / "packages" / "slopvac-lint" / "src" / "slopvac"


def verify_pin(src: Path = REFERENCE_SRC) -> list[str]:
    """What differs between pin.toml and this checkout.

    Every field is a version or a content hash, so the check reads no git state and
    gives the same answer in a full clone, a shallow clone and an exported tree.
    """
    recorded = tomllib.loads(PIN_TOML.read_text(encoding="utf-8"))
    problems = []
    for key, value in current_pin(src).items():
        if recorded.get(key) != value:
            problems.append(
                f"{key}: pinned {recorded.get(key)!r}, checkout has {value!r}"
            )
    return problems


def render_pin(pin: dict) -> str:
    lines = [
        "# Generated by scripts/contract/golden.py pin. Do not edit.",
        "# The reference the parity golden was recorded from. Changing it needs an approved reason.",
    ]
    lines += [f'{k} = "{v}"' for k, v in pin.items()]
    return "\n".join(lines) + "\n"


# --- commands --------------------------------------------------------------


def cmd_record(args: argparse.Namespace) -> int:
    problems = verify_pin()
    if problems:
        print("the reference checkout does not match pin.toml:", file=sys.stderr)
        for line in problems:
            print(f"  {line}", file=sys.stderr)
        print(
            "Run `golden.py pin` to move the pin, with an approved reason.",
            file=sys.stderr,
        )
        return 2
    inputs = load_inputs()
    for profile in PROFILES:
        results, runs = run_binary(args.reference, "parity", profile, inputs, "2x")
        for input_id, value in results.items():
            path = golden_path(profile, input_id)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(render_golden(value), encoding="utf-8")
        runs_path(profile).parent.mkdir(parents=True, exist_ok=True)
        runs_path(profile).write_text(render_golden(runs), encoding="utf-8")
    return 0


def compare(
    mode: str, command: str, only_profile: str | None = None, flavor: str = "3x"
) -> list[str]:
    inputs = load_inputs()
    problems: list[str] = []
    for profile in PROFILES:
        if only_profile and profile != only_profile:
            continue
        results, runs = run_binary(command, mode, profile, inputs, flavor)
        for entry in inputs:
            want = expected(mode, profile, entry["id"], flavor)
            got = mask_unverified(want, results.get(entry["id"]))
            if got != want:
                problems.append(
                    f"{mode}/{profile}/{entry['id']}: {describe(want, got)}"
                )
        problems += describe_runs(
            f"{mode}/{profile}", expected_runs(mode, profile, flavor), runs
        )
    return problems


def describe_runs(where: str, want: dict, got: dict) -> list[str]:
    problems = []
    for key in sorted(set(want) | set(got)):
        w, g = want.get(key), got.get(key)
        if w is None or g is None:
            problems.append(
                f"{where}: run {key} is {'missing' if g is None else 'unexpected'}"
            )
            continue
        if w["exit_code"] != g["exit_code"]:
            problems.append(
                f"{where}: run {key} exit status {g['exit_code']}, expected {w['exit_code']}"
            )
        if w["summary"] != g["summary"]:
            keys = sorted(
                k
                for k in set(w["summary"]) | set(g["summary"])
                if w["summary"].get(k) != g["summary"].get(k)
            )
            problems.append(f"{where}: run {key} summary differs in {keys}")
    return problems


def mask_unverified(want: dict, got: dict | None) -> dict | None:
    """Drop the end position from findings whose golden end was unverifiable."""
    if got is None:
        return None
    result = json.loads(json.dumps(got))
    keyed = {
        (f["rule_id"], f["line"], f["column"]): f
        for f in result["document"]["findings"]
    }
    for f in want["document"]["findings"]:
        if f["span"] is None and (
            hit := keyed.get((f["rule_id"], f["line"], f["column"]))
        ):
            hit.update(span=None, end_line=None, end_column=None)
    return result


def describe(want: dict, got: dict | None) -> str:
    if got is None:
        return "no document in the report"
    w, g = want["document"], got["document"]
    keys = sorted(k for k in set(w) | set(g) if w.get(k) != g.get(k))
    if "findings" in keys:
        wf = {(f["rule_id"], f["line"], f["column"]) for f in w["findings"]}
        gf = {(f["rule_id"], f["line"], f["column"]) for f in g["findings"]}
        return f"differs in {keys}; missing {sorted(wf - gf)[:3]}, extra {sorted(gf - wf)[:3]}"
    if want["exit_code"] != got["exit_code"]:
        keys.append("exit_code")
    return f"differs in {keys}"


def cmd_check(args: argparse.Namespace) -> int:
    problems = compare(args.mode, args.candidate, flavor=args.flavor)
    for p in problems:
        print(p, file=sys.stderr)
    return 1 if problems else 0


def cmd_diff(args: argparse.Namespace) -> int:
    inputs = load_inputs()
    problems = []
    for profile in PROFILES:
        a, runs_a = run_binary(args.a, args.mode, profile, inputs, args.flavor)
        b, runs_b = run_binary(args.b, args.mode, profile, inputs, args.flavor)
        for entry in inputs:
            if a.get(entry["id"]) != b.get(entry["id"]):
                problems.append(
                    f"{args.mode}/{profile}/{entry['id']}: "
                    f"{describe(a[entry['id']], b.get(entry['id']))}"
                )
        problems += describe_runs(f"{args.mode}/{profile}", runs_a, runs_b)
    for p in problems:
        print(p)
    return 1 if problems else 0


def cmd_inputs(_: argparse.Namespace) -> int:
    write_rule_examples()
    INPUTS_TOML.write_text(render_inputs(input_entries()), encoding="utf-8")
    return 0


def cmd_pin(args: argparse.Namespace) -> int:
    PIN_TOML.write_text(render_pin(current_pin(REFERENCE_SRC)), encoding="utf-8")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("record")
    p.add_argument("--reference", required=True)
    p.set_defaults(func=cmd_record)
    p = sub.add_parser("check")
    p.add_argument("--candidate", required=True)
    p.add_argument("--mode", choices=MODES, default="parity")
    p.add_argument("--flavor", choices=FLAVORS, default="3x")
    p.set_defaults(func=cmd_check)
    p = sub.add_parser("diff")
    p.add_argument("--a", required=True)
    p.add_argument("--b", required=True)
    p.add_argument("--mode", choices=MODES, default="parity")
    p.add_argument("--flavor", choices=FLAVORS, default="3x")
    p.set_defaults(func=cmd_diff)
    sub.add_parser("inputs").set_defaults(func=cmd_inputs)
    sub.add_parser("pin").set_defaults(func=cmd_pin)
    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
