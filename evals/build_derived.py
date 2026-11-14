"""Generate the derived documents: seeded defects, clean controls, rewrite pairs.

    python -m evals.build_derived                 # update manifest records
    python -m evals.build_derived --materialise   # also write texts to the cache

Every output is a pure function of the committed human documents, the defect
bank and `SEED`, so a fresh checkout reproduces the manifest hashes exactly.
Parents are always committed documents, which keeps the offline test able to
regenerate everything.

- seeded-defect: a human document with sentences inserted that the rules flag.
- clean-control: the same document with the rule's matching clean sentence at
  each insertion point, so each control is a minimal pair of its defect twin.
- rewrite-pair: a human document with planted fact edits (dropped sentence,
  changed number, flipped modal, swapped identifier). The gold edits are in
  the manifest. This is the zero-paraphrase tier; a model paraphrase is applied
  on top by a later run.
- model-twin: pending slots only; see `evals.twins`.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import random
import re
from collections.abc import Callable
from pathlib import Path
from typing import Any

from . import corpuslib as lib
from .textops import (
    CODE_SPAN_RE,
    NUMBER_RE,
    outline_for,
    prose_paragraphs,
    split_sentences,
)

GENERATOR_VERSION = "1"
SEED = "slopvac-evals-derived-1"
BANK_PATH = lib.EVALS_DIR / "defect-bank.json"
PROSE_GENRES = ("product", "reference", "tutorials", "readmes", "adrs")
SEEDED_PARENTS = 70
REWRITE_PARENTS = 50
TWIN_PARENTS = 24
TWIN_FAMILIES = ("anthropic", "openai", "google")
DEFECTS_PER_DOC = 3
EDITS_PER_DOC = 4
DERIVED_ORIGINS = ("seeded-defect", "clean-control", "rewrite-pair", "model-twin")
MODALS = (
    (re.compile(r"\bcan\b"), "cannot"),
    (re.compile(r"\bmust\b"), "must not"),
    (re.compile(r"\bshould\b"), "should not"),
    (re.compile(r"\bis\b"), "is not"),
    (re.compile(r"\bare\b"), "are not"),
    (re.compile(r"\balways\b"), "never"),
)


def _rng(*parts: str) -> random.Random:
    return random.Random("|".join((SEED, GENERATOR_VERSION, *parts)))


def bank_digest(bank: dict[str, Any]) -> str:
    return hashlib.sha256(
        json.dumps(bank["entries"], sort_keys=True).encode()
    ).hexdigest()


def eligible_parents(
    manifest: dict[str, Any], read: Callable[[dict[str, Any]], str]
) -> list[dict[str, Any]]:
    out = []
    for doc in sorted(manifest["documents"], key=lambda d: d["id"]):
        if (
            doc["origin"] == "human"
            and doc["committed"]
            and doc["genre"] in PROSE_GENRES
            and doc["format"] in ("markdown", "rst")
            and 250 <= doc["words"] <= 4000
            and len(prose_paragraphs(read(doc))) >= 6
        ):
            out.append(doc)
    return out


def _derived_doc(
    parent: dict[str, Any], origin: str, text: str, source: dict[str, Any]
) -> dict[str, Any]:
    prefix = {
        "seeded-defect": "seeded",
        "clean-control": "control",
        "rewrite-pair": "rewrite",
    }[origin]
    return {
        "id": f"{prefix}.{parent['id']}",
        "genre": parent["genre"],
        "origin": origin,
        "format": parent["format"],
        "license": parent["license"],
        "date": parent["date"],
        "source": source,
        "sha256": lib.sha256_text(text),
        "bytes": len(lib.normalise(text).encode()),
        "words": len(text.split()),
        "committed": False,
    }


def seed_defects(
    parent: dict[str, Any], text: str, bank: dict[str, Any]
) -> tuple[tuple[dict, str], tuple[dict, str]] | None:
    """Return the (defect, control) documents for one parent, or None if it cannot host any."""
    rng = _rng("seeded", parent["id"])
    lines = text.split("\n")
    paragraphs = prose_paragraphs(text)
    count = min(DEFECTS_PER_DOC, len(paragraphs) // 2)
    if count < 1:
        return None
    # Spread insertion points: one per equal slice of the document.
    step = len(paragraphs) / count
    points = sorted(
        {
            paragraphs[min(len(paragraphs) - 1, int(i * step + rng.random() * step))][1]
            for i in range(count)
        }
    )
    entries = list(bank["entries"])
    rng.shuffle(entries)
    chosen: list[dict[str, Any]] = []
    for entry in entries:
        if all(entry["rule"] != c["rule"] for c in chosen):
            chosen.append(entry)
        if len(chosen) == len(points):
            break

    def build(which: str) -> tuple[str, list[dict[str, Any]]]:
        out = list(lines)
        pairs = list(zip(points, chosen))
        for end, entry in sorted(pairs, reverse=True):
            out[end + 1 : end + 1] = ["", entry[which]]
        placed = []
        for rank, (end, entry) in enumerate(sorted(pairs), 1):
            placed.append(
                {
                    "rule": entry["rule"],
                    "line": end + 2 * rank + 1,
                    "text": entry[which],
                }
            )
        return "\n".join(out), placed

    defect_text, defects = build("bad")
    control_text, controls = build("good")
    common = {
        "type": "derived",
        "generator": f"seed-defects/v{GENERATOR_VERSION}",
        "seed": f"{SEED}|seeded|{parent['id']}",
        "parents": [parent["id"]],
    }
    defect = _derived_doc(
        parent, "seeded-defect", defect_text, {**common, "defects": defects}
    )
    control = _derived_doc(
        parent,
        "clean-control",
        control_text,
        {**common, "pair": defect["id"], "controls": controls},
    )
    return (defect, defect_text), (control, control_text)


def _replace_number(sentence: str, rng: random.Random) -> tuple[str, str] | None:
    matches = list(NUMBER_RE.finditer(sentence))
    if not matches:
        return None
    m = rng.choice(matches)
    value = int(m.group(1))
    new = value + rng.choice((1, 2, 5, 10))
    return sentence[: m.start()] + str(new) + sentence[m.end() :], f"{value} -> {new}"


def _flip_modal(sentence: str, rng: random.Random) -> tuple[str, str] | None:
    if re.search(r"\b(not|no|never|cannot)\b", sentence):
        return None
    options = [(rx, rep) for rx, rep in MODALS if rx.search(sentence)]
    if not options:
        return None
    rx, rep = rng.choice(options)
    return rx.sub(rep, sentence, count=1), f"modal -> {rep}"


def _swap_identifier(
    sentence: str, pool: list[str], rng: random.Random
) -> tuple[str, str] | None:
    spans = CODE_SPAN_RE.findall(sentence)
    others = [p for p in pool if p not in spans]
    if not spans or not others:
        return None
    old = rng.choice(spans)
    new = rng.choice(others)
    return sentence.replace(f"`{old}`", f"`{new}`", 1), f"`{old}` -> `{new}`"


def plant_edits(parent: dict[str, Any], text: str) -> tuple[dict[str, Any], str] | None:
    rng = _rng("rewrite", parent["id"])
    lines = text.split("\n")
    pool = sorted(set(CODE_SPAN_RE.findall(text)))
    candidates = []
    for first, last in prose_paragraphs(text):
        sentences = split_sentences("\n".join(lines[first : last + 1]))
        if len(sentences) >= 2:
            candidates.append((first, last, sentences))
    rng.shuffle(candidates)
    kinds = ["drop", "number", "negation", "entity"]
    edits: list[dict[str, Any]] = []
    replacements: dict[int, tuple[int, str]] = {}
    for first, last, sentences in candidates:
        if len(edits) >= EDITS_PER_DOC:
            break
        kind = kinds[len(edits) % len(kinds)]
        order = list(range(len(sentences)))
        rng.shuffle(order)
        for i in order:
            sentence = sentences[i]
            if kind == "drop":
                # Only drop sentences that carry a checkable fact.
                if not (NUMBER_RE.search(sentence) or CODE_SPAN_RE.search(sentence)):
                    continue
                rebuilt = " ".join(sentences[:i] + sentences[i + 1 :])
                detail = "dropped"
            else:
                if kind == "number":
                    changed = _replace_number(sentence, rng)
                elif kind == "negation":
                    changed = _flip_modal(sentence, rng)
                else:
                    changed = _swap_identifier(sentence, pool, rng)
                if changed is None:
                    continue
                rebuilt = " ".join(sentences[:i] + [changed[0]] + sentences[i + 1 :])
                detail = changed[1]
            replacements[first] = (last, rebuilt)
            edits.append(
                {
                    "kind": kind,
                    "line": first + 1,
                    "detail": detail,
                    "original_sentence": sentence,
                }
            )
            break
    if len(edits) < 2:
        return None
    out: list[str] = []
    skip_until = -1
    for index, line in enumerate(lines):
        if index <= skip_until:
            continue
        if index in replacements:
            skip_until, rebuilt = replacements[index]
            out.append(rebuilt)
        else:
            out.append(line)
    edits.sort(key=lambda e: e["line"])
    new_text = "\n".join(out)
    doc = _derived_doc(
        parent,
        "rewrite-pair",
        new_text,
        {
            "type": "derived",
            "generator": f"plant-edits/v{GENERATOR_VERSION}",
            "seed": f"{SEED}|rewrite|{parent['id']}",
            "parents": [parent["id"]],
            "original_sha256": parent["sha256"],
            "edits": edits,
        },
    )
    return doc, new_text


def twin_slots(
    parents: list[dict[str, Any]], read: Callable[[dict[str, Any]], str]
) -> list[dict[str, Any]]:
    out = []
    for parent in parents:
        outline_hash = lib.sha256_text(
            outline_for(read(parent), parent["genre"], parent["format"])
        )
        for family in TWIN_FAMILIES:
            out.append(
                {
                    "id": f"twin.{family}.{parent['id']}",
                    "genre": parent["genre"],
                    "origin": "model-twin",
                    "format": parent["format"],
                    "license": "LicenseRef-model-output",
                    "date": None,
                    "status": "pending",
                    "source": {
                        "type": "model-output",
                        "family": family,
                        "outline_of": parent["id"],
                        "outline_sha256": outline_hash,
                    },
                    "sha256": None,
                    "committed": False,
                }
            )
    return out


def generate_all(
    manifest: dict[str, Any],
    bank: dict[str, Any],
    read: Callable[[dict[str, Any]], str],
) -> tuple[list[tuple[dict, str]], list[dict[str, Any]]]:
    """Return (derived documents with text, twin slots)."""
    parents = eligible_parents(manifest, read)
    pick = _rng("pick").sample(
        parents, min(len(parents), SEEDED_PARENTS + REWRITE_PARENTS)
    )
    seeded_parents, rewrite_parents = pick[:SEEDED_PARENTS], pick[SEEDED_PARENTS:]
    outputs: list[tuple[dict, str]] = []
    for parent in sorted(seeded_parents, key=lambda d: d["id"]):
        pair = seed_defects(parent, read(parent), bank)
        if pair:
            outputs.extend(pair)
    for parent in sorted(rewrite_parents, key=lambda d: d["id"]):
        rewrite = plant_edits(parent, read(parent))
        if rewrite:
            outputs.append(rewrite)
    twin_parents = [p for p in parents if p["format"] == "markdown"]
    twin_pick = _rng("twins").sample(twin_parents, min(len(twin_parents), TWIN_PARENTS))
    return outputs, twin_slots(sorted(twin_pick, key=lambda d: d["id"]), read)


def committed_reader(doc: dict[str, Any]) -> str:
    return lib.committed_file(doc).read_text(encoding="utf-8")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument(
        "--materialise", action="store_true", help="write texts to the cache"
    )
    parser.add_argument(
        "--cache",
        type=Path,
        default=Path(os.environ.get("SLOPVAC_EVALS_CACHE", lib.DEFAULT_CACHE)),
    )
    args = parser.parse_args(argv)
    manifest = lib.load_manifest()
    bank = json.loads(BANK_PATH.read_text(encoding="utf-8"))
    outputs, slots = generate_all(manifest, bank, committed_reader)

    existing = {
        d["id"]: d for d in manifest["documents"] if d["origin"] == "model-twin"
    }
    slot_records = []
    for slot in slots:
        old = existing.get(slot["id"])
        same = (
            old and old["source"]["outline_sha256"] == slot["source"]["outline_sha256"]
        )
        slot_records.append(old if same else slot)
    manifest["documents"] = (
        [d for d in manifest["documents"] if d["origin"] not in DERIVED_ORIGINS]
        + [doc for doc, _ in outputs]
        + slot_records
    )
    manifest["derived"] = {
        "seed": SEED,
        "generator_version": GENERATOR_VERSION,
        "bank_sha256": bank_digest(bank),
        "twin_families": list(TWIN_FAMILIES),
    }
    lib.dump_manifest(manifest)
    errors = lib.validate_manifest(manifest)
    for error in errors[:20]:
        print(error)
    if args.materialise:
        for doc, text in outputs:
            target = lib.cache_file(args.cache, doc["id"])
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(lib.normalise(text), encoding="utf-8")
    print(f"{len(outputs)} derived documents, {len(slot_records)} twin slots")
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
