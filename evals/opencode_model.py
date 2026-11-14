"""Model command for `evals models`: one prompt on stdin, one answer on stdout.

    python -m evals.opencode_model --model amazon-bedrock/global.anthropic.claude-haiku-5-5 \
        --aws-profile <profile> --aws-region <region>

It runs `opencode run` in an empty temporary project with a bare agent that has
no tools and a one-line system prompt, so repository rules, plugins and tool
schemas do not change what the model answers.

Isolation is by construction, not by trust in the caller's environment:

- `HOME`, the XDG directories, `TMPDIR` and `PWD` all point inside the temporary
  project, which holds only the bare config. The user's `~/.config/opencode`,
  `~/.opencode`, plugins, rules and auth never load.
- The child gets an allow-listed environment. No `OPENCODE_*` or caller `XDG_*`
  variable reaches it.
- AWS credentials are resolved up front with `aws configure export-credentials`
  for the chosen profile and handed over as the three standard variables. The
  child has no AWS profile files to read. The adapter never prints a credential.
- `opencode` stays in the adapter's process group, so the harness's group kill
  reaches it. On its own timeout the adapter kills `opencode` and everything
  below it.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import shutil
import subprocess
import sys
import tempfile
from collections.abc import Callable, Mapping
from pathlib import Path

from . import procs

AGENT = "evalbare"
# Variables copied from the caller. HOME is not among them.
ALLOWED_ENV = ("PATH", "USER", "LANG", "LC_ALL", "AWS_REGION", "AWS_DEFAULT_REGION")
# Used only to run `aws configure export-credentials`, never given to opencode.
CREDENTIAL_ENV = (
    "PATH",
    "HOME",
    "USER",
    "LANG",
    "LC_ALL",
    "AWS_CONFIG_FILE",
    "AWS_SHARED_CREDENTIALS_FILE",
    "AWS_PROFILE",
    "AWS_REGION",
    "AWS_DEFAULT_REGION",
)
CREDENTIAL_TIMEOUT = 60
MIN_LIFETIME = 600  # seconds a credential must still be valid when an attempt starts
CREDENTIAL_KEYS = {
    "AccessKeyId": "AWS_ACCESS_KEY_ID",
    "SecretAccessKey": "AWS_SECRET_ACCESS_KEY",
    "SessionToken": "AWS_SESSION_TOKEN",
}


class ModelCallError(Exception):
    pass


def build_config(region: str | None) -> dict:
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
    if region:
        config["provider"] = {"amazon-bedrock": {"options": {"region": region}}}
    return config


def isolated_env(
    tmp: Path,
    parent: Mapping[str, str],
    region: str | None,
    credentials: Mapping[str, str] | None = None,
) -> dict[str, str]:
    """The only environment the child sees. Its HOME is an empty directory."""
    env = {k: parent[k] for k in ALLOWED_ENV if k in parent}
    if region:
        env["AWS_REGION"] = region
    env.update(credentials or {})
    home = tmp / "home"
    env.update(
        HOME=str(home),
        PWD=str(tmp),
        TMPDIR=str(tmp),
        XDG_CONFIG_HOME=str(tmp / "xdg-config"),
        XDG_DATA_HOME=str(tmp / "xdg-data"),
        XDG_STATE_HOME=str(tmp / "xdg-state"),
        XDG_CACHE_HOME=str(tmp / "xdg-cache"),
    )
    return env


def export_credentials(
    profile: str, parent: Mapping[str, str]
) -> tuple[dict[str, str], dt.datetime | None]:
    """Resolve an AWS profile to the three credential variables and their expiry.

    The helper stays in the caller's process group (`own_group=False`), so the
    harness's group kill reaches it when a run is cancelled. Nothing is printed.
    """
    env = {k: parent[k] for k in CREDENTIAL_ENV if k in parent}
    try:
        proc = procs.run_group(
            [
                "aws",
                "configure",
                "export-credentials",
                "--profile",
                profile,
                "--format",
                "process",
            ],
            timeout=CREDENTIAL_TIMEOUT,
            env=env,
            own_group=False,
        )
    except FileNotFoundError:
        raise ModelCallError("the aws CLI is required to resolve credentials") from None
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError:
        data = {}
    if proc.returncode != 0 or "AccessKeyId" not in data:
        raise ModelCallError(f"could not resolve credentials for profile {profile!r}")
    expiry = None
    if data.get("Expiration"):
        expiry = dt.datetime.fromisoformat(str(data["Expiration"]))
    return (
        {
            env_name: data[key]
            for key, env_name in CREDENTIAL_KEYS.items()
            if key in data
        },
        expiry,
    )


def fresh_credentials(
    profile: str,
    parent: Mapping[str, str],
    now: Callable[[], dt.datetime] = lambda: dt.datetime.now(dt.UTC),
) -> dict[str, str]:
    """Credentials with at least MIN_LIFETIME seconds left; export again once if short."""
    for _ in range(2):
        credentials, expiry = export_credentials(profile, parent)
        if expiry is None or (expiry - now()).total_seconds() >= MIN_LIFETIME:
            return credentials
    raise ModelCallError(
        f"credentials for profile {profile!r} expire in under {MIN_LIFETIME // 60} minutes"
    )


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
    credentials: Mapping[str, str] | None = None,
) -> str:
    """Ask `model` once. `keep` names a directory to copy the temp project into, for audits."""
    parent = os.environ if parent_env is None else parent_env
    region = region or parent.get("AWS_REGION") or parent.get("AWS_DEFAULT_REGION")
    profile = profile or parent.get("AWS_PROFILE")
    if credentials is None and profile:
        credentials = fresh_credentials(profile, parent)
    with tempfile.TemporaryDirectory(prefix="slopvac-model-eval-") as raw:
        tmp = Path(raw).resolve()
        (tmp / "home").mkdir()
        (tmp / "opencode.json").write_text(
            json.dumps(build_config(region)), encoding="utf-8"
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
                env=isolated_env(tmp, parent, region, credentials),
                own_group=False,
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
