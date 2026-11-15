"""Run the cases against one plugin command and build a report."""

from __future__ import annotations

import json
import time
import tomllib
import unicodedata
from collections.abc import Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from .cases import ALL_KINDS, CASES, Env, handshake
from .schemas import Schemas
from .session import CaseFailure, CaseSkip, SuiteError

PASS, FAIL, SKIP = "pass", "fail", "skip"


# Default-ignorable code points outside the Cf category: fillers and selectors.
_IGNORABLE = (
    0x034F, 0x115F, 0x1160, 0x17B4, 0x17B5, 0x180B, 0x180C, 0x180D, 0x180F,
    0x3164, 0xFFA0,
)  # fmt: skip
_IGNORABLE_RANGES = ((0xFE00, 0xFE0F), (0xE0100, 0xE01EF))
_UNSAFE_CATEGORIES = frozenset({"Cc", "Cf", "Cs", "Co", "Zl", "Zp"})


def _unsafe(char: str) -> bool:
    code = ord(char)
    return (
        unicodedata.category(char) in _UNSAFE_CATEGORIES
        or 0xFDD0 <= code <= 0xFDEF  # noncharacters
        or code & 0xFFFE == 0xFFFE
        or code in _IGNORABLE
        or any(low <= code <= high for low, high in _IGNORABLE_RANGES)
    )


def escape_text(text: str) -> str:
    """Show control, format, bidi, zero-width and other invisible characters as escapes.

    Case details quote text the plugin chose, and a terminal acts on raw controls.
    The set of characters matches the Rust crate `slopvac-sanitize` (the host's
    sanitiser, threat model O1): Cc, Cf, Co, Zl, Zp, default-ignorable code points
    and noncharacters. Unassigned code points stay as they are, since the set
    changes with each Unicode version. The spelling of an escape is Python's.
    """
    out = []
    for char in text:
        code = ord(char)
        if not _unsafe(char):
            out.append(char)
        elif code < 0x100:
            out.append(f"\\x{code:02x}")
        elif code < 0x10000:
            out.append(f"\\u{code:04x}")
        else:
            out.append(f"\\U{code:08x}")
    return "".join(out)


@dataclass(frozen=True)
class CaseResult:
    id: str
    title: str
    status: str
    detail: str = ""


@dataclass
class Report:
    results: list[CaseResult] = field(default_factory=list)

    @property
    def failed(self) -> list[CaseResult]:
        return [r for r in self.results if r.status == FAIL]

    @property
    def exit_code(self) -> int:
        """0 when every applicable case passed, else 1."""
        return 1 if self.failed else 0

    def status(self, case_id: str) -> str:
        return next(r.status for r in self.results if r.id == case_id)

    def to_text(self) -> str:
        lines = [
            f"{r.status.upper():4} {r.id:4} {escape_text(r.title)}"
            + (f"\n       {escape_text(r.detail)}" if r.detail else "")
            for r in self.results
        ]
        counts = {
            s: sum(r.status == s for r in self.results) for s in (PASS, FAIL, SKIP)
        }
        lines.append(
            f"{counts[PASS]} passed, {counts[FAIL]} failed, {counts[SKIP]} skipped"
        )
        return "\n".join(lines)

    def to_json(self) -> str:
        return json.dumps(
            {
                "results": [
                    {
                        "id": r.id,
                        "title": r.title,
                        "status": r.status,
                        "detail": r.detail,
                    }
                    for r in self.results
                ],
                "ok": not self.failed,
            },
            indent=2,
        )


def load_declaration(path: Path) -> dict[str, Any]:
    """Read a PluginDeclaration from a pack manifest (TOML) or a JSON file."""
    text = path.read_text(encoding="utf-8")
    data = tomllib.loads(text) if path.suffix == ".toml" else json.loads(text)
    declaration = data.get("plugin", data)
    if not isinstance(declaration, dict):
        raise SuiteError(f"{path}: no plugin declaration")
    return declaration


def run_conformance(
    command: Sequence[str],
    *,
    declaration: dict[str, Any] | None = None,
    inputs: dict[str, Any] | None = None,
    timeout: float = 10.0,
    exit_timeout: float = 2.0,
    cancel_grace: float = 2.0,
    pass_env: Sequence[str] = (),
    only: Sequence[str] | None = None,
    case_budget: float = 120.0,
    suite_budget: float = 900.0,
) -> Report:
    """Run P1 to P19 against ``command``. Raises SuiteError when it cannot start.

    ``case_budget`` and ``suite_budget`` are wall-clock seconds the host spends on one
    case and on the whole run. Every wait is cut to the time left, and a case that
    runs out fails, whatever the per-reply ``timeout`` allows.
    """
    started = time.monotonic()
    suite_end = started + suite_budget
    env = Env(
        command,
        Schemas(),
        timeout=timeout,
        exit_timeout=exit_timeout,
        cancel_grace=cancel_grace,
        pass_env=pass_env,
        inputs=inputs or {},
        declaration=declaration,
    )
    env.deadline = min(started + case_budget, suite_end)
    try:
        with env.session() as probe:
            reply = handshake(probe)
            # A hook kind the host does not know is ignored (section 4.2). P19 still
            # reads the full list from its own reply.
            env.hooks = [
                (h["kind"], h["id"])
                for h in reply["capabilities"]["hooks"]
                if h["kind"] in ALL_KINDS
            ]
    except CaseFailure:
        pass  # P1 reports the cause. Hooks read before a failed close still count.
    report = Report()
    for case_id, (title, fn) in CASES.items():
        if only and case_id not in only:
            continue
        now = time.monotonic()
        if now >= suite_end:
            report.results.append(
                CaseResult(
                    case_id,
                    title,
                    FAIL,
                    f"the suite budget of {suite_budget:g} s ran out before this case",
                )
            )
            continue
        env.deadline = min(now + case_budget, suite_end)
        status, detail = PASS, ""
        try:
            fn(env)
        except CaseSkip as skip:
            status, detail = SKIP, str(skip)
        except CaseFailure as failure:
            status, detail = FAIL, str(failure)
        # Teardown waits stop at the deadline, but the kill after them still takes
        # time. A case that ends past its deadline did not finish in its budget.
        if status != FAIL and time.monotonic() > env.deadline:
            status, detail = FAIL, "the case ran past its time budget"
        report.results.append(CaseResult(case_id, title, status, detail))
    return report
