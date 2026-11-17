"""A small evaluator for GitHub Actions ``if:`` expressions and job routing.

It covers the subset the repository's workflows use: ``!``, ``==``, ``!=``, ``&&``,
``||``, parentheses, string literals, dotted context lookups and the status
functions. Tests use it to run the real conditions from the workflow files.
"""

from __future__ import annotations

import math
import re
from collections.abc import Mapping

_TOKEN = re.compile(
    r"\s*(?:(?P<op>&&|\|\||==|!=|!|\(|\))"
    r"|'(?P<str>(?:[^']|'')*)'"
    r"|(?P<call>[A-Za-z]+)\(\)"
    r"|(?P<name>[A-Za-z_][\w.-]*))"
)
_STATUS = re.compile(r"\b(?:always|cancelled|success|failure)\(\)")


def _tokens(text: str) -> list[tuple[str, str]]:
    out: list[tuple[str, str]] = []
    pos = 0
    text = text.rstrip()
    while pos < len(text):
        match = _TOKEN.match(text, pos)
        if match is None or match.end() == pos:
            raise ValueError(f"cannot read the expression at {text[pos:]!r}")
        kind = match.lastgroup or ""
        value = match.group(kind)
        out.append((kind, value.replace("''", "'") if kind == "str" else value))
        pos = match.end()
    return out


def _truthy(value: object) -> bool:
    if isinstance(value, float):
        return value != 0 and not math.isnan(value)
    return bool(value)


def _number(value: object) -> float:
    if value is None or value == "":
        return 0.0
    if isinstance(value, bool):
        return float(value)
    try:
        return float(value)  # type: ignore[arg-type]
    except ValueError:
        return math.nan


def _equal(left: object, right: object) -> bool:
    if isinstance(left, str) and isinstance(right, str):
        return left.lower() == right.lower()
    if left is None and right is None:
        return True
    return _number(left) == _number(right)


class _Parser:
    def __init__(self, text: str, context: Mapping[str, object], status: str) -> None:
        self.tokens = _tokens(text)
        self.at = 0
        self.context = context
        self.status = status

    def peek(self) -> tuple[str, str] | None:
        return self.tokens[self.at] if self.at < len(self.tokens) else None

    def take(self) -> tuple[str, str]:
        token = self.peek()
        if token is None:
            raise ValueError("the expression ends too early")
        self.at += 1
        return token

    def is_op(self, op: str) -> bool:
        token = self.peek()
        return token is not None and token == ("op", op)

    def parse(self) -> object:
        value = self.or_()
        if self.peek() is not None:
            raise ValueError(f"unexpected {self.peek()!r}")
        return value

    def or_(self) -> object:
        value = self.and_()
        while self.is_op("||"):
            self.take()
            right = self.and_()
            value = value if _truthy(value) else right
        return value

    def and_(self) -> object:
        value = self.compare()
        while self.is_op("&&"):
            self.take()
            right = self.compare()
            value = right if _truthy(value) else value
        return value

    def compare(self) -> object:
        left = self.unary()
        for op in ("==", "!="):
            if self.is_op(op):
                self.take()
                same = _equal(left, self.unary())
                return same if op == "==" else not same
        return left

    def unary(self) -> object:
        if self.is_op("!"):
            self.take()
            return not _truthy(self.unary())
        return self.primary()

    def primary(self) -> object:
        kind, value = self.take()
        if kind == "op" and value == "(":
            inner = self.or_()
            if self.take() != ("op", ")"):
                raise ValueError("missing )")
            return inner
        if kind == "str":
            return value
        if kind == "call":
            if value == "always":
                return True
            if value not in ("cancelled", "success", "failure"):
                raise ValueError(f"unknown function {value}()")
            return self.status == value
        if kind == "name":
            return self.context.get(value)
        raise ValueError(f"unexpected {value!r}")


def evaluate(
    condition: str, context: Mapping[str, object], *, status: str = "success"
) -> bool:
    """Evaluate ``condition`` against dotted ``context`` keys, as Actions does.

    ``status`` is what the status functions report. A key that is not in
    ``context`` reads as null.
    """
    text = " ".join(condition.split())
    if text.startswith("${{") and text.endswith("}}"):
        text = text[3:-2]
    return _truthy(_Parser(text, context, status).parse())


def _needs(job: Mapping[str, object]) -> list[str]:
    needs = job.get("needs", [])
    return [needs] if isinstance(needs, str) else list(needs)  # type: ignore[arg-type]


def run_jobs(
    jobs: Mapping[str, Mapping[str, object]],
    context: Mapping[str, object],
    outputs: Mapping[str, Mapping[str, object]],
    *,
    fail: frozenset[str] = frozenset(),
) -> set[str]:
    """Return the jobs that run, when every job that runs succeeds except ``fail``.

    A job with no status function in its ``if`` is skipped when any needed job did
    not succeed. A job that names one, such as ``!cancelled()``, decides alone.
    A job only sees the jobs in its own ``needs``.
    """
    results: dict[str, str] = {}
    pending = dict(jobs)
    while pending:
        progressed = False
        for name, job in list(pending.items()):
            needs = _needs(job)
            if any(n not in results for n in needs):
                continue
            scope = dict(context)
            for need in needs:
                scope[f"needs.{need}.result"] = results[need]
                for key, value in outputs.get(need, {}).items():
                    scope[f"needs.{need}.outputs.{key}"] = value
            condition = job.get("if")
            if condition is not None and _STATUS.search(str(condition)):
                runs = evaluate(str(condition), scope)
            else:
                runs = all(results[n] == "success" for n in needs) and (
                    condition is None or evaluate(str(condition), scope)
                )
            results[name] = (
                ("failure" if name in fail else "success") if runs else "skipped"
            )
            del pending[name]
            progressed = True
        if not progressed:
            raise ValueError(f"jobs need each other: {sorted(pending)}")
    return {name for name, result in results.items() if result != "skipped"}
