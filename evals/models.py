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
from collections import Counter, defaultdict
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
                "passage": original + "\n" + candidate,
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


def _squash(text: str) -> str:
    return " ".join(text.split())


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
        "metric": AUTHOR_METRIC,
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
        # Echoing the input keeps every fact and adds no slop, so it must not pass.
        out["unchanged"] = _squash(output) == _squash(task["source"])
        out["success"] = (
            bool(output)
            and not out["unchanged"]
            and not pres["lost"]
            and after <= before
        )
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


REVIEW_METRIC = "detection by quotation"
AUTHOR_METRIC = "lexical retention smoke check"
# Bump when scoring changes. Rows scored before 15.17 carry no version and are
# "legacy": they were never rescored, so they must not take the new metric names.
SCORER_VERSION = "15.17"
LEGACY_SCORER = "legacy"
LEGACY_METRIC = "legacy (pre-15.17)"

# A finding that denies the loss is not a finding. These are plain phrase lists, not
# a meaning-level oracle. A denial counts only when nothing else in the explanation
# asserts a change, so "still says 63, but the candidate changed it" is kept.
_KEPT = (
    r"(?:preserved|kept|retained|intact|unchanged|present|correct"
    r"|says?|states?|reads?|has|contains?|includes?|keeps?|retains?|preserves?)"
)
NEGATED_RE = re.compile(
    rf"(?:\bnot|\bnever|\bno\s+longer|n['\u2019]t)\s+(?:been\s+|be\s+)?{_KEPT}\b",
    re.IGNORECASE,
)
CHANGE_RE = re.compile(
    r"\b(?:chang(?:ed|es)|now|instead|replaced|became|becomes|differs?|different"
    r"|lost|loses|dropped|missing|removed|omitted|altered|swapped|deleted"
    r"|vanished|gone|wrong|incorrect|increased|decreased)\b",
    re.IGNORECASE,
)
DENIAL_RE = re.compile(
    r"\b(?:is|are|was|were|remains?|stays?|been)\s+"
    r"(?:preserved|kept|retained|intact|correct|unchanged|present)\b"
    r"|\bunchanged\b|\bno\s+(?:change|difference|loss|problem|issue)s?\b"
    r"|\bnot\s+(?:changed|lost|missing|dropped|altered)\b"
    r"|\bnothing\s+(?:is\s+)?(?:lost|missing|changed)\b"
    r"|\bstill\s+(?:says|states|reads|has|contains|includes)\b"
    r"|\bcandidate\s+(?:still\s+)?(?:keeps|retains|preserves|contains|includes|has)\b"
    r"|\bsame\s+as\s+the\s+original\b",
    re.IGNORECASE,
)


def _asserts_change(problem: str) -> bool:
    """True unless the explanation only denies the loss or change.

    A negated denial ("not preserved", "no longer says") asserts a change. A plain
    denial is overruled by a contrasting assertion left once the denial is removed.
    """
    if NEGATED_RE.search(problem):
        return True
    rest = DENIAL_RE.sub(" ", problem)
    if rest == problem:
        return True  # no denial phrase at all
    return CHANGE_RE.search(rest) is not None


def _norm(text: str) -> str:
    """Lowercase, drop markdown code ticks, unify quotes and collapse whitespace."""
    text = text.replace("`", "").replace("\u2019", "'").replace("\u2018", "'")
    text = text.replace("\u201c", '"').replace("\u201d", '"')
    return " ".join(text.lower().split())


def _in_passage(quote: str, passage: str) -> bool:
    """True when the quote appears word for word, in order, in the passage.

    Matching is normalised and contiguous, and the quote must start and end on a
    token edge, so "3" is not found inside "13". It has no minimum length: a
    complete fact token such as `--offline` or `63` is a genuine quote.
    """
    needle = _norm(quote).strip(" .,;:!?\"'")
    if not needle:
        return False
    pattern = rf"(?<!\w){re.escape(needle)}(?!\w)"
    return re.search(pattern, _norm(passage)) is not None


def _hits(quote: str, sentence: str) -> bool:
    """True when the quote's words mostly come from `sentence` (edit location).

    This is separate from `_in_passage`. A quote of fewer than MIN_QUOTE_WORDS
    words locates an edit only when it holds a fact token.
    """
    q, s = _words(quote), _words(sentence)
    if not q or len(q & s) / len(q) < QUOTE_OVERLAP:
        return False
    return len(q) >= MIN_QUOTE_WORDS or any(facts.extract(quote).values())


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
    """Score a review by quotation: the quote, the location and a non-denying problem.

    A finding counts when its quote is in the passage, it locates the planted or
    lost change, and its `problem` does not deny the change.
    """
    reported = [r for r in _parse_findings(answer) if _asserts_change(r["problem"])]
    if task.get("variant") == "authored":
        lost = task["lost"]
        passage = task["passage"]

        def names(fact: str, r: Mapping[str, str]) -> bool:
            return _in_passage(r["quote"], passage) and _mentions(
                fact, r["quote"] + " " + r["problem"]
            )

        found = [f for f in lost if any(names(f, r) for r in reported)]
        edits_total, edits_found = len(lost), len(found)
        alarms = len(reported) - sum(
            1 for r in reported if any(names(f, r) for f in lost)
        )
    else:
        edits = task["edits"]
        passage = task["passage"]

        def locates(r: Mapping[str, str], e: Mapping[str, str]) -> bool:
            return _in_passage(r["quote"], passage) and _hits(r["quote"], e["sentence"])

        matched = [any(locates(r, e) for r in reported) for e in edits]
        edits_total, edits_found = len(edits), sum(matched)
        alarms = len(reported) - sum(
            1 for r in reported if any(locates(r, e) for e in edits)
        )
    recall = edits_found / edits_total if edits_total else None
    return {
        "recall": recall,
        "found": edits_found,
        "planted": edits_total,
        "false_alarms": alarms,
        "metric": REVIEW_METRIC,
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
        "passage": author_task["source"] + "\n" + output,
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
    only_model: str | None = None,
) -> dict[str, Any]:
    """Count the calls a run makes, so the payer can approve them first.

    With `only_model`, count just the calls that model adds to an earlier run:
    its own answers, its reviews of the other models' rewrites and the other
    models' reviews of its rewrites. The other models' rewrites come from the
    earlier run's journal (`execute(candidates=...)`), so they cost no calls.
    """
    if repeats < MIN_REPEATS:
        raise ModelEvalError(f"repeats must be at least {MIN_REPEATS}")
    if only_model is not None and only_model not in {m["name"] for m in models}:
        raise ModelEvalError(f"--only-model {only_model!r} is not in the models file")
    authors = [t for t in tasks if t["role"] == "author"]
    planted = [t for t in tasks if t["role"] == "reviewer"]
    chars = sum(len(t["prompt"]) for t in authors)
    planted_chars = sum(len(t["prompt"]) for t in planted)
    per_model = {}
    for m in models:
        if only_model is not None and m["name"] != only_model:
            continue
        per_model[m["name"]] = {
            "author_calls": len(authors) * repeats,
            "planted_review_calls": len(planted) * repeats,
            "input_tokens_estimate": (chars + planted_chars) * repeats // 4,
        }
    # Each rewrite answer is also reviewed once per other model.
    rewrites = [t for t in authors if t["type"] == "rewrite"]
    others = max(len(models) - 1, 0)
    if only_model is None:
        cross = len(rewrites) * repeats * len(models) * others
    else:
        # Reviews with the model as author, plus reviews with it as reviewer.
        cross = 2 * len(rewrites) * repeats * others
    calls = (
        sum(v["author_calls"] + v["planted_review_calls"] for v in per_model.values())
        + cross
    )
    return {
        "tasks": len(tasks),
        "models": len(models),
        "repeats": repeats,
        "only_model": only_model,
        "per_model": per_model,
        "authored_review_calls": cross,
        "total_calls": calls,
    }


class BudgetExceeded(ModelEvalError):
    """The run would exceed its invocation ceiling. Nothing more is spawned.

    When it stops a call between retries, `attempts` and `usage` carry what the
    earlier attempts of that call already spent.
    """

    attempts: int = 0
    usage: dict[str, int] | None = None


class CallFailed(ModelEvalError):
    """A model command failed on every attempt."""

    def __init__(
        self, message: str, attempts: int, usage: dict[str, int] | None = None
    ) -> None:
        super().__init__(message)
        self.attempts = attempts
        self.usage = usage


class Cancelled(BaseException):
    """The run was interrupted. Like KeyboardInterrupt, nothing catches it on the way up."""


class Reply(NamedTuple):
    text: str
    attempts: int
    usage: dict[str, int] | None = None


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
        self.reserved_by_model: Counter[str | None] = Counter()
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
                if event["event"] == "reserve":
                    self.reserved_by_model[event.get("model")] += 1
        self.spent = len(reserved)
        self.unfinished_at_start = len(reserved - finished)
        self._handle = ledger.open("a", encoding="utf-8")

    def _append(self, event: dict[str, Any]) -> None:
        if self._handle is not None:
            self._handle.write(json.dumps(event, sort_keys=True) + "\n")
            self._handle.flush()
            os.fsync(self._handle.fileno())

    def reserve(self, model: str | None = None) -> int:
        """Persist one reservation, then return its number. Raises at the ceiling."""
        with self._lock:
            if self.spent >= self.ceiling:
                raise BudgetExceeded(
                    f"invocation ceiling of {self.ceiling} reached; nothing more was started"
                )
            self.spent += 1
            self.reserved_by_model[model] += 1
            self._append({"event": "reserve", "n": self.spent, "model": model})
            return self.spent

    def finish(self, number: int, status: str, model: str | None = None) -> None:
        with self._lock:
            self._append(
                {"event": "finish", "n": number, "status": status, "model": model}
            )

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

    Each line holds a job id, the model, the answer, any error, the attempts it
    used and the token `usage` (null when the model command did not report it).
    Lines from older runs lack `model` and `usage`. The last line for a job wins
    when answers are reused, but `executions` lists every line, because each one is
    spend. A job that failed is asked again on resume, and its earlier attempts
    still count against the budget.
    """

    def __init__(self, path: Path) -> None:
        self.path = path
        self._lock = threading.Lock()
        self._entries: dict[str, dict[str, Any]] = {}
        self._lines: list[dict[str, Any]] = []
        self._attempts = 0
        if path.is_file():
            for line in path.read_text("utf-8").splitlines():
                if line:
                    entry = json.loads(line)
                    self._entries[entry["job"]] = entry
                    self._lines.append(entry)
                    self._attempts += entry["attempts"]

    @property
    def attempts(self) -> int:
        return self._attempts

    def executions(self) -> list[dict[str, Any]]:
        """Every journal line as accounting, in order: superseded and failed ones too."""
        return [
            {
                "job": e["job"],
                "model": e.get("model"),
                "error": e.get("error"),
                "attempts": e["attempts"],
                "usage": e.get("usage"),
            }
            for e in self._lines
        ]

    def done(self, job: str) -> dict[str, Any] | None:
        entry = self._entries.get(job)
        return entry if entry and not entry.get("error") else None

    def has(self, job: str) -> bool:
        """Whether the job ran at all, failed or not (it was spent either way)."""
        return job in self._entries

    def record(
        self,
        job: str,
        answer: str,
        error: str | None,
        attempts: int,
        *,
        model: str | None = None,
        usage: Mapping[str, int] | None = None,
    ) -> None:
        entry = {
            "job": job,
            "model": model,
            "answer": answer,
            "error": error,
            "attempts": attempts,
            "usage": dict(usage) if usage else None,
        }
        with self._lock:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            with self.path.open("a", encoding="utf-8") as fh:
                fh.write(json.dumps(entry, sort_keys=True) + "\n")
                fh.flush()
                os.fsync(fh.fileno())
            self._entries[job] = entry
            self._lines.append(entry)


def unjournaled(
    executions: Iterable[Mapping[str, Any]], reserved: Mapping[str | None, int]
) -> list[dict[str, Any]]:
    """Ledger reservations that no journal line accounts for, as usage-less executions.

    Ctrl-C keeps the ledger's reservations but skips the journal, so the journal
    alone understates the spend. Each journal line covers `attempts` reservations
    of its model. A reservation beyond those counts as an invocation with no
    usage on record, which makes the cost report incomplete.
    """
    covered: Counter[str | None] = Counter()
    for e in executions:
        covered[e.get("model")] += e.get("attempts", 1)
    return [
        {
            "job": None,
            "model": model,
            "error": "interrupted: reserved in the ledger, never journaled",
            "attempts": 1,
            "usage": None,
        }
        for model, count in sorted(reserved.items(), key=lambda kv: str(kv[0]))
        for _ in range(max(count - covered[model], 0))
    ]


def ledger_reservations(path: Path) -> Counter[str | None]:
    """Reservations per model in a ledger (older entries name no model)."""
    reserved: Counter[str | None] = Counter()
    for line in path.read_text("utf-8").splitlines():
        if line and (event := json.loads(line))["event"] == "reserve":
            reserved[event.get("model")] += 1
    return reserved


def command_ask(
    command: str,
    attempts: int = 3,
    *,
    budget: Budget | None = None,
    timeout: int = procs.OUTER_TIMEOUT,
    live: procs.LiveGroups | None = None,
    model: str | None = None,
) -> Callable[[str], Reply]:
    """Run `command` per prompt. `model` names it in the ledger.

    Token usage comes from the command's final stderr line (`procs.USAGE_PREFIX`).
    A reply carries the usage summed over its attempts, or None when any attempt
    that ran to the end reported none.
    """
    procs.require_noreap_wait()
    argv = shlex.split(command)

    def ask(prompt: str) -> Reply:
        last = ""
        used = 0
        reports: list[dict[str, int] | None] = []
        for _ in range(attempts):
            if live and live.cancelled.is_set():
                raise Cancelled
            try:
                number = budget.reserve(model) if budget else 0
            except BudgetExceeded as exc:
                # Earlier attempts of this call already spent tokens: hand them on.
                exc.attempts, exc.usage = used, procs.sum_usage(reports)
                raise
            used += 1
            try:
                proc = procs.run_group(argv, input=prompt, timeout=timeout, live=live)
            except subprocess.TimeoutExpired:
                if budget:
                    budget.finish(number, "timeout", model)
                reports.append(None)
                last = f"model command timed out after {timeout}s"
                continue
            except BaseException:
                if budget:
                    budget.finish(number, "interrupted", model)
                raise
            if live and live.cancelled.is_set():
                if budget:
                    budget.finish(number, "interrupted", model)
                raise Cancelled
            ok = proc.returncode == 0 and bool(proc.stdout.strip())
            if budget:
                budget.finish(number, "ok" if ok else "fail", model)
            reports.append(procs.parse_usage_line(proc.stderr))
            if ok:
                return Reply(proc.stdout, used, procs.sum_usage(reports))
            detail = procs.strip_usage_lines(proc.stderr)[:300]
            last = procs.redact(f"model command exited {proc.returncode}: {detail}")
        raise CallFailed(last, used, procs.sum_usage(reports))

    return ask


def _row(
    task: Mapping[str, Any],
    model: Mapping[str, Any],
    repeat: int,
    answer: str,
    usage: Mapping[str, int] | None,
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
        "scorer_version": SCORER_VERSION,
        "output_sha256": hashlib.sha256(answer.encode()).hexdigest(),
        "usage": dict(usage) if usage else None,
    }


def _safe(
    ask: Callable[[str], Any],
    job: str,
    prompt: str,
    journal: Journal | None,
    model: str,
) -> tuple[str, str | None, dict[str, int] | None]:
    """Ask once, journal the outcome, and turn a failed call into an empty answer.

    Returns the answer, the error and the token usage (None when not reported). When
    the budget runs out between retries, the attempts already made are journaled
    first, so their spend survives the stop.
    """
    if journal and (entry := journal.done(job)):
        return entry["answer"], None, entry.get("usage")
    answer, error, attempts, usage = "", None, 1, None
    try:
        reply = ask(prompt)
        if isinstance(reply, Reply):
            answer, attempts, usage = reply.text, reply.attempts, reply.usage
        else:
            answer = reply
    except BudgetExceeded as exc:
        if journal and exc.attempts:
            journal.record(
                job,
                "",
                str(exc)[:300],
                exc.attempts,
                model=model,
                usage=exc.usage,
            )
        raise
    except CallFailed as exc:
        error, attempts, usage = str(exc)[:300], exc.attempts, exc.usage
    except ModelEvalError as exc:
        error = str(exc)[:300]
    if journal:
        journal.record(job, answer, error, attempts, model=model, usage=usage)
    return answer, error, usage


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


def review_job_id(
    review_task: Mapping[str, Any], author: str, reviewer: str, repeat: int
) -> str:
    return job_id(
        "review", reviewer, review_task["prompt"], repeat, author, review_task["id"]
    )


def same_file(a: Path, b: Path) -> bool:
    """True when both paths name one file, through links, or one resolved path."""
    try:
        return os.path.samefile(a, b)
    except OSError:
        return a.resolve() == b.resolve()


def reject_aliases(frozen: Path, **writable: Path) -> None:
    """Refuse a frozen journal that is also a file this run writes."""
    for role, path in writable.items():
        if same_file(frozen, path):
            raise ModelEvalError(
                f"--candidates-from {frozen.name} is also the {role}; "
                "the frozen journal must stay read-only"
            )


def frozen_candidates(
    tasks: Sequence[Mapping[str, Any]],
    models: Sequence[Mapping[str, Any]],
    repeats: int,
    only_model: str | None,
    candidates: Journal | None,
) -> list[tuple[Mapping[str, Any], Mapping[str, Any], int, str]]:
    """The other models' rewrites from an earlier journal, or an error before any call."""
    if only_model is None or candidates is None:
        return []
    found, missing = [], []
    for model in models:
        if model["name"] == only_model:
            continue
        for task in tasks:
            if task["role"] != "author" or task["type"] != "rewrite":
                continue
            for repeat in range(repeats):
                job = job_id("task", model["name"], task["prompt"], repeat)
                entry = candidates.done(job)
                if entry is None:
                    missing.append(f"{model['name']}/{task['id']}/{repeat}")
                else:
                    found.append((task, model, repeat, entry["answer"]))
    done = [
        job_id("task", only_model, task["prompt"], repeat)
        for task in tasks
        for repeat in range(repeats)
    ]
    for task, author, repeat, answer in found:
        review_task = authored_review_task(task, answer)
        done.append(review_job_id(review_task, author["name"], only_model, repeat))
    overlap = sum(1 for job in done if candidates.has(job))
    if overlap:
        raise ModelEvalError(
            f"{overlap} jobs of {only_model} already ran in {candidates.path}; "
            "choose a model the frozen journal has not run, or a fresh candidates journal"
        )
    if missing:
        raise ModelEvalError(
            f"{len(missing)} candidate rewrites are missing from {candidates.path} "
            f"(first: {', '.join(missing[:3])}); no call was made"
        )
    return found


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
    only_model: str | None = None,
    candidates: Journal | None = None,
) -> dict[str, Any]:
    """Run every task against every model; reviewers see other models' rewrites.

    With `only_model`, run just the jobs that model adds to an earlier run (see
    `plan`). The other models' rewrites then come from `candidates`, the earlier
    run's journal, so the reviewed pool stays the one the earlier reviewers saw.
    If a candidate is missing there, no call is made. `candidates` is only read.

    Calls run on `workers` threads. A failed call is recorded as an empty answer
    with its error, so it counts as a failure and the run goes on. A `journal`
    stores every finished call, and a `budget` stops the run before it spawns a
    process past the ceiling; `BudgetExceeded` then propagates and the journal
    keeps all finished work.
    """
    expected = plan(tasks, models, repeats, only_model)
    if (only_model is None) != (candidates is None):
        raise ModelEvalError("only_model and candidates go together")
    frozen = frozen_candidates(tasks, models, repeats, only_model, candidates)
    if candidates is not None:
        reject_aliases(
            candidates.path,
            **({"journal": journal.path} if journal else {}),
            **({"ledger": budget.ledger} if budget and budget.ledger else {}),
        )
    live = procs.LiveGroups()
    make_ask = ask_for or (
        lambda m: command_ask(m["command"], budget=budget, live=live, model=m["name"])
    )
    asks = {m["name"]: make_ask(m) for m in models}
    first = [
        (model, task, repeat)
        for model in models
        for task in tasks
        for repeat in range(repeats)
        if only_model is None or model["name"] == only_model
    ]

    def run_first(job: tuple[Any, Any, int]) -> tuple[str, str | None, Any]:
        model, task, repeat = job
        return _safe(
            asks[model["name"]],
            job_id("task", model["name"], task["prompt"], repeat),
            task["prompt"],
            journal,
            model["name"],
        )

    answers = _map_jobs(run_first, first, workers, live)
    runs: list[dict[str, Any]] = []
    rewrite_outputs: list[tuple[Mapping[str, Any], Mapping[str, Any], int, str]] = []
    for (model, task, repeat), (answer, error, usage) in zip(
        first, answers, strict=True
    ):
        row = _row(task, model, repeat, answer, usage)
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
    rewrite_outputs.extend(frozen)
    second = []
    for task, author, repeat, answer in rewrite_outputs:
        review_task = authored_review_task(task, answer)
        for reviewer in models:
            if reviewer["name"] != author["name"] and (
                only_model is None or only_model in (reviewer["name"], author["name"])
            ):
                second.append((review_task, author, reviewer, repeat))

    def run_second(job: tuple[Any, Any, Any, int]) -> tuple[str, str | None, Any]:
        review_task, author, reviewer, repeat = job
        return _safe(
            asks[reviewer["name"]],
            review_job_id(review_task, author["name"], reviewer["name"], repeat),
            review_task["prompt"],
            journal,
            reviewer["name"],
        )

    replies = _map_jobs(run_second, second, workers, live)
    for (review_task, author, reviewer, repeat), (reply, error, usage) in zip(
        second, replies, strict=True
    ):
        row = _row(review_task, reviewer, repeat, reply, usage)
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
        "scorer_version": SCORER_VERSION,
        "tasks_sha256": tasks_digest(tasks),
        "repeats": repeats,
        "only_model": only_model,
        "planned_calls": expected["total_calls"],
        "failed_calls": sum(1 for r in runs if r.get("error")),
        "unscored_runs": sum(1 for r in runs if r.get("unscored")),
        "invocations": budget.spent if budget else None,
        "executions": [
            *journal.executions(),
            *(
                unjournaled(journal.executions(), budget.reserved_by_model)
                if budget
                else []
            ),
        ]
        if journal
        else [
            {
                "job": None,
                "model": r["model"],
                "error": r.get("error"),
                "usage": r["usage"],
            }
            for r in runs
        ],
        "models": [
            {"name": m["name"], "family": m["family"], "tier": m["tier"]}
            for m in models
        ],
        "runs": runs,
    }


# --- summary -----------------------------------------------------------------------


def file_scorer_version(doc: Mapping[str, Any]) -> str | None:
    """The scorer version of a results file; None means legacy.

    A row without its own `scorer_version` takes the file header's. A file with
    no header is legacy, and a row that names a different version than its header
    (or any version under a legacy header) is an error.
    """
    header = doc.get("scorer_version")
    for run in doc["runs"]:
        if "scorer_version" in run and run["scorer_version"] != header:
            raise ModelEvalError(
                f"run {run['task']!r} has scorer version {run['scorer_version']!r}, "
                f"which disagrees with its file header ({header or 'legacy'})"
            )
    return header


def merge_results(*docs: Mapping[str, Any]) -> dict[str, Any]:
    """Join results files of one task set into one document for `summarize`.

    The files must share the schema, prompt version, task-set hash and repeats,
    and no run may appear twice. Models keep their first-seen order.
    """
    if not docs:
        raise ModelEvalError("nothing to merge")
    first = docs[0]
    for key in ("schema", "prompt_version", "tasks_sha256", "repeats"):
        for doc in docs[1:]:
            if doc.get(key) != first.get(key):
                raise ModelEvalError(
                    f"cannot merge results: {key} differs "
                    f"({first.get(key)!r} vs {doc.get(key)!r})"
                )
    versions = {file_scorer_version(doc) for doc in docs}
    if len(versions) > 1:
        shown = ", ".join(sorted(str(v or "legacy") for v in versions))
        raise ModelEvalError(f"cannot merge results: scorer versions differ ({shown})")
    roster: dict[str, dict[str, Any]] = {}
    for doc in docs:
        for m in doc["models"]:
            if roster.setdefault(m["name"], m) != m:
                raise ModelEvalError(
                    f"cannot merge results: model {m['name']!r} differs"
                )
    runs: list[dict[str, Any]] = []
    seen: set[tuple[str, str, int]] = set()
    for doc in docs:
        for run in doc["runs"]:
            key = (run["task"], run["model"], run["repeat"])
            if key in seen:
                raise ModelEvalError(f"cannot merge results: duplicate run {key}")
            seen.add(key)
            runs.append(run)
    joined = {"scorer_version": versions.pop()} if versions != {None} else {}
    return joined | {
        "schema": first["schema"],
        "date": max(doc["date"] for doc in docs),
        "prompt_version": first["prompt_version"],
        "tasks_sha256": first["tasks_sha256"],
        "repeats": first["repeats"],
        "models": list(roster.values()),
        "runs": runs,
    }


# --- summary -----------------------------------------------------------------------


def summarize(results: Mapping[str, Any], *, resamples: int = 1000) -> dict[str, Any]:
    """Success rate per review type, role and model, with a task-cluster interval.

    Repeats of one task are averaged first, so the task is the unit of resampling.
    Reviewer runs that grade a rewrite with no lost facts are excluded: there
    was nothing to find. Unscored runs (an incomplete lint) are left out too.
    A run without a `scorer_version` is legacy and keeps a legacy label. Rows from
    different scorer versions never share a ranking: that raises ModelEvalError.
    """
    groups: dict[tuple[str, str, str], dict[str, list[float]]] = defaultdict(
        lambda: defaultdict(list)
    )
    info: dict[str, dict[str, str]] = {m["name"]: m for m in results["models"]}
    versions: set[str] = set()
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
        versions.add(run.get("scorer_version") or LEGACY_SCORER)
        key = (run["type"], run["role"], run["model"])
        groups[key][run["task"]].append(1.0 if score["success"] else 0.0)
    if len(versions) > 1:
        raise ModelEvalError(
            f"mixed scorer versions {sorted(versions)}: rescore the legacy rows "
            "from the journal before summarising"
        )
    version = next(iter(versions), results.get("scorer_version") or LEGACY_SCORER)
    legacy = version == LEGACY_SCORER
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
                "metric": LEGACY_METRIC
                if legacy
                else (REVIEW_METRIC if role == "reviewer" else AUTHOR_METRIC),
                "scorer_version": version,
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
    version = file_scorer_version(results)
    carried = {"scorer_version": version} if version is not None else {}
    return carried | {
        "schema": RESULTS_SCHEMA,
        "date": results["date"],
        "prompt_version": results["prompt_version"],
        "scorer_version": version,
        "repeats": results["repeats"],
        "rows": rows,
        "ranking": ranking,
    }


RECOMMENDATIONS_SCHEMA = 2


def recommendations(
    results: Mapping[str, Any], *, publish: bool = False
) -> dict[str, Any]:
    """Reviewer rows for `crates/slopvac-agents/data/model-evals.json`.

    The agents crate ranks reviewer models per review type from this file. Each
    row is the `summarize` row for the `reviewer` role without its bootstrap
    interval, so the output is deterministic. Review types with no reviewer
    task in the results have no rows.

    Nothing is published unless the caller asks: without `publish` the file has
    status `pending-reevaluation` and no rows, so a results file whose scorer is
    under review feeds no recommendation. `publish` writes the rows with status
    `published`.
    """
    if not publish:
        return {
            "schema_version": RECOMMENDATIONS_SCHEMA,
            "status": "pending-reevaluation",
            "date": results["date"],
            "repeats": results["repeats"],
            "rows": [],
        }
    summary = summarize(results, resamples=1)
    rows = [
        {
            "review_type": r["type"],
            "family": r["family"],
            "tier": r["tier"],
            "model": r["model"],
            "success_rate": round(r["success_rate"], 4),
            "tasks": r["tasks"],
            "runs": r["runs"],
        }
        for r in summary["rows"]
        if r["role"] == "reviewer"
    ]
    return {
        "schema_version": RECOMMENDATIONS_SCHEMA,
        "status": "published",
        "date": results["date"],
        "repeats": results["repeats"],
        "rows": rows,
    }


# --- token usage and cost ----------------------------------------------------------

PRICES_PATH = lib.EVALS_DIR / "prices.json"
CACHE_KINDS = ("cache_read", "cache_write")
UNKNOWN_MODEL = "(unknown)"


def load_usage_records(path: Path) -> list[tuple[str, dict[str, int] | None]]:
    """(model, usage) per execution from a results file or a `.journal.jsonl`.

    The unit is the execution, not the score row. Every journal line is one
    execution and each is spend: a job asked again after a failure has several,
    and a job whose answer was superseded (a review of an empty candidate, then the
    author resumed) has a line but no row. One execution can also back several
    rows. So a results file carries an `executions` section, copied from the
    journal, and the cost comes from that. A results file without it (written
    before it existed) falls back to one execution per row, which can miss
    superseded work. Lines without `usage` or `model` come from older runs.
    For a `<run>.journal.jsonl` the sibling `<run>.ledger.jsonl` adds the
    reservations that never reached the journal (an interrupted call).
    """
    if path.suffix == ".jsonl":
        entries = [json.loads(x) for x in path.read_text("utf-8").splitlines() if x]
        stem = path.name.removesuffix(".journal.jsonl")
        ledger = path.with_name(stem + ".ledger.jsonl")
        if path.name.endswith(".journal.jsonl") and ledger.is_file():
            entries += unjournaled(entries, ledger_reservations(ledger))
    else:
        data = load(path)
        entries = data["executions"] if "executions" in data else data["runs"]
    return [(e.get("model") or UNKNOWN_MODEL, e.get("usage")) for e in entries]


def load_prices(path: Path) -> dict[str, Any]:
    try:
        prices = json.loads(path.read_text("utf-8"))
        table = prices["models"]
        for price in table.values():
            float(price["input"]), float(price["output"])
            for kind in CACHE_KINDS:
                if kind in price:
                    float(price[kind])
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as exc:
        raise ModelEvalError(f"{path}: not a usable price list ({exc})") from None
    return prices


def estimate_cost(
    records: Iterable[tuple[str, Mapping[str, int] | None]], prices: Mapping[str, Any]
) -> dict[str, Any]:
    """Tokens and an estimated list-price cost per model.

    Cost is input tokens at the input price plus output and reasoning tokens at
    the output price, per million, plus cache read and write tokens at their own
    prices when the model has them. A model is `complete` only when every one of
    its invocations reported usage, the price list names it, and every kind of
    cache token it used has a price; any gap makes the total incomplete too.
    """
    table = prices["models"]
    per: dict[str, dict[str, Any]] = {}
    for model, usage in records:
        row = per.setdefault(
            model, {"invocations": 0, "missing_usage": 0, "tokens": {}}
        )
        row["invocations"] += 1
        if usage is None:
            row["missing_usage"] += 1
            continue
        for key, value in usage.items():
            row["tokens"][key] = row["tokens"].get(key, 0) + value
    total_cost, complete = 0.0, True
    for model, row in sorted(per.items()):
        price = table.get(model)
        tokens = row["tokens"]
        cache_priced = True
        if price is None:
            row["cost_usd"] = None
        else:
            billed_out = tokens.get("output", 0) + tokens.get("reasoning", 0)
            cost = tokens.get("input", 0) * float(price["input"]) + billed_out * float(
                price["output"]
            )
            for kind in CACHE_KINDS:
                if not tokens.get(kind):
                    continue
                if kind in price:
                    cost += tokens[kind] * float(price[kind])
                else:
                    cache_priced = False  # cache tokens with no price to bill them at
            row["cost_usd"] = round(cost / 1_000_000, 6)
            total_cost += row["cost_usd"]
        row["complete"] = (
            price is not None and row["missing_usage"] == 0 and cache_priced
        )
        complete = complete and row["complete"]
    return {
        "source": prices.get("source"),
        "models": dict(sorted(per.items())),
        "total_cost_usd": round(total_cost, 6),
        "complete": complete,
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
