"""Labelling: two-family judges, order-swapped prompts, human adjudication.

The workflow turns engine findings into labels that the precision gate can trust.

    sample    pick findings to label, stratified by rule     -> labels/items.jsonl
    judge     ask one model family about every item, twice   -> labels/judgments/<family>.jsonl
    merge     combine families into labels and a dispute queue
    resolve   record a human verdict for one item

No step calls a model by itself. `judge` runs the command you give it, which
reads a prompt on stdin and writes the answer on stdout, the same contract as
`evals.twins`. Calling a paid provider needs approval from the lead first.

Rules of the procedure
1. Two judges from different model families label each item. Each judge sees the
   item twice with the two options in swapped positions. A judge whose answer
   follows the position instead of the content is `unstable`.
2. An item is labelled automatically only when both families are stable and
   agree. Every other item goes to the dispute queue for a human.
3. A human also checks a seeded sample of the automatic labels. A human verdict
   always wins over judges.
4. Every change of a label appends a revision. Nothing is overwritten.
5. A judge's family must match its model id (`model_family`), so rule 1 holds.
   The recorded model id is the identity; the `--family` flag is only a claim.
6. A checkpoint is keyed by item, order, model and prompt version. Rows without a
   model or prompt version, or with another prompt version, are unusable and
   `merge` ignores them (it does not migrate them); `judge` asks those questions
   again. When a family has rows from several models, `merge` uses only the
   model of the newest row.
   Every judgment records `model` exactly as `--model` gave it, and every
   automatic label copies those ids into `models` (family -> model id).
7. `merge` writes no labels while a spot-check item has no human verdict, and
   exits 2. Sets that hold human verdicts only have nothing to spot-check.
"""

from __future__ import annotations

import datetime as dt
import hashlib
import json
import random
import re
import shlex
import subprocess
from collections import Counter, defaultdict
from collections.abc import Callable, Iterable, Mapping, Sequence
from pathlib import Path
from typing import Any

from . import corpuslib as lib
from . import runner
from .splits import split_of

LABELS_DIR = lib.EVALS_DIR / "labels"
SAMPLE_SEED = "slopvac-evals-labels-1"
PROMPT_VERSION = "1"
SPOT_CHECK_RATE = 0.10
MIN_POSITIVES = 20  # adjudicated true positives a rule needs before it is gated
VERDICTS = ("tp", "fp", "unsure")
ORDERS = ("tp-first", "fp-first")
CONTEXT_LINES = 3

PROMPT = """\
You review one finding from a prose linter. Decide whether the flagged text really is an instance of the pattern the rule describes, in this document's own context.

Rule: {rule}
What it looks for: {about}

Document genre: {genre}
Lines {first}-{last} of the document (the flagged line is marked with >>):
{context}

Flagged text: {text!r}

Options:
{options}

Answer with the letter of one option, then a short reason on the same line.
"""

OPTION_TEXT = {
    "tp": "The flagged text is a genuine instance of the pattern; the rule is right.",
    "fp": "The flagged text is not an instance, or the pattern is acceptable here; the rule is wrong.",
    "unsure": "The text alone does not settle it.",
}


# Patterns for the vendor named inside a model id. A model id that matches no
# vendor, or more than one, has no usable identity and fails the check.
FAMILY_PATTERNS = {
    "anthropic": r"claude|anthropic",
    "openai": r"gpt|openai|(?<![a-z0-9])o[134](?![a-z0-9])",
    "google": r"gemini|gemma",
    "meta": r"llama",
    "mistral": r"mistral|mixtral|codestral",
    "amazon": r"(?<![a-z])nova(?![a-z])|titan",
    "deepseek": r"deepseek",
    "qwen": r"qwen",
}


def model_family(model: str) -> str | None:
    """The one vendor family a model id names, or None when it names none or several."""
    found = [
        fam for fam, pat in FAMILY_PATTERNS.items() if re.search(pat, model.lower())
    ]
    return found[0] if len(found) == 1 else None


def check_identity(family: str, model: str) -> None:
    """Raise unless `model` names `family`; the two-family rule depends on it."""
    actual = model_family(model)
    if actual != family.lower():
        seen = actual or "no single known family"
        raise runner.EngineError(
            f"judge family {family!r} does not match model {model!r} ({seen})"
        )


def completed(
    rows: Iterable[Mapping[str, Any]], model: str, prompt_version: str | None = None
) -> set[tuple[str, str]]:
    """(item, order) pairs already judged by this model under this prompt version."""
    version = PROMPT_VERSION if prompt_version is None else prompt_version
    return {
        (r["item"], r["order"])
        for r in rows
        if r.get("model") == model and r.get("prompt_version") == version
    }


def usable_judgments(
    rows: Sequence[Mapping[str, Any]],
) -> tuple[list[Mapping[str, Any]], int]:
    """Keep rows that merge may trust, in order, and count the rest.

    A row is usable with a model, the current prompt version and a family that
    matches the model. Per family, only the newest row's model counts.
    """
    fresh = [
        r for r in rows if r.get("model") and r.get("prompt_version") == PROMPT_VERSION
    ]
    for r in fresh:
        check_identity(r["family"], r["model"])
    newest = {r["family"]: r["model"] for r in fresh}
    usable = [r for r in fresh if r["model"] == newest[r["family"]]]
    return usable, len(rows) - len(usable)


def today() -> str:
    return dt.date.today().isoformat()


def item_id(doc: str, rule: str, line: int) -> str:
    return hashlib.sha256(f"{doc}|{rule}|{line}".encode()).hexdigest()[:12]


# --- jsonl helpers -------------------------------------------------------------


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    if not path.is_file():
        return []
    return [json.loads(line) for line in path.read_text("utf-8").splitlines() if line]


def write_jsonl(path: Path, rows: Iterable[Mapping[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    body = "".join(
        json.dumps(r, sort_keys=True, ensure_ascii=False) + "\n" for r in rows
    )
    path.write_text(body, encoding="utf-8")


def append_jsonl(path: Path, rows: Iterable[Mapping[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as fh:
        for r in rows:
            fh.write(json.dumps(r, sort_keys=True, ensure_ascii=False) + "\n")


# --- rule notes ------------------------------------------------------------------


def rule_notes(rules_dir: Path) -> dict[str, str]:
    """Map `category.rule` to a one-line description from the rule YAML."""
    import yaml

    notes: dict[str, str] = {}
    for path in sorted(rules_dir.glob("*.yml")):
        for data in yaml.safe_load_all(path.read_text("utf-8")):
            for rule in (data or {}).get("rules", []):
                about = rule.get("name") or ""
                if rule.get("message"):
                    message = re.sub(r"\s+", " ", rule["message"])
                    about = f"{about}. {message}"
                notes[f"{data['id']}.{rule['id']}"] = about.strip()
    return notes


# --- sampling --------------------------------------------------------------------


def sample_items(
    manifest: Mapping[str, Any],
    results: Mapping[str, Any],
    *,
    per_rule: int = 40,
    origins: Sequence[str] = ("human", "stated-llm"),
    seed: str = SAMPLE_SEED,
    existing: Iterable[str] = (),
) -> list[dict[str, Any]]:
    """Pick up to `per_rule` unlabelled findings per rule, balanced across documents.

    Only findings on text with no inserted defects are sampled: a seeded
    sentence is a true positive by construction and says nothing about
    precision. Items already in `existing` are kept out, so reruns extend the
    set instead of replacing it.
    """
    docs = {d["id"]: d for d in manifest["documents"]}
    have = set(existing)
    pool: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for doc_id, res in results["documents"].items():
        doc = docs[doc_id]
        if doc["origin"] not in origins:
            continue
        for f in res["findings"]:
            iid = item_id(doc_id, f["rule"], f["line"])
            if iid in have:
                continue
            pool[f["rule"]].append(
                {
                    "id": iid,
                    "doc": doc_id,
                    "genre": doc["genre"],
                    "origin": doc["origin"],
                    "rule": f["rule"],
                    "line": f["line"],
                    "text": f.get("text", ""),
                    "split": split_of(doc),
                }
            )
    picked: list[dict[str, Any]] = []
    for rule in sorted(pool):
        rng = random.Random(f"{seed}|{rule}")
        by_doc: dict[str, list[dict[str, Any]]] = defaultdict(list)
        for it in pool[rule]:
            by_doc[it["doc"]].append(it)
        queues = [by_doc[d] for d in sorted(by_doc)]
        for q in queues:
            rng.shuffle(q)
        rng.shuffle(queues)
        chosen: list[dict[str, Any]] = []
        while queues and len(chosen) < per_rule:
            for q in list(queues):
                if len(chosen) == per_rule:
                    break
                chosen.append(q.pop())
                if not q:
                    queues.remove(q)
        picked.extend(chosen)
    return picked


# --- prompts and judging -----------------------------------------------------------


def context_for(
    text: str, line: int, width: int = CONTEXT_LINES
) -> tuple[int, int, str]:
    lines = text.splitlines()
    first = max(1, line - width)
    last = min(len(lines), line + width)
    body = "\n".join(
        f"{'>>' if n == line else '  '} {n}: {lines[n - 1]}"
        for n in range(first, last + 1)
    )
    return first, last, body


def build_prompt(
    item: Mapping[str, Any], order: str, doc_text: str, notes: Mapping[str, str]
) -> tuple[str, dict[str, str]]:
    """Return the prompt and the letter -> verdict map for this option order."""
    if order not in ORDERS:
        raise ValueError(f"unknown order {order!r}")
    verdicts = ["tp", "fp"] if order == "tp-first" else ["fp", "tp"]
    verdicts.append("unsure")
    letters = {chr(ord("A") + i): v for i, v in enumerate(verdicts)}
    options = "\n".join(f"{k}) {OPTION_TEXT[v]}" for k, v in letters.items())
    first, last, context = context_for(doc_text, item["line"])
    prompt = PROMPT.format(
        rule=item["rule"],
        about=notes.get(item["rule"], "(no description)"),
        genre=item["genre"],
        first=first,
        last=last,
        context=context,
        text=item["text"],
        options=options,
    )
    return prompt, letters


def parse_answer(answer: str, letters: Mapping[str, str]) -> str:
    """Read the first option letter; anything unreadable counts as unsure."""
    m = re.match(r"\s*\(?([A-C])\b", answer)
    return letters.get(m.group(1), "unsure") if m else "unsure"


def judge_items(
    items: Sequence[Mapping[str, Any]],
    ask: Callable[[str], str],
    *,
    family: str,
    model: str,
    manifest: Mapping[str, Any],
    cache: Path,
    notes: Mapping[str, str],
    done: Iterable[tuple[str, str]] = (),
    sink: Callable[[dict[str, Any]], None] | None = None,
) -> list[dict[str, Any]]:
    """Judge every pending (item, order) pair. `ask` maps a prompt to an answer.

    `sink` receives each judgment as soon as it exists, so a caller can persist it
    before the next paid call. A run that stops half way then resumes without
    asking the same question twice.
    """
    check_identity(family, model)
    docs = {d["id"]: d for d in manifest["documents"]}
    skip = set(done)
    texts: dict[str, str] = {}
    rows: list[dict[str, Any]] = []
    for item in items:
        for order in ORDERS:
            if (item["id"], order) in skip:
                continue
            if item["doc"] not in texts:
                texts[item["doc"]] = runner.text_path(
                    docs[item["doc"]], cache
                ).read_text("utf-8")
            prompt, letters = build_prompt(item, order, texts[item["doc"]], notes)
            row = {
                "item": item["id"],
                "family": family,
                "model": model,
                "order": order,
                "prompt_version": PROMPT_VERSION,
                "verdict": parse_answer(ask(prompt), letters),
            }
            if sink:
                sink(row)
            rows.append(row)
    return rows


def command_asker(command: str) -> Callable[[str], str]:
    argv = shlex.split(command)

    def ask(prompt: str) -> str:
        proc = subprocess.run(
            argv, input=prompt, capture_output=True, text=True, check=False
        )
        if proc.returncode != 0:
            raise runner.EngineError(
                f"provider command exited {proc.returncode}: {proc.stderr[:300]}"
            )
        return proc.stdout

    return ask


# --- merging ----------------------------------------------------------------------


def family_verdict(rows: Sequence[Mapping[str, Any]]) -> str:
    """One family's verdict: stable only if both option orders give the same answer."""
    by_order = {r["order"]: r["verdict"] for r in rows}
    if set(by_order) != set(ORDERS):
        return "missing"
    first, second = by_order[ORDERS[0]], by_order[ORDERS[1]]
    return first if first == second and first != "unsure" else "unstable"


def cohen_kappa(pairs: Sequence[tuple[str, str]]) -> float | None:
    """Chance-corrected agreement between two raters; None when undefined."""
    n = len(pairs)
    if n == 0:
        return None
    observed = sum(a == b for a, b in pairs) / n
    left, right = Counter(a for a, _ in pairs), Counter(b for _, b in pairs)
    expected = sum(left[k] * right[k] for k in left) / (n * n)
    if expected == 1:
        return None
    return (observed - expected) / (1 - expected)


def merge(
    items: Sequence[Mapping[str, Any]],
    judgments: Sequence[Mapping[str, Any]],
    human: Sequence[Mapping[str, Any]],
    *,
    spot_rate: float = SPOT_CHECK_RATE,
    seed: str = SAMPLE_SEED,
) -> dict[str, Any]:
    """Combine judge and human verdicts into labels, disputes and a spot-check queue."""
    by_item: dict[str, dict[str, list[Mapping[str, Any]]]] = defaultdict(
        lambda: defaultdict(list)
    )
    for j in judgments:
        by_item[j["item"]][j["family"]].append(j)
    people = {h["item"]: h for h in human}
    labels: list[dict[str, Any]] = []
    disputes: list[dict[str, Any]] = []
    auto: list[dict[str, Any]] = []
    pairs: list[tuple[str, str]] = []
    families = sorted({j["family"] for j in judgments})
    # Exact model ids as recorded in the judgments, copied to automatic labels.
    models = {j["family"]: j.get("model") for j in judgments}
    for item in items:
        verdicts = {
            fam: family_verdict(by_item[item["id"]].get(fam, [])) for fam in families
        }
        if len(families) >= 2:
            a, b = verdicts[families[0]], verdicts[families[1]]
            if a in ("tp", "fp") and b in ("tp", "fp"):
                pairs.append((a, b))
        base = {k: item[k] for k in ("doc", "rule", "line", "split")}
        base["item"] = item["id"]
        agree = (
            len(families) >= 2
            and len(set(verdicts.values())) == 1
            and next(iter(verdicts.values())) in ("tp", "fp")
        )
        if agree:
            # The spot-check sample is drawn from every judged agreement, also
            # after a human re-checked it, so the sample stays the same.
            auto.append({**base, "label": next(iter(verdicts.values()))})
        if item["id"] in people:
            h = people[item["id"]]
            labels.append({**base, "label": h["label"], "source": "human"})
            continue
        if agree:
            labels.append({**auto[-1], "source": "judges", "models": dict(models)})
        else:
            disputes.append({**base, "verdicts": verdicts})
    rng = random.Random(f"{seed}|spot")
    spot = sorted((a["item"] for a in auto), key=lambda i: (rng.random(), i))[
        : round(len(auto) * spot_rate)
    ]
    pending = [i for i in spot if i not in people]
    stats = {
        "items": len(items),
        "labelled": len(labels),
        "by_source": dict(Counter(label["source"] for label in labels)),
        "disputes": len(disputes),
        "kappa": cohen_kappa(pairs),
        "kappa_n": len(pairs),
        "unstable": sum(
            v == "unstable"
            for it in items
            for v in (family_verdict(by_item[it["id"]].get(f, [])) for f in families)
        ),
        "positives_by_rule": dict(
            sorted(
                Counter(lbl["rule"] for lbl in labels if lbl["label"] == "tp").items()
            )
        ),
    }
    return {
        "labels": labels,
        "disputes": disputes,
        "spot_check": spot,
        "spot_pending": pending,
        "stats": stats,
    }


def underlabelled(
    labels: Sequence[Mapping[str, Any]],
    rules: Iterable[str],
    minimum: int = MIN_POSITIVES,
) -> dict[str, int]:
    """Rules with fewer than `minimum` adjudicated true positives, with their count."""
    have = Counter(lbl["rule"] for lbl in labels if lbl["label"] == "tp")
    return {r: have[r] for r in sorted(set(rules)) if have[r] < minimum}


def record_human(
    *, item: str, label: str, reviewer: str, reason: str, date: str
) -> dict[str, Any]:
    """Build one human verdict row. The file is append-only; the last row wins."""
    if label not in ("tp", "fp"):
        raise ValueError("a human verdict is tp or fp")
    if not reason.strip():
        raise ValueError("a human verdict needs a reason")
    return {
        "item": item,
        "label": label,
        "reviewer": reviewer,
        "reason": reason,
        "date": date,
    }


def revisions_for_merge(
    previous: Mapping[str, str],
    labels: Sequence[Mapping[str, Any]],
    date: str,
    human: Sequence[Mapping[str, Any]] = (),
) -> list[dict[str, Any]]:
    """One revision for every label that is new or differs from `previous`."""
    latest = {h["item"]: h for h in human}
    out: list[dict[str, Any]] = []
    for lbl in labels:
        if previous.get(lbl["item"]) == lbl["label"]:
            continue
        rev = {
            "item": lbl["item"],
            "from": previous.get(lbl["item"]),
            "to": lbl["label"],
            "source": lbl["source"],
            "date": date,
        }
        if lbl["source"] == "human":
            h = latest[lbl["item"]]
            rev.update(reviewer=h["reviewer"], reason=h["reason"])
        out.append(rev)
    return out
