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
import hashlib
import json
import random
import re
import shlex
import subprocess
import tempfile
from collections import defaultdict
from collections.abc import Callable, Iterable, Mapping, Sequence
from pathlib import Path
from typing import Any

from . import corpuslib as lib
from . import facts, metrics, runner
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
    rng.shuffle(updates)
    for doc, edit, old, new in updates[:per_type]:
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


def tasks_digest(tasks: Sequence[Mapping[str, Any]]) -> str:
    return lib.sha256_text(json.dumps([t["id"] for t in tasks]))


# --- scoring ---------------------------------------------------------------------

Linter = Callable[[str], list[dict[str, Any]]]


def make_linter(engine: runner.Engine, suffix: str = ".md") -> Linter:
    """Lint a text with the engine and return slim findings."""

    def lint(text: str) -> list[dict[str, Any]]:
        with tempfile.TemporaryDirectory(prefix="slopvac-evals-") as tmp:
            target = Path(tmp) / f"candidate{suffix}"
            target.write_text(text, encoding="utf-8")
            staged = runner.Engine(
                engine.name, engine.command, Path(tmp), engine.unsupported
            )
            report = staged.lint([target])
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


def score_review(task: Mapping[str, Any], answer: str) -> dict[str, Any]:
    reported = _parse_findings(answer)
    if task.get("variant") == "authored":
        lost = task["lost"]
        found = [
            f
            for f in lost
            if any(
                f.lower() in (r["quote"] + " " + r["problem"]).lower() for r in reported
            )
        ]
        edits_total, edits_found = len(lost), len(found)
        alarms = sum(
            1
            for r in reported
            if not any(
                f.lower() in (r["quote"] + " " + r["problem"]).lower() for f in lost
            )
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
) -> dict[str, Any] | None:
    """Turn an author rewrite into a review task. None when nothing was lost."""
    output = _clean(author_output)
    lost = [
        item["fact"]
        for item in facts.preservation(author_task["source"], output)["lost"]
    ]
    return {
        "id": task_id(
            "authored", author_task["id"], hashlib.sha256(output.encode()).hexdigest()
        ),
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


def command_ask(command: str) -> Callable[[str], str]:
    argv = shlex.split(command)

    def ask(prompt: str) -> str:
        proc = subprocess.run(
            argv, input=prompt, capture_output=True, text=True, check=False
        )
        if proc.returncode != 0:
            raise ModelEvalError(
                f"model command exited {proc.returncode}: {proc.stderr[:300]}"
            )
        return proc.stdout

    return ask


def execute(
    tasks: Sequence[Mapping[str, Any]],
    models: Sequence[Mapping[str, Any]],
    repeats: int,
    lint: Linter,
    *,
    ask_for: Callable[[Mapping[str, Any]], Callable[[str], str]] = lambda m: (
        command_ask(m["command"])
    ),
    date: str | None = None,
) -> dict[str, Any]:
    """Run every task against every model; reviewers see other models' rewrites."""
    expected = plan(tasks, models, repeats)
    asks = {m["name"]: ask_for(m) for m in models}
    runs: list[dict[str, Any]] = []
    rewrite_outputs: list[tuple[Mapping[str, Any], Mapping[str, Any], int, str]] = []
    for model in models:
        for task in tasks:
            for repeat in range(repeats):
                answer = asks[model["name"]](task["prompt"])
                row = {
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
                if task["role"] == "author":
                    row["score"] = score_author(task, answer, lint)
                    if task["type"] == "rewrite":
                        rewrite_outputs.append((task, model, repeat, answer))
                else:
                    row["score"] = score_review(task, answer)
                runs.append(row)
    for task, author, repeat, answer in rewrite_outputs:
        review_task = authored_review_task(task, answer)
        for reviewer in models:
            if reviewer["name"] == author["name"]:
                continue
            reply = asks[reviewer["name"]](review_task["prompt"])
            runs.append(
                {
                    "task": review_task["id"],
                    "type": "rewrite",
                    "role": "reviewer",
                    "variant": "authored",
                    "model": reviewer["name"],
                    "family": reviewer["family"],
                    "tier": reviewer["tier"],
                    "author_model": author["name"],
                    "author_family": author["family"],
                    "repeat": repeat,
                    "output_sha256": hashlib.sha256(reply.encode()).hexdigest(),
                    "score": score_review(review_task, reply)
                    | {"lost_facts": len(review_task["lost"])},
                }
            )
    return {
        "schema": RESULTS_SCHEMA,
        "date": date or dt.date.today().isoformat(),
        "prompt_version": PROMPT_VERSION,
        "tasks_sha256": tasks_digest(tasks),
        "repeats": repeats,
        "planned_calls": expected["total_calls"],
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
    was nothing to find.
    """
    groups: dict[tuple[str, str, str], dict[str, list[float]]] = defaultdict(
        lambda: defaultdict(list)
    )
    info: dict[str, dict[str, str]] = {m["name"]: m for m in results["models"]}
    for run in results["runs"]:
        score = run["score"]
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
