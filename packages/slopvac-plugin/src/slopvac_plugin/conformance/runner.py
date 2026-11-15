"""Run the cases against one plugin command and build a report."""

from __future__ import annotations

import bisect
import json
import time
import tomllib
from collections.abc import Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from ._unsafe_ranges import UNSAFE_RANGES
from .cases import ALL_KINDS, CASES, Env, handshake
from .schemas import Schemas
from .session import CaseFailure, CaseSkip, SuiteError

PASS, FAIL, SKIP = "pass", "fail", "skip"


_LOWS = [low for low, _ in UNSAFE_RANGES]


def _unsafe(char: str) -> bool:
    code = ord(char)
    if 0xD800 <= code <= 0xDFFF:  # a lone surrogate, which Rust text cannot hold
        return True
    i = bisect.bisect_right(_LOWS, code) - 1
    return i >= 0 and code <= UNSAFE_RANGES[i][1]


def escape_text(text: str) -> str:
    """Show control, format, bidi, zero-width and other invisible characters as escapes.

    Case details quote text the plugin chose, and a terminal acts on raw controls.

    The set of characters is the table of the Rust crate `slopvac-sanitize` (the
    host's sanitiser, threat model O1 and TM-11): `_unsafe_ranges.py` is generated
    from the same Unicode data, and a test compares the two. Unassigned code
    points other than reserved default-ignorables stay as they are. The spelling
    of an escape is Python's (`\\x1b`, `\\u202e`, `\\U000e0041`). A literal
    backslash is doubled, so the text `\\u202e` cannot pass for an escaped control.

    A GitHub Actions runner reads a line that starts with `::` (after white space)
    and a `##[` anywhere as a workflow command (`ActionCommand.cs` in
    actions/runner), so the first colon of such a `::` becomes `\\x3a` and each
    `##[` becomes `\\x23#[`.
    """
    out = []
    for char in text:
        code = ord(char)
        if char == "\\":
            out.append("\\\\")
        elif not _unsafe(char):
            out.append(char)
        elif code < 0x100:
            out.append(f"\\x{code:02x}")
        elif code < 0x10000:
            out.append(f"\\u{code:04x}")
        else:
            out.append(f"\\U{code:08x}")
    return _guard_commands("".join(out))


def _guard_commands(text: str) -> str:
    lines = []
    for line in text.splitlines(keepends=True):
        rest = line.lstrip()
        if rest.startswith("::"):
            line = line[: len(line) - len(rest)] + "\\x3a" + rest[1:]
        lines.append(line)
    return "".join(lines).replace("##[", "\\x23#[")


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
