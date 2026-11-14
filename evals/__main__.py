"""Command line for the eval harness: python -m evals <command>.

run     lint the corpus with an engine and store the results
score   score stored results (the offline tier; starts no engine)
ab      paired comparison of two stored results
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

from . import ab as ab_mod
from . import corpuslib as lib
from . import runner, scoring


def _cache(args: argparse.Namespace) -> Path:
    return Path(
        args.cache or os.environ.get("SLOPVAC_EVALS_CACHE") or lib.DEFAULT_CACHE
    )


def _emit(data: object, out: str | None) -> None:
    text = json.dumps(data, indent=1, sort_keys=True) + "\n"
    if out:
        Path(out).write_text(text, encoding="utf-8")
    else:
        sys.stdout.write(text)


def cmd_run(args: argparse.Namespace) -> int:
    manifest = lib.load_manifest()
    engine = runner.Engine.parse(args.engine, Path(args.cwd) if args.cwd else None)
    base = (
        runner.available_selector(_cache(args))
        if args.available
        else runner.default_selector
    )
    wanted = set(args.only or ())

    def select(doc):
        return base(doc) and (not wanted or doc["origin"] in wanted)

    try:
        results = runner.run_engine(engine, manifest, _cache(args), select=select)
    except runner.EngineError as exc:
        print(f"evals run: {exc}", file=sys.stderr)
        return 2
    runner.dump_results(results, Path(args.out))
    return 0


def cmd_score(args: argparse.Namespace) -> int:
    manifest = lib.load_manifest()
    results = runner.load_results(Path(args.results))
    try:
        runner.check_corpus(results, manifest)
    except runner.EngineError as exc:
        print(f"evals score: {exc}", file=sys.stderr)
        return 2
    labels = scoring.load_verdicts(Path(args.labels)) if args.labels else None
    card = scoring.score(
        manifest, results, split=args.split, labels=labels, resamples=args.resamples
    )
    _emit(card, args.out)
    return 0


def cmd_ab(args: argparse.Namespace) -> int:
    manifest = lib.load_manifest()
    try:
        report = ab_mod.ab(
            manifest,
            runner.load_results(Path(args.a)),
            runner.load_results(Path(args.b)),
            split=args.split,
            resamples=args.resamples,
        )
    except runner.EngineError as exc:
        print(f"evals ab: {exc}", file=sys.stderr)
        return 2
    report["regressions"] = ab_mod.regressions(report)
    _emit(report, args.out)
    return 1 if report["regressions"] and args.fail_on_regression else 0


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="python -m evals", description=__doc__)
    sub = p.add_subparsers(dest="command", required=True)

    r = sub.add_parser("run", help="lint the corpus and store results")
    r.add_argument("--engine", default="python", help="'python' or a slopvac command")
    r.add_argument("--cwd", help="working directory for the engine")
    r.add_argument("--cache", help="corpus cache directory")
    r.add_argument("--only", action="append", help="limit to an origin class")
    r.add_argument(
        "--available", action="store_true", help="skip documents with no local text"
    )
    r.add_argument("--out", required=True)
    r.set_defaults(func=cmd_run)

    s = sub.add_parser("score", help="score stored results")
    s.add_argument("results")
    s.add_argument("--split", default="all", choices=("all", "tuning", "holdout"))
    s.add_argument(
        "--labels", help="adjudicated labels (JSON Lines) for precision and recall"
    )
    s.add_argument("--resamples", type=int, default=1000)
    s.add_argument("--out")
    s.set_defaults(func=cmd_score)

    a = sub.add_parser("ab", help="paired comparison of two stored results")
    a.add_argument("a")
    a.add_argument("b")
    a.add_argument("--split", default="all", choices=("all", "tuning", "holdout"))
    a.add_argument("--resamples", type=int, default=1000)
    a.add_argument("--fail-on-regression", action="store_true")
    a.add_argument("--out")
    a.set_defaults(func=cmd_ab)
    return p


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
