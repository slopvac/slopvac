#!/usr/bin/env python3
"""Entry points for the slopvac 3.0 composite GitHub Action (action-v3/action.yml).

``reporter``  choose the reviewdog reporter for this event.
``lint``      run the binary once per report format and keep its gate exit code.

The binary renders every format itself; this script never reformats findings.
Target resolution reuses ``action_entry.py resolve-targets``.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from collections.abc import Mapping, Sequence
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from action_entry import ActionError, _read_paths

REVIEW = "github-pr-review"
PR_ANNOTATIONS = "github-pr-annotations"
CHECK = "github-check"
NONE = "none"

_SUMMARY_KEYS = ("score", "findings", "errors", "warnings", "suggestions", "passed")


def _write_outputs(env: Mapping[str, str], values: Mapping[str, object]) -> None:
    target = env.get("GITHUB_OUTPUT")
    if not target:
        return
    with Path(target).open("a", encoding="utf-8") as handle:
        handle.writelines(f"{name}={value}\n" for name, value in values.items())


def choose_reporter(env: Mapping[str, str]) -> tuple[str, bool]:
    """Return ``(reviewdog reporter, suggestions)`` for this event.

    ``auto`` posts a pull request review with suggestions when the token can
    write: a pull request from the same repository, not from Dependabot (whose
    token is read-only). Otherwise it falls back to annotations that need no
    write token. Outside pull requests it reports through a check run.
    """
    requested = env.get("REPORTER", "auto") or "auto"
    if requested not in {"auto", "review", "annotations", "none"}:
        raise ActionError(
            f"reporter must be auto, review, annotations or none, got {requested!r}"
        )
    if requested == "none":
        return NONE, False
    if env.get("EVENT_NAME") not in {"pull_request", "pull_request_target"}:
        return CHECK, False
    same_repo = bool(env.get("HEAD_REPO")) and env.get("HEAD_REPO") == env.get(
        "BASE_REPO"
    )
    can_write = same_repo and env.get("ACTOR") != "dependabot[bot]"
    if requested == "annotations" or not can_write:
        return PR_ANNOTATIONS, False
    return REVIEW, True


def _command(
    env: Mapping[str, str], fmt: str, out: Path, targets: Sequence[str]
) -> list[str]:
    command = [env.get("BINARY") or "slopvac"]
    if env.get("OFFLINE", "true") == "true":
        command.append("--offline")
    command += ["lint", "--format", fmt, "--out", str(out)]
    if env.get("CONFIG"):
        command += ["--config", env["CONFIG"]]
    if env.get("MIN_SCORE"):
        command += ["--min-score", env["MIN_SCORE"]]
    if env.get("MAX_PER_100"):
        command += ["--max-per-100-words", env["MAX_PER_100"]]
    return [*command, *targets]


def _run(command: Sequence[str], env: Mapping[str, str]) -> int:
    return subprocess.run(command, check=False, env={**os.environ, **env}).returncode


def lint(env: Mapping[str, str] = os.environ) -> int:
    """Run the gate (JSON) first, then each report the caller asked for.

    The JSON run decides the exit code, so the binary alone is the gate. The
    other runs only render. An exit of 2 from any run means the check was
    incomplete and is never softened by ``fail-on-findings``.
    """
    temp = Path(env.get("RUNNER_TEMP") or ".")
    targets = _read_paths(Path(env["PATH_LIST"])) if env.get("PATH_LIST") else []
    if not targets:
        _write_outputs(env, {"passed": "true"})
        print("::notice title=slopvac::no target paths; nothing to lint")
        return 0

    reports = {
        "json": Path(env.get("REPORT_JSON") or temp / "slopvac-report.json"),
        "rdjsonl": Path(env.get("RDJSONL_FILE") or temp / "slopvac.rdjsonl"),
    }
    if env.get("SARIF", "true") == "true":
        reports["sarif"] = Path(env.get("SARIF_FILE") or "slopvac.sarif")
    if env.get("CODECLIMATE", "true") == "true":
        reports["codeclimate"] = Path(
            env.get("CODECLIMATE_FILE") or "slopvac-codeclimate.json"
        )

    gate = _run(_command(env, "json", reports["json"], targets), env)
    for fmt, out in reports.items():
        if fmt != "json" and _run(_command(env, fmt, out, targets), env) == 2:
            gate = 2

    outputs: dict[str, object] = {f"{fmt}-file": out for fmt, out in reports.items()}
    if reports["json"].is_file():
        summary = json.loads(reports["json"].read_text(encoding="utf-8")).get(
            "summary", {}
        )
        outputs |= {
            key: str(summary[key]).lower() for key in _SUMMARY_KEYS if key in summary
        }
    _write_outputs(env, outputs)

    if gate == 2:
        return 2
    return 0 if gate == 0 or env.get("FAIL_ON_FINDINGS", "true") == "false" else gate


def main(argv: Sequence[str]) -> int:
    if len(argv) != 2 or argv[1] not in {"reporter", "lint"}:
        print("usage: action_v3.py reporter|lint", file=sys.stderr)
        return 2
    try:
        if argv[1] == "reporter":
            reporter, suggestions = choose_reporter(os.environ)
            _write_outputs(
                os.environ,
                {"reporter": reporter, "suggestions": str(suggestions).lower()},
            )
            return 0
        return lint()
    except ActionError as err:
        print(f"::error title=slopvac::{err}")
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
