"""Tuning/holdout split and corpus slices.

A document's split follows its root parent, so a seeded document, its clean
control, a rewrite pair and every model twin land on the same side as the
human text they came from. That keeps held-out text out of rule tuning.
"""

from __future__ import annotations

import hashlib
from collections.abc import Mapping
from typing import Any

SPLIT_SEED = "slopvac-evals-split-1"
HOLDOUT_PERCENT = 20


def root_id(doc: Mapping[str, Any]) -> str:
    source = doc.get("source", {})
    parents = source.get("parents")
    if parents:
        return parents[0]
    if source.get("outline_of"):
        return source["outline_of"]
    return doc["id"]


def split_of(doc: Mapping[str, Any], holdout_percent: int = HOLDOUT_PERCENT) -> str:
    digest = hashlib.sha256(f"{SPLIT_SEED}|{root_id(doc)}".encode()).digest()
    bucket = int.from_bytes(digest[:4], "big") % 100
    return "holdout" if bucket < holdout_percent else "tuning"
