"""Model command for `evals models`: one prompt on stdin, one answer on stdout.

    python -m evals.opencode_model --model amazon-bedrock/global.anthropic.claude-haiku-5-5 \
        --aws-profile <profile> --aws-region <region>

It runs `opencode run` in an empty temporary project with a bare agent that has
no tools and a one-line system prompt, so repository rules, plugins and tool
schemas do not change what the model answers.

Isolation is by construction, not by trust in the caller's environment:

- The child gets an allow-listed environment. No `OPENCODE_*` or `XDG_*` variable
  from the caller reaches it, and the XDG directories point inside the temporary
  project, so the user's global opencode config, plugins and database stay out.
- `PWD` matches the temporary working directory.
- Credentials come from the AWS profile files under `HOME`, which the AWS SDK
  reads. The adapter never reads or prints a credential.
- On timeout or interrupt the whole process group is killed and reaped.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from collections.abc import Mapping
from pathlib import Path

from . import procs

AGENT = "evalbare"
ALLOWED_ENV = (
    "PATH",
    "HOME",
    "USER",
    "LANG",
    "LC_ALL",
    "AWS_PROFILE",
    "AWS_REGION",
    "AWS_DEFAULT_REGION",
    "AWS_CONFIG_FILE",
    "AWS_SHARED_CREDENTIALS_FILE",
)


class ModelCallError(Exception):
    pass


def build_config(region: str | None, profile: str | None) -> dict:
    config: dict = {
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
    options = {k: v for k, v in (("region", region), ("profile", profile)) if v}
    if options:
        config["provider"] = {"amazon-bedrock": {"options": options}}
    return config


def isolated_env(
    tmp: Path, parent: Mapping[str, str], region: str | None, profile: str | None
) -> dict[str, str]:
    """The only environment the child sees."""
    env = {k: parent[k] for k in ALLOWED_ENV if k in parent}
    if profile:
        env["AWS_PROFILE"] = profile
    if region:
        env["AWS_REGION"] = region
    env.update(
        PWD=str(tmp),
        TMPDIR=str(tmp),
        XDG_CONFIG_HOME=str(tmp / "xdg-config"),
        XDG_DATA_HOME=str(tmp / "xdg-data"),
        XDG_STATE_HOME=str(tmp / "xdg-state"),
        XDG_CACHE_HOME=str(tmp / "xdg-cache"),
    )
    return env


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


def call(
    model: str,
    prompt: str,
    timeout: int = procs.ADAPTER_TIMEOUT,
    *,
    region: str | None = None,
    profile: str | None = None,
    parent_env: Mapping[str, str] | None = None,
    keep: Path | None = None,
) -> str:
    """Ask `model` once. `keep` names a directory to copy the temp project into, for audits."""
    parent = os.environ if parent_env is None else parent_env
    region = region or parent.get("AWS_REGION") or parent.get("AWS_DEFAULT_REGION")
    profile = profile or parent.get("AWS_PROFILE")
    with tempfile.TemporaryDirectory(prefix="slopvac-model-eval-") as raw:
        tmp = Path(raw).resolve()
        (tmp / "opencode.json").write_text(
            json.dumps(build_config(region, profile)), encoding="utf-8"
        )
        try:
            proc = procs.run_group(
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
                timeout=timeout,
                cwd=tmp,
                env=isolated_env(tmp, parent, region, profile),
            )
        finally:
            if keep:
                shutil.copytree(
                    tmp, keep, dirs_exist_ok=True, ignore_dangling_symlinks=True
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
    parser.add_argument("--timeout", type=int, default=procs.ADAPTER_TIMEOUT)
    parser.add_argument("--aws-region")
    parser.add_argument("--aws-profile")
    parser.add_argument(
        "--keep-dir", type=Path, help="copy the temp project here for an audit"
    )
    args = parser.parse_args(argv)
    if args.timeout >= procs.OUTER_TIMEOUT:
        parser.error(
            f"--timeout must be below the harness timeout of {procs.OUTER_TIMEOUT}s"
        )
    try:
        sys.stdout.write(
            call(
                args.model,
                sys.stdin.read(),
                args.timeout,
                region=args.aws_region,
                profile=args.aws_profile,
                keep=args.keep_dir,
            )
        )
    except (ModelCallError, subprocess.TimeoutExpired) as exc:
        print(f"opencode_model: {procs.redact(str(exc))}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
