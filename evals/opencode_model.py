"""Model command for `evals models`: one prompt on stdin, one answer on stdout.

    python -m evals.opencode_model --model amazon-bedrock/global.anthropic.claude-haiku-5-5

It runs `opencode run` in an empty temporary directory with a bare agent that has
no tools and a one-line system prompt, so repository rules and tool schemas do
not change what the model answers. The provider login stays with opencode, and
this script never reads or prints a credential.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path

AGENT = "evalbare"
CONFIG = {
    "$schema": "https://opencode.ai/config.json",
    "agent": {
        AGENT: {
            "mode": "primary",
            "description": "Bare agent for model evals",
            "prompt": "Follow the user instruction exactly. Output only what is asked.",
            "tools": {"*": False},
        }
    },
}


class ModelCallError(Exception):
    pass


def parse_events(stdout: str) -> str:
    """Join the text parts of an `opencode run --format json` stream."""
    parts: list[str] = []
    for line in stdout.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("type") == "error":
            message = (event.get("error") or {}).get("message", "unknown error")
            raise ModelCallError(str(message)[:300])
        if event.get("type") == "text":
            parts.append(event["part"].get("text", ""))
    return "".join(parts)


def call(model: str, prompt: str, timeout: int = 600) -> str:
    with tempfile.TemporaryDirectory(prefix="slopvac-model-eval-") as tmp:
        (Path(tmp) / "opencode.json").write_text(json.dumps(CONFIG), encoding="utf-8")
        proc = subprocess.run(
            [
                "opencode",
                "run",
                "-m",
                model,
                "--agent",
                AGENT,
                "--archived",
                "--format",
                "json",
            ],
            input=prompt,
            capture_output=True,
            text=True,
            cwd=tmp,
            timeout=timeout,
            check=False,
        )
    answer = parse_events(proc.stdout)
    if proc.returncode != 0 or not answer:
        raise ModelCallError(f"opencode exited {proc.returncode} with no answer")
    return answer


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--model", required=True, help="provider/model as `opencode models` lists it"
    )
    parser.add_argument("--timeout", type=int, default=600)
    args = parser.parse_args(argv)
    try:
        sys.stdout.write(call(args.model, sys.stdin.read(), args.timeout))
    except (ModelCallError, subprocess.TimeoutExpired) as exc:
        print(f"opencode_model: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
