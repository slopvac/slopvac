#!/usr/bin/env python3
"""Report in-progress beads whose work branch has sat idle past a threshold.

Runs ``bd --readonly list --status in_progress --has-metadata-key branch --json``,
keeps beads that carry a non-empty ``metadata.branch`` and a ``started_at``
older than ``TRUNK_DEV_STALE_HOURS`` (default 48), and prints a Markdown list.
Exit codes: 0 none stale, 1 at least one stale, 2 the ``bd`` query failed or
returned something that is not a JSON list.

A bead is stale only when its age is strictly greater than the threshold.
A missing ``started_at`` is skipped silently; an unparseable one is skipped
with a warning on stderr.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from collections.abc import Mapping, Sequence
from datetime import UTC, datetime
from typing import Any

DEFAULT_HOURS = 48.0
BD_ARGS = (
    "--readonly",
    "list",
    "--status",
    "in_progress",
    "--has-metadata-key",
    "branch",
    "--json",
    "--limit",
    "0",
)


def _parse_time(value: str) -> datetime:
    parsed = datetime.fromisoformat(value)
    return parsed if parsed.tzinfo else parsed.replace(tzinfo=UTC)


def find_stale(
    beads: Sequence[Mapping[str, Any]], hours: float, now: datetime
) -> list[tuple[Mapping[str, Any], float]]:
    """Return ``(bead, age_in_hours)`` for each stale in-progress bead with a branch."""
    stale = []
    for bead in beads:
        if bead.get("status") != "in_progress":
            continue
        metadata = bead.get("metadata") or {}
        if not metadata.get("branch"):
            continue
        started = bead.get("started_at")
        if not started:
            continue
        try:
            age = (now - _parse_time(started)).total_seconds() / 3600
        except (ValueError, TypeError, AttributeError):
            print(
                f"warning: {bead.get('id')}: unparseable started_at {started!r}",
                file=sys.stderr,
            )
            continue
        if age > hours:
            stale.append((bead, age))
    return stale


def render(stale: Sequence[tuple[Mapping[str, Any], float]], hours: float) -> str:
    lines = [
        f"## Stale in-progress branches older than {hours:g} hours ({len(stale)})",
        "",
    ]
    for bead, age in stale:
        assignee = bead.get("assignee") or "unassigned"
        lines.append(
            f"- {bead['id']}: `{bead['metadata']['branch']}` ({age / 24:.1f} days; {assignee})"
        )
    return "\n".join(lines) + "\n"


def main(
    env: Mapping[str, str] | None = None,
    now: datetime | None = None,
) -> int:
    env = os.environ if env is None else env
    now = now or datetime.now(UTC)
    hours = float(env.get("TRUNK_DEV_STALE_HOURS") or DEFAULT_HOURS)
    bd = env.get("BD_BIN") or "bd"
    try:
        result = subprocess.run(
            [bd, *BD_ARGS], capture_output=True, text=True, check=False, env=dict(env)
        )
    except OSError as error:
        print(f"error: cannot run {bd}: {error}", file=sys.stderr)
        return 2
    if result.returncode != 0:
        print(
            f"error: bd exited {result.returncode}: {result.stderr.strip()}",
            file=sys.stderr,
        )
        return 2
    try:
        beads = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        print(f"error: bd printed invalid JSON: {error}", file=sys.stderr)
        return 2
    if not isinstance(beads, list):
        print("error: bd printed JSON that is not a list", file=sys.stderr)
        return 2
    stale = find_stale(beads, hours, now)
    if stale:
        print(render(stale, hours), end="")
        return 1
    print(f"No in-progress bead branches older than {hours:g} hours.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
