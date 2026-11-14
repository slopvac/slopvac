"""``python -m slopvac_plugin.conformance [options] -- <plugin command...>``."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from .runner import load_declaration, run_conformance
from .schemas import MissingDependency
from .session import SuiteError

USAGE_EXIT = 2


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="slopvac-plugin-conformance",
        description="Drive plugin protocol v1 cases P1 to P19 against a plugin command. "
        "Exit 0 when every applicable case passes, 1 when a case fails, 2 when the suite cannot run.",
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        help="pack manifest (TOML) or declaration (JSON), for P19",
    )
    parser.add_argument(
        "--inputs", type=Path, help="JSON file with per-hook sample requests"
    )
    parser.add_argument("--only", help="comma-separated case ids, for example P1,P13")
    parser.add_argument(
        "--timeout", type=float, default=10.0, help="seconds to wait for a reply"
    )
    parser.add_argument(
        "--exit-timeout", type=float, default=2.0, help="seconds to wait for exit"
    )
    parser.add_argument(
        "--cancel-grace",
        type=float,
        default=2.0,
        help="seconds a plugin may take to answer after a cancel (section 6.3)",
    )
    parser.add_argument(
        "--case-budget",
        type=float,
        default=120.0,
        help="wall-clock seconds one case may take, all waits included",
    )
    parser.add_argument(
        "--suite-budget",
        type=float,
        default=900.0,
        help="wall-clock seconds the whole run may take",
    )
    parser.add_argument(
        "--env",
        action="append",
        default=[],
        metavar="NAME",
        help="pass this environment variable to the plugin. The plugin otherwise gets "
        "only PATH and LANG (and the basics Windows needs). Repeat for more names",
    )
    parser.add_argument("--format", choices=("text", "json"), default="text")
    parser.add_argument(
        "command", nargs=argparse.REMAINDER, help="-- <plugin command...>"
    )
    args = parser.parse_args(argv)
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("give the plugin command after --")
    try:
        report = run_conformance(
            command,
            declaration=load_declaration(args.manifest) if args.manifest else None,
            inputs=json.loads(args.inputs.read_text(encoding="utf-8"))
            if args.inputs
            else None,
            timeout=args.timeout,
            exit_timeout=args.exit_timeout,
            cancel_grace=args.cancel_grace,
            pass_env=args.env,
            case_budget=args.case_budget,
            suite_budget=args.suite_budget,
            only=args.only.split(",") if args.only else None,
        )
    except (SuiteError, MissingDependency, OSError, ValueError) as exc:
        print(f"conformance incomplete: {exc}", file=sys.stderr)
        return USAGE_EXIT
    print(report.to_json() if args.format == "json" else report.to_text())
    return report.exit_code


if __name__ == "__main__":
    sys.exit(main())
