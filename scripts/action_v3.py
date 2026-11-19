#!/usr/bin/env python3
"""Entry points for the slopvac 3.0 composite GitHub Action (action-v3/action.yml).

``sync``      run ``slopvac plugins sync`` when the project has a pack lockfile.
``lint``      run the binary once per report format and keep its gate exit code.
``report``    post the rdjsonl findings with reviewdog; never gates the job.

The binary renders every format itself; this script never reformats findings.
Target resolution reuses ``action_entry.py resolve-targets``.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import unicodedata
from collections.abc import Mapping, Sequence
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from action_entry import ActionError, _read_paths

REVIEW = "github-pr-review"
PR_ANNOTATIONS = "github-pr-annotations"
CHECK = "github-check"
NONE = "none"

# Exit codes of the binary. Only FINDINGS is a verdict that a caller may soften.
OK = 0
FINDINGS = 1
INCOMPLETE = 2

LOCKFILE = ".slopvac/packs.lock"

_SUMMARY_KEYS = ("score", "findings", "errors", "warnings", "suggestions", "passed")


def _write_outputs(env: Mapping[str, str], values: Mapping[str, object]) -> None:
    target = env.get("GITHUB_OUTPUT")
    if not target:
        return
    with Path(target).open("a", encoding="utf-8") as handle:
        handle.writelines(f"{name}={value}\n" for name, value in values.items())


def workspace_path(raw: str, env: Mapping[str, str]) -> Path:
    """Resolve a report destination and keep it inside ``$GITHUB_WORKSPACE``.

    A relative path is taken from the workspace. The path is normalised
    lexically, then refused when it leaves the workspace or when the workspace
    or any existing component below it, the file included, is a symlink
    (CWE-22, CWE-59). The normalised path is the one the caller writes to, so
    ``a/link/../b`` cannot resolve differently on disk than it did here.
    """
    workspace = Path(env.get("GITHUB_WORKSPACE") or Path.cwd())
    if workspace.is_symlink():
        raise ActionError(f"the workspace {str(workspace)!r} is a symlink")
    base = Path(os.path.realpath(workspace))
    if "\0" in raw:
        raise ActionError("a report path contains a NUL byte")
    candidate = Path(os.path.normpath(base / raw))
    if candidate == base or base not in candidate.parents:
        raise ActionError(f"report path {raw!r} is outside the workspace")
    current = base
    for part in candidate.relative_to(base).parts:
        current /= part
        if current.is_symlink():
            raise ActionError(f"report path {raw!r} goes through a symlink")
    return candidate


def choose_reporter(env: Mapping[str, str]) -> tuple[str, bool]:
    """Return ``(reviewdog reporter, suggestions)`` for this event.

    ``auto`` tries a pull request review with suggestions only for a pull
    request from the same repository that Dependabot did not open. The token
    may still be read-only, so ``report`` falls back to annotations when the
    review fails. Other pull requests use annotations, which need no write
    token. Outside pull requests the reporter is a check run.
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


def _binary(env: Mapping[str, str]) -> str:
    return env.get("BINARY") or "slopvac"


def _command(
    env: Mapping[str, str], fmt: str, out: Path, targets: Sequence[str]
) -> list[str]:
    command = [_binary(env), "--offline"]  # policies.md 1.4: no opt-out
    command += ["lint", "--format", fmt, "--out", str(out)]
    if env.get("CONFIG"):
        command += ["--config", env["CONFIG"]]
    if env.get("MIN_SCORE"):
        command += ["--min-score", env["MIN_SCORE"]]
    if env.get("MAX_PER_100"):
        command += ["--max-per-100-words", env["MAX_PER_100"]]
    # "--" keeps a file named like an option from being parsed as one.
    return [*command, "--", *targets]


# Characters that sanitize.rs `is_unsafe` also covers: controls, format
# characters (bidi overrides), line and paragraph separators, private use and
# surrogates.
_UNSAFE_CATEGORIES = frozenset({"Cc", "Cf", "Zl", "Zp", "Co", "Cs"})


def escape_log_line(line: str) -> str:
    r"""Escape one line of untrusted text so the runner reads no command in it.

    Mirrors ``workflow_data`` in crates/slopvac-sanitize (TM-13/O4): ``%``
    becomes ``%25``, ``::`` becomes ``%3A%3A``, ``##[`` is neutralised, and a
    backslash or an unsafe character becomes ``\\`` or ``\u{..}``. Tab stays.
    Callers split on newlines first, so each line is emitted as a line.
    """
    out = []
    for char in line:
        if char == "%":
            out.append("%25")
        elif char == "\\":
            out.append("\\\\")
        elif char != "\t" and unicodedata.category(char) in _UNSAFE_CATEGORIES:
            out.append(f"\\u{{{ord(char):x}}}")
        else:
            out.append(char)
    return "".join(out).replace("::", "%3A%3A").replace("##[", "\\u{23}#[")


def _relay_stderr(raw: bytes) -> None:
    """Re-emit the binary's stderr line by line through ``escape_log_line``.

    The binary echoes PR-controlled text (a file name may hold a newline and
    ``::error::``), so its stderr must never reach the job log raw.
    """
    for line in raw.decode("utf-8", errors="replace").splitlines():
        print(escape_log_line(line), file=sys.stderr)


def _run(command: Sequence[str], env: Mapping[str, str]) -> int:
    """Run the binary, relay its stderr escaped, and return its exit code."""
    done = subprocess.run(
        command, check=False, env={**os.environ, **env}, stderr=subprocess.PIPE
    )
    _relay_stderr(done.stderr)
    return done.returncode


def sync(env: Mapping[str, str] = os.environ) -> int:
    """Run ``plugins sync`` from the lockfile, as its own step before the lint.

    A project with no lockfile needs no packs, so there is nothing to sync.
    Any failure (the binary exits 2 on a missing or mismatched digest) fails
    the action.
    """
    lockfile = Path(env.get("LOCKFILE") or LOCKFILE)
    if not lockfile.is_file():
        print(f"::notice title=slopvac::no {lockfile}; skipping plugins sync")
        return OK
    code = _run([_binary(env), "plugins", "sync"], env)
    if code != OK:
        print(f"::error title=slopvac::plugins sync failed (status {code})")
        return INCOMPLETE
    return OK


def lint(env: Mapping[str, str] = os.environ) -> int:
    """Run the gate (JSON) first, then each report the caller asked for.

    The JSON run decides the findings verdict, so the binary alone is the gate.
    The other runs only render. Only exit 1 from the JSON run is softened by
    ``fail-on-findings``. Exit 2, any other code and death by signal from any
    run mean the check was incomplete and always fail.
    """
    temp = Path(env.get("RUNNER_TEMP") or ".")
    targets = _read_paths(Path(env["PATH_LIST"])) if env.get("PATH_LIST") else []
    if not targets:
        _write_outputs(env, {"passed": "true"})
        print("::notice title=slopvac::no target paths; nothing to lint")
        return 0

    reports = {
        "json": Path(env.get("REPORT_JSON") or temp / "slopvac-report.json"),
        "rdjsonl": Path(env.get("RDJSONL_FILE") or "slopvac.rdjsonl"),
    }
    if env.get("SARIF", "true") == "true":
        reports["sarif"] = Path(env.get("SARIF_FILE") or "slopvac.sarif")
    if env.get("CODECLIMATE", "true") == "true":
        reports["codeclimate"] = Path(
            env.get("CODECLIMATE_FILE") or "slopvac-codeclimate.json"
        )
    # The JSON gate report lives in the runner temp directory. Every other
    # destination can come from an input, so none may leave the workspace.
    for fmt in reports.keys() - {"json"}:
        reports[fmt] = workspace_path(str(reports[fmt]), env)

    verdict = OK
    incomplete = False
    for fmt, out in reports.items():
        out.parent.mkdir(parents=True, exist_ok=True)
        code = _run(_command(env, fmt, out, targets), env)
        if fmt == "json":
            verdict = code
        # Renderers may exit 1 for findings; that is not a failure to render.
        if code not in {OK, FINDINGS}:
            incomplete = True
            print(f"::error title=slopvac::{fmt} run ended abnormally (status {code})")

    outputs: dict[str, object] = {
        f"{fmt}-file": out for fmt, out in reports.items() if out.is_file()
    }
    if reports["json"].is_file():
        summary = json.loads(reports["json"].read_text(encoding="utf-8")).get(
            "summary", {}
        )
        outputs |= {
            key: str(summary[key]).lower() for key in _SUMMARY_KEYS if key in summary
        }
    _write_outputs(env, outputs)

    if incomplete:
        return INCOMPLETE
    if verdict == FINDINGS and env.get("FAIL_ON_FINDINGS", "true") != "false":
        return FINDINGS
    return OK


def report(env: Mapping[str, str] = os.environ) -> int:
    """Post the rdjsonl findings with reviewdog. Reporting never gates the job.

    A review needs a write token that the workflow may not grant, and the
    token's permissions are not observable. So when the review reporter fails,
    retry with annotations, which need no write permission. A missing reviewdog
    or a failed post only warns: the slopvac exit code is the gate.
    """
    reporter, _ = choose_reporter(env)
    if reporter == NONE:
        return 0
    source = Path(env.get("RDJSONL") or "")
    if not source.is_file():
        print("::warning title=slopvac::no rdjsonl report; skipping reviewdog")
        return 0
    attempts = [reporter, PR_ANNOTATIONS] if reporter == REVIEW else [reporter]
    code = OK
    for index, name in enumerate(attempts):
        command = [
            env.get("REVIEWDOG") or "reviewdog",
            "-f=rdjsonl",
            "-name=slopvac",
            f"-reporter={name}",
            "-filter-mode=added",
            "-fail-level=none",
        ]
        try:
            with source.open("rb") as stdin:
                code = subprocess.run(
                    command, check=False, stdin=stdin, env={**os.environ, **env}
                ).returncode
        except OSError as err:
            print(f"::warning title=slopvac::reviewdog is unavailable: {err}")
            return 0
        if code == 0:
            return 0
        if index + 1 < len(attempts):
            print(
                f"::warning title=slopvac::reviewdog {name} exited {code}; "
                f"falling back to {attempts[index + 1]}"
            )
    print(f"::warning title=slopvac::reviewdog could not post findings (status {code})")
    return 0


def main(argv: Sequence[str]) -> int:
    commands = {"sync": sync, "lint": lint, "report": report}
    if len(argv) != 2 or argv[1] not in commands:
        print("usage: action_v3.py sync|lint|report", file=sys.stderr)
        return 2
    try:
        return commands[argv[1]]()
    except ActionError as err:
        print(f"::error title=slopvac::{err}")
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
