"""Statistics for the eval harness: rates, intervals and document-cluster bootstrap.

Standard library only. Every random draw comes from a seeded generator, so a
result is a pure function of its inputs.
"""

from __future__ import annotations

import math
import random
from collections.abc import Callable, Sequence
from typing import TypeVar

T = TypeVar("T")

Z95 = 1.959963984540054


def rate(hits: int, total: int) -> float | None:
    """Return hits/total, or None when nothing was measured."""
    return hits / total if total else None


def wilson(hits: int, total: int, z: float = Z95) -> tuple[float, float] | None:
    """Wilson score interval for a proportion; None when total is 0."""
    if total <= 0:
        return None
    p = hits / total
    denom = 1 + z * z / total
    centre = (p + z * z / (2 * total)) / denom
    half = z * math.sqrt(p * (1 - p) / total + z * z / (4 * total * total)) / denom
    return max(0.0, centre - half), min(1.0, centre + half)


def percentile(sorted_values: Sequence[float], q: float) -> float:
    """Linear-interpolated percentile of an ascending sequence, q in [0, 1]."""
    if not sorted_values:
        raise ValueError("percentile of an empty sequence")
    pos = q * (len(sorted_values) - 1)
    lo = math.floor(pos)
    hi = math.ceil(pos)
    frac = pos - lo
    return sorted_values[lo] * (1 - frac) + sorted_values[hi] * frac


def cluster_bootstrap(
    clusters: Sequence[T],
    statistic: Callable[[Sequence[T]], float | None],
    *,
    resamples: int = 2000,
    seed: str = "slopvac-evals-bootstrap-1",
    alpha: float = 0.05,
) -> dict[str, float | int | None]:
    """Resample whole clusters (documents) with replacement.

    Findings inside one document are correlated, so the document is the unit of
    resampling. Returns the point estimate and a percentile interval; draws where
    the statistic is undefined (None) are dropped and counted.
    """
    point = statistic(clusters)
    if not clusters:
        return {"point": point, "low": None, "high": None, "n": 0, "undefined": 0}
    rng = random.Random(seed)
    n = len(clusters)
    draws: list[float] = []
    undefined = 0
    for _ in range(resamples):
        value = statistic([clusters[rng.randrange(n)] for _ in range(n)])
        if value is None:
            undefined += 1
        else:
            draws.append(value)
    if not draws:
        return {
            "point": point,
            "low": None,
            "high": None,
            "n": n,
            "undefined": undefined,
        }
    draws.sort()
    return {
        "point": point,
        "low": percentile(draws, alpha / 2),
        "high": percentile(draws, 1 - alpha / 2),
        "n": n,
        "undefined": undefined,
    }


def paired_bootstrap(
    pairs: Sequence[T],
    statistic_a: Callable[[Sequence[T]], float | None],
    statistic_b: Callable[[Sequence[T]], float | None],
    *,
    resamples: int = 2000,
    seed: str = "slopvac-evals-ab-1",
    alpha: float = 0.05,
) -> dict[str, float | int | None]:
    """Interval for statistic_b - statistic_a over the same resampled documents.

    Each element of `pairs` is one document with both arms attached, so both arms
    see the same draw. That is what makes the comparison paired.
    """

    def delta(sample: Sequence[T]) -> float | None:
        a = statistic_a(sample)
        b = statistic_b(sample)
        return None if a is None or b is None else b - a

    return cluster_bootstrap(pairs, delta, resamples=resamples, seed=seed, alpha=alpha)
