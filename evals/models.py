"""Model evals per review type: mechanical, update and rewrite.

A model run is one prompt sent to one model, scored locally. This module builds
the tasks, scores the answers and summarises them. It never calls a model on
its own. You give `run` a models file that names a command per model, and the
command reads the prompt on stdin and writes the answer on stdout.

    python -m evals models plan --models models.json --repeats 3
    python -m evals models run  --models models.json --repeats 3 --confirm-calls <N>
    python -m evals models summarize evals/results/models/<date>.json

`plan` prints the number of calls and a rough input size per model, so the person
who pays can approve them. `run` refuses to start unless `--confirm-calls`
equals the planned count.

Tasks
- author / mechanical: fix one flagged sentence and nothing else.
- author / update: apply one stated change to a passage.
- author / rewrite: rewrite a passage in clearer prose and keep every fact.
- reviewer / planted: find the facts a candidate lost or changed, when we planted
  the damage. The reviewer sees the original and the candidate.
- reviewer / authored: review another model's rewrite. Truth is the lexical
  alignment in `evals.facts`. The reviewer model differs from the author model.

Author tasks score pass or fail, and fact preservation is a number from 0 to 1.
Repeat each run at least three times, because one sample of a model says little.
"""

from __future__ import annotations

import datetime as dt
import fcntl
import hashlib
import json
import os
import random
import re
import shlex
import subprocess
import tempfile
import threading
from collections import defaultdict
from collections.abc import Callable, Iterable, Mapping, Sequence
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import Any, NamedTuple, Self

from . import corpuslib as lib
from . import facts, metrics, procs, runner
from .textops import prose_paragraphs

PROMPT_VERSION = "1"
RESULTS_SCHEMA = 1
RESULTS_DIR = lib.EVALS_DIR / "results" / "models"
MIN_REPEATS = 3
TYPES = ("mechanical", "update", "rewrite")
SEED = "slopvac-evals-models-1"
FALSE_ALARM_LIMIT = 1
QUOTE_OVERLAP = 0.6
MIN_QUOTE_WORDS = 3

AUTHOR_PROMPTS = {
    "mechanical": """\
Fix exactly one prose problem in the passage and change nothing else.

Problem: {about}
Flagged sentence: {sentence!r}

Passage:
{passage}

Output only the corrected passage.
""",
    "update": """\
Update the passage with the change below. Keep every other fact, number, identifier and requirement as it is.

Change: replace {old} with {new} where the passage states it.

Passage:
{passage}

Output only the updated passage.
""",
    "rewrite": """\
Rewrite the passage in plain, direct prose. Keep every number, identifier, code span and requirement. Do not add facts.

Passage:
{passage}

Output only the rewritten passage.
""",
}
REVIEW_PROMPT = """\
You compare an original passage with a candidate rewrite. List every fact the candidate lost or changed: numbers, identifiers, code spans, and requirements such as must, should or not.

Original:
{original}

Candidate:
{candidate}

Answer with a JSON list. Each item has "quote" (the exact words from either text) and "problem". Answer [] when the candidate keeps every fact.
"""


class ModelEvalError(Exception):
    pass


# --- tasks -----------------------------------------------------------------------


def task_id(*parts: str) -> str:
    return hashlib.sha256("|".join(parts).encode()).hexdigest()[:12]


def _read(doc: Mapping[str, Any], cache: Path) -> str:
    return runner.text_path(doc, cache).read_text("utf-8")


def build_tasks(
    manifest: Mapping[str, Any],
    cache: Path,
    *,
    per_type: int = 12,
    notes: Mapping[str, str] | None = None,
    seed: str = SEED,
) -> list[dict[str, Any]]:
    """Build a deterministic task set from the corpus."""
    notes = notes or {}
    docs = {d["id"]: d for d in manifest["documents"]}
    rng = random.Random(seed)
    tasks: list[dict[str, Any]] = []

    seeded = sorted(
        (d for d in docs.values() if d["origin"] == "seeded-defect"),
        key=lambda d: d["id"],
    )
    pool = []
    for doc in seeded:
        for defect in doc["source"]["defects"]:
            pool.append((doc, defect))
    rng.shuffle(pool)
    for doc, defect in pool[:per_type]:
        lines = _read(doc, cache).split("\n")
        at = defect["line"] - 1
        first, last = max(0, at - 2), min(len(lines), at + 3)
        passage = "\n".join(lines[first:last])
        tasks.append(
            {
                "id": task_id(
                    "mechanical", doc["id"], str(defect["line"]), defect["rule"]
                ),
                "type": "mechanical",
                "role": "author",
                "doc": doc["id"],
                "prompt": AUTHOR_PROMPTS["mechanical"].format(
                    about=notes.get(defect["rule"], defect["rule"]),
                    sentence=defect["text"],
                    passage=passage,
                ),
                "source": passage,
                "rule": defect["rule"],
                "genre": doc["genre"],
            }
        )

    pairs = sorted(
        (d for d in docs.values() if d["origin"] == "rewrite-pair"),
        key=lambda d: d["id"],
    )
    updates = []
    for doc in pairs:
        for edit in doc["source"]["edits"]:
            if edit["kind"] == "number":
                old, new = (part.strip() for part in edit["detail"].split("->"))
                updates.append((doc, edit, old, new))
    rng.shuffle(updates)  # shuffle first: the rng stream must not depend on the filter
    usable = [u for u in updates if _stands_alone(u[1]["original_sentence"], u[2])]
    for doc, edit, old, new in usable[:per_type]:
        passage = edit["original_sentence"]
        tasks.append(
            {
                "id": task_id("update", doc["id"], str(edit["line"]), old, new),
                "type": "update",
                "role": "author",
                "doc": doc["id"],
                "prompt": AUTHOR_PROMPTS["update"].format(
                    old=old, new=new, passage=passage
                ),
                "source": passage,
                "expected": [{"kind": "number", "old": old, "new": new}],
                "genre": doc["genre"],
            }
        )

    humans = sorted(
        (
            d
            for d in docs.values()
            if d["origin"] == "human" and d["format"] == "markdown"
        ),
        key=lambda d: d["id"],
    )
    rewrites = []
    for doc in humans:
        if not (
            doc.get("committed") or (cache / "docs" / f"{doc['id']}.txt").is_file()
        ):
            continue
        text = _read(doc, cache)
        lines = text.split("\n")
        for first, last in prose_paragraphs(text):
            passage = "\n".join(lines[first : last + 1])
            if (
                40 <= len(passage.split()) <= 160
                and facts.preservation(passage, passage)["facts"]
            ):
                rewrites.append((doc, passage, first + 1))
    rng.shuffle(rewrites)
    for doc, passage, line in rewrites[:per_type]:
        tasks.append(
            {
                "id": task_id("rewrite", doc["id"], str(line)),
                "type": "rewrite",
                "role": "author",
                "doc": doc["id"],
                "prompt": AUTHOR_PROMPTS["rewrite"].format(passage=passage),
                "source": passage,
                "genre": doc["genre"],
            }
        )

    for doc in pairs:
        parent = docs[doc["source"]["parents"][0]]
        original, candidate = _read(parent, cache), _read(doc, cache)
        tasks.append(
            {
                "id": task_id("planted", doc["id"]),
                "type": "rewrite",
                "role": "reviewer",
                "variant": "planted",
                "doc": doc["id"],
                "prompt": REVIEW_PROMPT.format(original=original, candidate=candidate),
                "edits": [
                    {"kind": e["kind"], "sentence": e["original_sentence"]}
                    for e in doc["source"]["edits"]
                ],
                "genre": doc["genre"],
            }
        )
    return tasks


def _stands_alone(passage: str, old: str) -> bool:
    """True when `old` is a bare number fact and sits in no code span or identifier.

    A change inside a code span or issue reference cannot be written as a number
    change, so such a passage makes an ill-posed update task.
    """
    found = facts.extract(passage)
    embedded = re.compile(rf"(?<!\d){re.escape(old)}(?!\d)")
    return found["number"][old] > 0 and not any(
        embedded.search(fact) for kind in ("code", "identifier") for fact in found[kind]
    )


def tasks_digest(tasks: Sequence[Mapping[str, Any]]) -> str:
    """Hash the whole task set: ids, prompts, sources, expected edits and rule data."""
    return lib.sha256_text(json.dumps(list(tasks), sort_keys=True))


# --- scoring ---------------------------------------------------------------------

Linter = Callable[[str], list[dict[str, Any]]]


class LintIncomplete(ModelEvalError):
    """The engine could not check every rule, so "no findings" proves nothing."""


def make_linter(engine: runner.Engine, suffix: str = ".md") -> Linter:
    """Lint a text with the engine and return slim findings.

    Raises `LintIncomplete` when the engine reports unchecked rules (its exit 2),
    because a rule that did not run must never count as cleared.
    """

    def lint(text: str) -> list[dict[str, Any]]:
        with tempfile.TemporaryDirectory(prefix="slopvac-evals-") as tmp:
            target = Path(tmp) / f"candidate{suffix}"
            target.write_text(text, encoding="utf-8")
            staged = runner.Engine(
                engine.name, engine.command, Path(tmp), engine.unsupported
            )
            report = staged.lint([target])
        unchecked = [
            str(note) for d in report["documents"] for note in d.get("unchecked") or []
        ]
        if unchecked:
            raise LintIncomplete(f"lint incomplete: {'; '.join(unchecked)[:300]}")
        return [runner._slim(f) for d in report["documents"] for f in d["findings"]]

    return lint


def _clean(answer: str) -> str:
    """Strip a code fence a model wrapped around the passage."""
    text = answer.strip()
    m = re.match(r"^```[\w-]*\n(.*?)\n```$", text, re.DOTALL)
    return m.group(1).strip() if m else text


def score_author(task: Mapping[str, Any], answer: str, lint: Linter) -> dict[str, Any]:
    output = _clean(answer)
    expected = task.get("expected", [])
    pres = facts.preservation(task["source"], output, expected)
    words = max(len(output.split()), 1)
    out: dict[str, Any] = {
        "fact_rate": pres["rate"],
        "facts_lost": len(pres["lost"]),
        "words": words,
        "empty": not output,
    }
    if task["type"] == "mechanical":
        fired = any(f["rule"] == task["rule"] for f in lint(output))
        out["rule_cleared"] = not fired
        out["success"] = bool(output) and not fired and not pres["lost"]
    elif task["type"] == "update":
        out["change_applied"] = pres["changes_applied"] == pres["changes_expected"]
        out["success"] = bool(output) and out["change_applied"] and not pres["lost"]
    else:
        before = len(lint(task["source"])) / max(len(task["source"].split()), 1)
        after = len(lint(output)) / words
        out["slop_before_per_word"] = before
        out["slop_after_per_word"] = after
        out["success"] = bool(output) and not pres["lost"] and after <= before
    return out


def _words(text: str) -> set[str]:
    return {w for w in re.findall(r"[a-z0-9_]+", text.lower()) if len(w) > 1}


def _parse_findings(answer: str) -> list[dict[str, str]]:
    m = re.search(r"\[.*\]", answer, re.DOTALL)
    if not m:
        return []
    try:
        items = json.loads(m.group(0))
    except json.JSONDecodeError:
        return []
    return [
        {"quote": str(i.get("quote", "")), "problem": str(i.get("problem", ""))}
        for i in items
        if isinstance(i, dict)
    ]


def _hits(quote: str, sentence: str) -> bool:
    q, s = _words(quote), _words(sentence)
    return len(q) >= MIN_QUOTE_WORDS and len(q & s) / len(q) >= QUOTE_OVERLAP


def _mentions(fact: str, text: str) -> bool:
    """True when `fact` appears as a complete token.

    A number must not sit inside a longer number ("3" in "13", "3.5" or "3,500").
    An identifier or flag must not continue ("--offline" in "--offline-mode").
    """
    body = re.escape(fact.lower())
    if re.fullmatch(r"\d+(?:[.,]\d+)*%?", fact):
        pattern = rf"(?<![\w])(?<!\d[.,]){body}(?!\w)(?![.,]\d)"
    else:
        pattern = rf"(?<![\w-]){body}(?![\w-])(?!\.\w)"
    return re.search(pattern, text.lower()) is not None


def score_review(task: Mapping[str, Any], answer: str) -> dict[str, Any]:
    reported = _parse_findings(answer)
    if task.get("variant") == "authored":
        lost = task["lost"]
        found = [
            f
            for f in lost
            if any(_mentions(f, r["quote"] + " " + r["problem"]) for r in reported)
        ]
        edits_total, edits_found = len(lost), len(found)
        alarms = sum(
            1
            for r in reported
            if not any(_mentions(f, r["quote"] + " " + r["problem"]) for f in lost)
        )
    else:
        edits = task["edits"]
        matched = [
            any(_hits(r["quote"], e["sentence"]) for r in reported) for e in edits
        ]
        edits_total, edits_found = len(edits), sum(matched)
        alarms = sum(
            1
            for r in reported
            if not any(_hits(r["quote"], e["sentence"]) for e in edits)
        )
    recall = edits_found / edits_total if edits_total else None
    return {
        "recall": recall,
        "found": edits_found,
        "planted": edits_total,
        "false_alarms": alarms,
        "success": edits_total > 0
        and edits_found == edits_total
        and alarms <= FALSE_ALARM_LIMIT,
    }


def authored_review_task(
    author_task: Mapping[str, Any], author_output: str
) -> dict[str, Any]:
    """Turn an author rewrite into a review task with the facts it lost."""
    output = _clean(author_output)
    lost = [
        item["fact"]
        for item in facts.preservation(author_task["source"], output)["lost"]
    ]
    return {
        # The id follows the source task only. Resampling needs a key that does not
        # change with what a model happened to write.
        "id": task_id("authored", author_task["id"]),
        "type": "rewrite",
        "role": "reviewer",
        "variant": "authored",
        "doc": author_task["doc"],
        "prompt": REVIEW_PROMPT.format(
            original=author_task["source"], candidate=output
        ),
        "lost": lost,
        "genre": author_task["genre"],
    }


# --- models and runs ----------------------------------------------------------------


def load_models(path: Path) -> list[dict[str, Any]]:
    data = json.loads(path.read_text("utf-8"))
    models = data["models"]
    seen: set[str] = set()
    for m in models:
        for key in ("name", "family", "tier", "command"):
            if not m.get(key):
                raise ModelEvalError(f"{path}: every model needs {key!r}")
        if m["name"] in seen:
            raise ModelEvalError(f"{path}: duplicate model {m['name']!r}")
        seen.add(m["name"])
    return models


def plan(
    tasks: Sequence[Mapping[str, Any]],
    models: Sequence[Mapping[str, Any]],
    repeats: int,
) -> dict[str, Any]:
    """Count the calls a run makes, so the payer can approve them first."""
    if repeats < MIN_REPEATS:
        raise ModelEvalError(f"repeats must be at least {MIN_REPEATS}")
    authors = [t for t in tasks if t["role"] == "author"]
    planted = [t for t in tasks if t["role"] == "reviewer"]
    chars = sum(len(t["prompt"]) for t in authors)
    planted_chars = sum(len(t["prompt"]) for t in planted)
    per_model = {}
    for m in models:
        per_model[m["name"]] = {
            "author_calls": len(authors) * repeats,
            "planted_review_calls": len(planted) * repeats,
            "input_tokens_estimate": (chars + planted_chars) * repeats // 4,
        }
    # Each rewrite answer is also reviewed once per other model.
    rewrites = [t for t in authors if t["type"] == "rewrite"]
    cross = len(rewrites) * repeats * len(models) * max(len(models) - 1, 0)
    calls = (
        sum(v["author_calls"] + v["planted_review_calls"] for v in per_model.values())
        + cross
    )
    return {
        "tasks": len(tasks),
        "models": len(models),
        "repeats": repeats,
        "per_model": per_model,
        "authored_review_calls": cross,
        "total_calls": calls,
    }


class BudgetExceeded(ModelEvalError):
    """The run would exceed its invocation ceiling. Nothing more is spawned."""


class CallFailed(ModelEvalError):
    """A model command failed on every attempt."""

    def __init__(self, message: str, attempts: int) -> None:
        super().__init__(message)
        self.attempts = attempts


class Cancelled(BaseException):
    """The run was interrupted. Like KeyboardInterrupt, nothing catches it on the way up."""


class Reply(NamedTuple):
    text: str
    attempts: int


class Budget:
    """A ceiling on model-command invocations, retries included, kept on disk.

    Every `reserve` is appended to a ledger and fsynced BEFORE a process starts,
    so a crash between the reservation and the spawn still counts as spent. On
    resume the spent count is the number of reservations in the ledger. An
    exclusive lock on `<ledger>.lock` is held for the life of the run: a second
    live run on the same ledger refuses to start, which stops independent
    launches from each spending the full ceiling.

    With no ledger path the budget lives in memory only, which suits tests.
    """

    def __init__(self, ceiling: int, ledger: Path | str | None = None) -> None:
        ledger = Path(ledger) if ledger is not None else None
        self.ceiling = ceiling
        self.ledger = ledger
        self.spent = 0
        self.unfinished_at_start = 0
        self._lock = threading.Lock()
        self._fd: int | None = None
        self._handle: Any = None
        if ledger is not None:
            self._open(ledger)

    def _open(self, ledger: Path) -> None:
        ledger.parent.mkdir(parents=True, exist_ok=True)
        self._fd = os.open(str(ledger) + ".lock", os.O_CREAT | os.O_RDWR, 0o600)
        try:
            fcntl.flock(self._fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            os.close(self._fd)
            self._fd = None
            raise ModelEvalError(
                f"another run holds the ledger lock for {ledger.name}; refusing to start"
            ) from None
        reserved: set[int] = set()
        finished: set[int] = set()
        if ledger.is_file():
            for line in ledger.read_text("utf-8").splitlines():
                if not line:
                    continue
                event = json.loads(line)
                (reserved if event["event"] == "reserve" else finished).add(event["n"])
        self.spent = len(reserved)
        self.unfinished_at_start = len(reserved - finished)
        self._handle = ledger.open("a", encoding="utf-8")

    def _append(self, event: dict[str, Any]) -> None:
        if self._handle is not None:
            self._handle.write(json.dumps(event, sort_keys=True) + "\n")
            self._handle.flush()
            os.fsync(self._handle.fileno())

    def reserve(self) -> int:
        """Persist one reservation, then return its number. Raises at the ceiling."""
        with self._lock:
            if self.spent >= self.ceiling:
                raise BudgetExceeded(
                    f"invocation ceiling of {self.ceiling} reached; nothing more was started"
                )
            self.spent += 1
            self._append({"event": "reserve", "n": self.spent})
            return self.spent

    def finish(self, number: int, status: str) -> None:
        with self._lock:
            self._append({"event": "finish", "n": number, "status": status})

    def close(self) -> None:
        with self._lock:
            if self._handle is not None:
                self._handle.close()
                self._handle = None
            if self._fd is not None:
                os.close(self._fd)  # closing the descriptor drops the lock
                self._fd = None

    def __enter__(self) -> Self:
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()


def job_id(kind: str, model: str, prompt: str, repeat: int, *scope: str) -> str:
    """A stable id: the same model, prompt, repeat and scope always map to one job.

    `scope` separates jobs whose prompts match but whose samples must stay
    independent, such as one candidate written by several author models.
    """
    digest = hashlib.sha256(prompt.encode()).hexdigest()
    key = "|".join([kind, model, digest, str(repeat), *scope])
    return hashlib.sha256(key.encode()).hexdigest()[:16]


class Journal:
    """Append-only record of finished jobs, so a stopped run resumes without repeats.

    Each line holds a job id, the answer, any error and the attempts it used. The
    last line for a job wins. A job that failed is asked again on resume, and its
    earlier attempts still count against the budget.
    """

    def __init__(self, path: Path) -> None:
        self.path = path
        self._lock = threading.Lock()
        self._entries: dict[str, dict[str, Any]] = {}
        self._attempts = 0
        if path.is_file():
            for line in path.read_text("utf-8").splitlines():
                if line:
                    entry = json.loads(line)
                    self._entries[entry["job"]] = entry
                    self._attempts += entry["attempts"]

    @property
    def attempts(self) -> int:
        return self._attempts

    def done(self, job: str) -> dict[str, Any] | None:
        entry = self._entries.get(job)
        return entry if entry and not entry.get("error") else None

    def record(self, job: str, answer: str, error: str | None, attempts: int) -> None:
        entry = {"job": job, "answer": answer, "error": error, "attempts": attempts}
        with self._lock:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            with self.path.open("a", encoding="utf-8") as fh:
                fh.write(json.dumps(entry, sort_keys=True) + "\n")
                fh.flush()
                os.fsync(fh.fileno())
            self._entries[job] = entry


def command_ask(
    command: str,
    attempts: int = 3,
    *,
    budget: Budget | None = None,
    timeout: int = procs.OUTER_TIMEOUT,
    live: procs.LiveGroups | None = None,
) -> Callable[[str], Reply]:
    argv = shlex.split(command)

    def ask(prompt: str) -> Reply:
        last = ""
        used = 0
        for _ in range(attempts):
            if live and live.cancelled.is_set():
                raise Cancelled
            number = budget.reserve() if budget else 0
            used += 1
            try:
                proc = procs.run_group(argv, input=prompt, timeout=timeout, live=live)
            except subprocess.TimeoutExpired:
                if budget:
                    budget.finish(number, "timeout")
                last = f"model command timed out after {timeout}s"
                continue
            except BaseException:
                if budget:
                    budget.finish(number, "interrupted")
                raise
            if live and live.cancelled.is_set():
                if budget:
                    budget.finish(number, "interrupted")
                raise Cancelled
            ok = proc.returncode == 0 and bool(proc.stdout.strip())
            if budget:
                budget.finish(number, "ok" if ok else "fail")
            if ok:
                return Reply(proc.stdout, used)
            last = procs.redact(
                f"model command exited {proc.returncode}: {proc.stderr[:300]}"
            )
        raise CallFailed(last, used)

    return ask


def _row(
    task: Mapping[str, Any], model: Mapping[str, Any], repeat: int, answer: str
) -> dict[str, Any]:
    return {
        "task": task["id"],
        "type": task["type"],
        "role": task["role"],
        "variant": task.get("variant"),
        "model": model["name"],
        "family": model["family"],
        "tier": model["tier"],
        "repeat": repeat,
        "output_sha256": hashlib.sha256(answer.encode()).hexdigest(),
    }


def _safe(
    ask: Callable[[str], Any], job: str, prompt: str, journal: Journal | None
) -> tuple[str, str | None]:
    """Ask once, journal the outcome, and turn a failed call into an empty answer."""
    if journal and (entry := journal.done(job)):
        return entry["answer"], None
    answer, error, attempts = "", None, 1
    try:
        reply = ask(prompt)
        if isinstance(reply, Reply):
            answer, attempts = reply.text, reply.attempts
        else:
            answer = reply
    except BudgetExceeded:
        raise
    except CallFailed as exc:
        error, attempts = str(exc)[:300], exc.attempts
    except ModelEvalError as exc:
        error = str(exc)[:300]
    if journal:
        journal.record(job, answer, error, attempts)
    return answer, error


def _map_jobs(
    fn: Callable[[Any], Any], jobs: Sequence[Any], workers: int, live: procs.LiveGroups
) -> list[Any]:
    """Run `fn` over `jobs` on a thread pool, in order.

    On Ctrl-C the pending jobs are cancelled and the running model process groups
    are killed BEFORE the pool is waited on, so the interrupt returns promptly and
    leaves no model command behind. Any other error lets running jobs finish.
    """
    pool = ThreadPoolExecutor(max_workers=max(workers, 1))
    futures: list[Any] = []
    try:
        # Submission sits inside the try: an interrupt part-way through still
        # cancels the futures submitted so far and kills the started commands.
        for job in jobs:
            futures.append(pool.submit(fn, job))
        return [f.result() for f in futures]
    except KeyboardInterrupt:
        for f in futures:
            f.cancel()
        live.cancel()
        raise
    finally:
        pool.shutdown(wait=True, cancel_futures=False)


def execute(
    tasks: Sequence[Mapping[str, Any]],
    models: Sequence[Mapping[str, Any]],
    repeats: int,
    lint: Linter,
    *,
    ask_for: Callable[[Mapping[str, Any]], Callable[[str], Any]] | None = None,
    date: str | None = None,
    workers: int = 1,
    budget: Budget | None = None,
    journal: Journal | None = None,
) -> dict[str, Any]:
    """Run every task against every model; reviewers see other models' rewrites.

    Calls run on `workers` threads. A failed call is recorded as an empty answer
    with its error, so it counts as a failure and the run goes on. A `journal`
    stores every finished call, and a `budget` stops the run before it spawns a
    process past the ceiling; `BudgetExceeded` then propagates and the journal
    keeps all finished work.
    """
    expected = plan(tasks, models, repeats)
    live = procs.LiveGroups()
    make_ask = ask_for or (
        lambda m: command_ask(m["command"], budget=budget, live=live)
    )
    asks = {m["name"]: make_ask(m) for m in models}
    first = [
        (model, task, repeat)
        for model in models
        for task in tasks
        for repeat in range(repeats)
    ]

    def run_first(job: tuple[Any, Any, int]) -> tuple[str, str | None]:
        model, task, repeat = job
        return _safe(
            asks[model["name"]],
            job_id("task", model["name"], task["prompt"], repeat),
            task["prompt"],
            journal,
        )

    answers = _map_jobs(run_first, first, workers, live)
    runs: list[dict[str, Any]] = []
    rewrite_outputs: list[tuple[Mapping[str, Any], Mapping[str, Any], int, str]] = []
    for (model, task, repeat), (answer, error) in zip(first, answers, strict=True):
        row = _row(task, model, repeat, answer)
        if error:
            row["error"] = error
        if task["role"] == "author":
            try:
                row["score"] = score_author(task, answer, lint)
            except LintIncomplete as exc:
                # Neither a pass nor a fail: the summary leaves the run out.
                row["score"] = None
                row["unscored"] = str(exc)
            if task["type"] == "rewrite":
                rewrite_outputs.append((task, model, repeat, answer))
        else:
            row["score"] = score_review(task, answer)
        runs.append(row)
    second = []
    for task, author, repeat, answer in rewrite_outputs:
        review_task = authored_review_task(task, answer)
        for reviewer in models:
            if reviewer["name"] != author["name"]:
                second.append((review_task, author, reviewer, repeat))

    def run_second(job: tuple[Any, Any, Any, int]) -> tuple[str, str | None]:
        review_task, author, reviewer, repeat = job
        return _safe(
            asks[reviewer["name"]],
            job_id(
                "review",
                reviewer["name"],
                review_task["prompt"],
                repeat,
                author["name"],
                review_task["id"],
            ),
            review_task["prompt"],
            journal,
        )

    replies = _map_jobs(run_second, second, workers, live)
    for (review_task, author, reviewer, repeat), (reply, error) in zip(
        second, replies, strict=True
    ):
        row = _row(review_task, reviewer, repeat, reply)
        row["task"] = f"{review_task['id']}:{author['name']}"
        row["author_model"] = author["name"]
        row["author_family"] = author["family"]
        if error:
            row["error"] = error
        row["score"] = score_review(review_task, reply) | {
            "lost_facts": len(review_task["lost"])
        }
        runs.append(row)
    return {
        "schema": RESULTS_SCHEMA,
        "date": date or dt.date.today().isoformat(),
        "prompt_version": PROMPT_VERSION,
        "tasks_sha256": tasks_digest(tasks),
        "repeats": repeats,
        "planned_calls": expected["total_calls"],
        "failed_calls": sum(1 for r in runs if r.get("error")),
        "unscored_runs": sum(1 for r in runs if r.get("unscored")),
        "invocations": budget.spent if budget else None,
        "models": [
            {"name": m["name"], "family": m["family"], "tier": m["tier"]}
            for m in models
        ],
        "runs": runs,
    }


# --- summary -----------------------------------------------------------------------


def summarize(results: Mapping[str, Any], *, resamples: int = 1000) -> dict[str, Any]:
    """Success rate per review type, role and model, with a task-cluster interval.

    Repeats of one task are averaged first, so the task is the unit of resampling.
    Reviewer runs that grade a rewrite with no lost facts are excluded: there
    was nothing to find. Unscored runs (an incomplete lint) are left out too.
    """
    groups: dict[tuple[str, str, str], dict[str, list[float]]] = defaultdict(
        lambda: defaultdict(list)
    )
    info: dict[str, dict[str, str]] = {m["name"]: m for m in results["models"]}
    for run in results["runs"]:
        score = run["score"]
        if score is None:
            continue
        if (
            run["role"] == "reviewer"
            and run.get("variant") == "authored"
            and not score["planted"]
        ):
            continue
        key = (run["type"], run["role"], run["model"])
        groups[key][run["task"]].append(1.0 if score["success"] else 0.0)
    rows = []
    for (rtype, role, model), per_task in sorted(groups.items()):
        means = [sum(v) / len(v) for v in per_task.values()]
        boot = metrics.cluster_bootstrap(
            means, lambda s: sum(s) / len(s) if s else None, resamples=resamples
        )
        rows.append(
            {
                "type": rtype,
                "role": role,
                "model": model,
                "family": info[model]["family"],
                "tier": info[model]["tier"],
                "tasks": len(per_task),
                "runs": sum(len(v) for v in per_task.values()),
                "success_rate": boot["point"],
                "interval": [boot["low"], boot["high"]],
            }
        )
    ranking: dict[str, list[str]] = {}
    for rtype in TYPES:
        for role in ("author", "reviewer"):
            ordered = sorted(
                (r for r in rows if r["type"] == rtype and r["role"] == role),
                key=lambda r: (-r["success_rate"], r["model"]),
            )
            if ordered:
                ranking[f"{rtype}/{role}"] = [r["model"] for r in ordered]
    return {
        "schema": RESULTS_SCHEMA,
        "date": results["date"],
        "prompt_version": results["prompt_version"],
        "repeats": results["repeats"],
        "rows": rows,
        "ranking": ranking,
    }


def today() -> str:
    return dt.date.today().isoformat()


def default_output(date: str) -> Path:
    return RESULTS_DIR / f"{date}.json"


def load(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text("utf-8"))
    if data.get("schema") != RESULTS_SCHEMA:
        raise ModelEvalError(f"{path}: unsupported schema {data.get('schema')!r}")
    return data


def dump(data: Mapping[str, Any], path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=1, sort_keys=True) + "\n", encoding="utf-8")


def families(models: Iterable[Mapping[str, Any]]) -> set[str]:
    return {m["family"] for m in models}
