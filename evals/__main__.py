"""Command line for the eval harness: python -m evals <command>.

run     lint the corpus with an engine and store the results
score   score stored results (the offline tier; starts no engine)
ab      paired comparison of two stored results
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import os
import sys
from collections import Counter
from pathlib import Path

from . import ab as ab_mod
from . import corpuslib as lib
from . import gate, labelling, models, procs, runner, scoring


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
    if args.skip_format:
        engine = dataclasses.replace(
            engine, unsupported=engine.unsupported | frozenset(args.skip_format)
        )
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


def _split_cards(args: argparse.Namespace, manifest: dict) -> dict[str, dict]:
    results = runner.load_results(Path(args.results))
    runner.check_corpus(results, manifest)
    labels = scoring.load_verdicts(Path(args.labels)) if args.labels else None
    return {
        split: scoring.score(manifest, results, split=split, labels=labels)
        for split in gate.SPLITS
    }


def cmd_baseline(args: argparse.Namespace) -> int:
    manifest = lib.load_manifest()
    try:
        cards = _split_cards(args, manifest)
    except runner.EngineError as exc:
        print(f"evals baseline: {exc}", file=sys.stderr)
        return 2
    gate.dump_baseline(gate.make_baseline(cards), Path(args.out))
    return 0


def cmd_gate(args: argparse.Namespace) -> int:
    mode = args.mode or os.environ.get("SLOPVAC_EVALS_GATE", "report")
    if mode not in ("report", "enforce"):
        print(
            f"evals gate: mode must be report or enforce, not {mode!r}", file=sys.stderr
        )
        return 2
    manifest = lib.load_manifest()
    try:
        baseline = gate.load_baseline(Path(args.baseline))
        runner.check_corpus({"corpus": baseline.get("corpus")}, manifest)
        cards = _split_cards(args, manifest)
    except (OSError, ValueError, runner.EngineError) as exc:
        print(f"evals gate: {exc}", file=sys.stderr)
        return 2
    verdicts = scoring.load_verdicts(Path(args.labels)) if args.labels else {}
    rows = [{"rule": key[1], "label": label} for key, label in verdicts.items()]
    result = gate.compare(baseline, cards, labels=rows)
    text = gate.markdown(result, mode)
    if args.out:
        Path(args.out).write_text(text, encoding="utf-8")
    sys.stdout.write(text)
    return 1 if not result["passed"] and mode == "enforce" else 0


def _model_tasks(args: argparse.Namespace) -> tuple[list[dict], list[dict]]:
    notes = {}
    rules_dir = Path(args.rules_dir)
    if rules_dir.is_dir():
        notes = labelling.rule_notes(rules_dir)
    tasks = models.build_tasks(
        lib.load_manifest(), _cache(args), per_type=args.per_type, notes=notes
    )
    return tasks, models.load_models(Path(args.models))


def cmd_models(args: argparse.Namespace) -> int:
    try:
        if args.models_command == "summarize":
            docs = [models.load(Path(path)) for path in args.results]
            summary = models.summarize(
                models.merge_results(*docs) if len(docs) > 1 else docs[0]
            )
            _emit(summary, args.out)
            return 0
        if args.models_command == "recommend":
            results = models.load(Path(args.results))
            _emit(models.recommendations(results, publish=args.publish), args.out)
            return 0
        if args.models_command == "cost":
            report = models.estimate_cost(
                models.load_usage_records(Path(args.path)),
                models.load_prices(Path(args.prices)),
            )
            _emit(report, args.out)
            return 0
        tasks, roster = _model_tasks(args)
        if bool(args.only_model) != bool(args.candidates_from):
            raise models.ModelEvalError(
                "--only-model and --candidates-from go together"
            )
        candidates = (
            models.Journal(Path(args.candidates_from)) if args.candidates_from else None
        )
        counted = models.plan(tasks, roster, args.repeats, args.only_model)
        models.frozen_candidates(  # fail before any call if a candidate is missing
            tasks, roster, args.repeats, args.only_model, candidates
        )
        if args.models_command == "plan":
            _emit(counted, None)
            return 0
        if args.confirm_calls != counted["total_calls"]:
            print(
                f"evals models run: the plan is {counted['total_calls']} calls; "
                "pass that number to --confirm-calls to approve it",
                file=sys.stderr,
            )
            return 2
        engine = runner.Engine.parse(args.engine)
        out = Path(args.out) if args.out else models.default_output(models.today())
        journal_path = out.with_suffix(".journal.jsonl")
        ledger_path = out.with_suffix(".ledger.jsonl")
        if args.candidates_from:
            models.reject_aliases(
                Path(args.candidates_from),
                out=out,
                journal=journal_path,
                ledger=ledger_path,
                lock=Path(str(ledger_path) + ".lock"),
            )
        if (
            journal_path.is_file()
            and journal_path.stat().st_size
            and not ledger_path.is_file()
        ):
            print(
                f"evals models run: {journal_path.name} exists without {ledger_path.name}, "
                "so its spent invocations are unknown; start with a fresh --out path",
                file=sys.stderr,
            )
            return 2
        journal = models.Journal(journal_path)
        try:
            with models.Budget(args.max_invocations, ledger_path) as budget:
                results = models.execute(
                    tasks,
                    roster,
                    args.repeats,
                    models.make_linter(engine),
                    workers=args.workers,
                    budget=budget,
                    journal=journal,
                    only_model=args.only_model,
                    candidates=candidates,
                )
        except models.BudgetExceeded as exc:
            print(
                f"evals models run: {exc}; {journal.path} keeps the finished calls",
                file=sys.stderr,
            )
            return 2
        models.dump(results, out)
        return 0
    except (
        models.ModelEvalError,
        runner.EngineError,
        procs.UnsupportedPlatform,
    ) as exc:
        print(f"evals models: {exc}", file=sys.stderr)
        return 2


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


DEFAULT_RULES_DIR = lib.EVALS_DIR.parent / "packages/slopvac-lint/src/slopvac/rules"


def _paths(args: argparse.Namespace) -> dict[str, Path]:
    d = Path(args.dir)
    return {
        "items": d / "items.jsonl",
        "judgments": d / "judgments",
        "human": d / "human.jsonl",
        "labels": d / "labels.jsonl",
        "revisions": d / "revisions.jsonl",
        "disputes": d / "disputes.jsonl",
        "spot": d / "spot-check.json",
        "stats": d / "stats.json",
    }


def cmd_label_sample(args: argparse.Namespace) -> int:
    p = _paths(args)
    existing = labelling.read_jsonl(p["items"])
    manifest = lib.load_manifest()
    results = runner.load_results(Path(args.results))
    try:
        runner.check_corpus(results, manifest)
    except runner.EngineError as exc:
        print(f"evals label sample: {exc}", file=sys.stderr)
        return 2
    new = labelling.sample_items(
        manifest,
        results,
        per_rule=args.per_rule,
        existing=[i["id"] for i in existing],
    )
    if args.dry_run:
        per_rule = Counter(i["rule"] for i in new)
        for rule in sorted(per_rule):
            print(f"{rule}\t{per_rule[rule]}")
        print(f"total\t{len(new)}")
        return 0
    labelling.write_jsonl(p["items"], [*existing, *new])
    print(f"{len(new)} new items, {len(existing) + len(new)} total")
    return 0


def _judge_start_refusal(judgments: Path, out: Path, ledger: Path) -> str | None:
    """Why a judge run must not start, or None. The ledger holds the spent count.

    Every artifact an earlier run leaves (the judgments file of any size and the
    ledger lock) with no ledger would let a fresh ledger reset the ceiling.
    """
    wanted = {out.name, ledger.name, ledger.name + ".lock"}
    folded = {w.lower() for w in wanted}
    if judgments.is_dir():
        strays = sorted(
            f.name
            for f in judgments.iterdir()
            if f.name not in wanted and f.name.lower() in folded
        )
        if strays:
            return (
                f"{', '.join(strays)} differs from the family files only in letter "
                "case, so a ledger could fork; rename or remove it"
            )
    if not ledger.exists():
        lock = ledger.with_name(ledger.name + ".lock")
        left = [f.name for f in (out, lock) if f.exists()]
        if left:
            return (
                f"{' and '.join(left)} exists without {ledger.name}, so its spent "
                "invocations are unknown; start with a fresh --dir"
            )
    return None


def cmd_label_judge(args: argparse.Namespace) -> int:
    p = _paths(args)
    out = p["judgments"] / f"{args.family}.jsonl"
    try:
        labelling.check_identity(args.family, args.model)  # before any ledger exists
    except runner.EngineError as exc:
        print(f"evals label judge: {exc}", file=sys.stderr)
        return 2
    ledger = out.with_suffix(".ledger.jsonl")
    try:
        refusal = _judge_start_refusal(p["judgments"], out, ledger)
        if refusal:
            print(f"evals label judge: {refusal}", file=sys.stderr)
            return 2
        earlier = labelling.read_jsonl(out)
        done = labelling.completed(earlier, args.model)
        spent = labelling.failed_attempts(earlier, args.model)
        with models.Budget(args.max_invocations, ledger) as budget:
            rows = labelling.judge_items(
                labelling.read_jsonl(p["items"]),
                labelling.command_asker(args.provider_cmd),
                family=args.family,
                model=args.model,
                manifest=lib.load_manifest(),
                cache=_cache(args),
                notes=labelling.rule_notes(Path(args.rules_dir)),
                done=done,
                attempts=spent,
                sink=lambda row: labelling.append_jsonl(out, [row]),
                budget=budget,
            )
    except models.BudgetExceeded as exc:
        print(
            f"evals label judge: {exc}; {out} keeps the finished judgments",
            file=sys.stderr,
        )
        return 2
    except (models.ModelEvalError, runner.EngineError, OSError) as exc:
        print(f"evals label judge: {exc}", file=sys.stderr)
        return 2
    written = sum(r.get("status") != labelling.FAILED for r in rows)
    print(f"{written} judgments written to {out}")
    skipped = labelling.skipped_pairs(labelling.read_jsonl(out), args.model)
    if skipped:
        print(
            f"skipped {len(skipped)} pair{'s' if len(skipped) != 1 else ''} after "
            f"{labelling.MAX_ATTEMPTS} failed attempts (no judgment; the merge "
            "queues them for review):"
        )
        for item, order in skipped:
            print(f"  {item} {order}")
    return 0


def cmd_label_mark_failed(args: argparse.Namespace) -> int:
    """Record failed attempts that no judgments row holds, so `judge` counts them."""
    p = _paths(args)
    out = p["judgments"] / f"{args.family}.jsonl"
    try:
        labelling.check_identity(args.family, args.model)
        if args.attempts < 1:
            raise runner.EngineError("--attempts must be at least 1")
        if args.order not in labelling.ORDERS:
            raise runner.EngineError(
                f"unknown order {args.order!r}; use one of {', '.join(labelling.ORDERS)}"
            )
        if args.item not in {i["id"] for i in labelling.read_jsonl(p["items"])}:
            raise runner.EngineError(f"unknown item {args.item!r}")
        row = {
            "item": args.item,
            "family": args.family,
            "model": args.model,
            "order": args.order,
            "prompt_version": labelling.PROMPT_VERSION,
            "status": labelling.FAILED,
            "error": "provider-exit",
            "recorded": "manual",
        }
        labelling.append_jsonl(out, [row] * args.attempts)
    except (runner.EngineError, OSError) as exc:
        print(f"evals label mark-failed: {exc}", file=sys.stderr)
        return 2
    print(f"{args.attempts} failed attempts recorded in {out}")
    return 0


def cmd_label_merge(args: argparse.Namespace) -> int:
    p = _paths(args)
    try:
        judgments, ignored = labelling.usable_judgments(
            [
                r
                for f in sorted(p["judgments"].glob("*.jsonl"))
                for r in labelling.read_jsonl(f)
            ]
        )
    except runner.EngineError as exc:
        print(f"evals label merge: {exc}", file=sys.stderr)
        return 2
    if ignored:
        print(
            f"evals label merge: ignored {ignored} judgments without a model, "
            "from another prompt version or from an older model",
            file=sys.stderr,
        )
    previous = {r["item"]: r["label"] for r in labelling.read_jsonl(p["labels"])}
    merged = labelling.merge(
        labelling.read_jsonl(p["items"]), judgments, labelling.read_jsonl(p["human"])
    )
    p["spot"].parent.mkdir(parents=True, exist_ok=True)
    p["spot"].write_text(
        json.dumps(merged["spot_check"], indent=1) + "\n", encoding="utf-8"
    )
    if merged["spot_pending"]:
        print(
            f"evals label merge: {len(merged['spot_pending'])} of "
            f"{len(merged['spot_check'])} spot-check items have no human verdict; "
            f"record them with `label resolve` (see {p['spot']}), then merge again",
            file=sys.stderr,
        )
        return 2
    labelling.write_jsonl(p["labels"], merged["labels"])
    labelling.write_jsonl(p["disputes"], merged["disputes"])
    labelling.append_jsonl(
        p["revisions"],
        labelling.revisions_for_merge(
            previous,
            merged["labels"],
            args.date or labelling.today(),
            labelling.read_jsonl(p["human"]),
        ),
    )
    p["stats"].write_text(
        json.dumps(merged["stats"], indent=1, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(
        json.dumps(
            {k: v for k, v in merged["stats"].items() if k != "positives_by_rule"}
        )
    )
    return 0


def cmd_label_resolve(args: argparse.Namespace) -> int:
    p = _paths(args)
    if args.item not in {i["id"] for i in labelling.read_jsonl(p["items"])}:
        print(f"evals label resolve: unknown item {args.item}", file=sys.stderr)
        return 2
    try:
        row = labelling.record_human(
            item=args.item,
            label=args.verdict,
            reviewer=args.reviewer,
            reason=args.reason,
            date=args.date or labelling.today(),
        )
    except ValueError as exc:
        print(f"evals label resolve: {exc}", file=sys.stderr)
        return 2
    labelling.append_jsonl(p["human"], [row])
    return 0


def cmd_label_worksheet(args: argparse.Namespace) -> int:
    p = _paths(args)
    try:
        judgments, _ = labelling.usable_judgments(
            [
                r
                for f in sorted(p["judgments"].glob("*.jsonl"))
                for r in labelling.read_jsonl(f)
            ]
        )
        docs = {d["id"]: d for d in lib.load_manifest()["documents"]}
        cache = _cache(args)
        families, rows = labelling.worksheet_rows(
            labelling.read_jsonl(p["items"]),
            judgments,
            labelling.read_jsonl(p["human"]),
            doc_text=lambda doc: runner.text_path(docs[doc], cache).read_text("utf-8"),
            notes=labelling.rule_notes(Path(args.rules_dir)),
        )
        labelling.write_worksheet(Path(args.out), families, rows)
    except (runner.EngineError, OSError) as exc:
        print(f"evals label worksheet: {exc}", file=sys.stderr)
        return 2
    print(f"{len(rows)} rows written to {args.out}")
    return 0


def cmd_label_import_worksheet(args: argparse.Namespace) -> int:
    p = _paths(args)
    try:
        rows = labelling.worksheet_verdicts(
            Path(args.csv),
            {i["id"] for i in labelling.read_jsonl(p["items"])},
            labelling.read_jsonl(p["human"]),
            reviewer=args.reviewer,
            date=args.date or labelling.today(),
        )
        labelling.append_jsonl(p["human"], rows)
    except (ValueError, OSError) as exc:
        print(f"evals label import-worksheet: {exc}", file=sys.stderr)
        return 2
    print(f"{len(rows)} human verdicts recorded")
    return 0


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
    r.add_argument(
        "--skip-format",
        action="append",
        help="skip a document format, for a run that must not depend on installed converters",
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

    lab = sub.add_parser("label", help="build and maintain adjudicated labels")
    lab.add_argument(
        "--dir", default=str(labelling.LABELS_DIR), help="labels directory"
    )
    lsub = lab.add_subparsers(dest="label_command", required=True)

    ls = lsub.add_parser("sample", help="pick findings to label")
    ls.add_argument("results")
    ls.add_argument("--per-rule", type=int, default=40)
    ls.add_argument(
        "--dry-run",
        action="store_true",
        help="print the new item count per rule and in total; write nothing",
    )
    ls.set_defaults(func=cmd_label_sample)

    lj = lsub.add_parser("judge", help="ask one model family about every pending item")
    lj.add_argument("--family", required=True)
    lj.add_argument("--model", required=True)
    lj.add_argument("--provider-cmd", required=True, help="reads a prompt on stdin")
    lj.add_argument(
        "--max-invocations",
        type=int,
        required=True,
        help="hard cap on provider calls, counted in the family ledger across resumes",
    )
    lj.add_argument("--rules-dir", default=str(DEFAULT_RULES_DIR))
    lj.add_argument("--cache")
    lj.set_defaults(func=cmd_label_judge)

    lf = lsub.add_parser(
        "mark-failed", help="record failed judge attempts that no judgment row holds"
    )
    lf.add_argument("--family", required=True)
    lf.add_argument("--model", required=True)
    lf.add_argument("--attempts", required=True, type=int)
    lf.add_argument("item")
    lf.add_argument("order")
    lf.set_defaults(func=cmd_label_mark_failed)

    lm = lsub.add_parser("merge", help="combine judgments into labels and disputes")
    lm.add_argument("--date", default=None)
    lm.set_defaults(func=cmd_label_merge)

    lr = lsub.add_parser("resolve", help="record a human verdict")
    lr.add_argument("item")
    lr.add_argument("verdict", choices=("tp", "fp"))
    lr.add_argument("--reviewer", required=True)
    lr.add_argument("--reason", required=True)
    lr.add_argument("--date", default=None)
    lr.set_defaults(func=cmd_label_resolve)

    lw = lsub.add_parser(
        "worksheet", help="write the disputes and spot-check items as one CSV"
    )
    lw.add_argument("--out", required=True)
    lw.add_argument("--rules-dir", default=str(DEFAULT_RULES_DIR))
    lw.add_argument("--cache")
    lw.set_defaults(func=cmd_label_worksheet)

    li = lsub.add_parser(
        "import-worksheet", help="record the tp and fp rows of a filled worksheet"
    )
    li.add_argument("csv")
    li.add_argument("--reviewer", required=True)
    li.add_argument("--date", default=None)
    li.set_defaults(func=cmd_label_import_worksheet)

    b = sub.add_parser("baseline", help="write a ratchet baseline from stored results")
    b.add_argument("results")
    b.add_argument("--labels")
    b.add_argument("--out", required=True)
    b.set_defaults(func=cmd_baseline)

    g = sub.add_parser("gate", help="compare stored results with a baseline")
    g.add_argument("results")
    g.add_argument("--baseline", required=True)
    g.add_argument("--labels")
    g.add_argument(
        "--mode",
        choices=("report", "enforce"),
        help="default: SLOPVAC_EVALS_GATE, else report",
    )
    g.add_argument("--out", help="write the markdown summary here")
    g.set_defaults(func=cmd_gate)
    mo = sub.add_parser("models", help="compare models per review type")
    msub = mo.add_subparsers(dest="models_command", required=True)
    for name in ("plan", "run"):
        sp = msub.add_parser(name, help=f"{name} the model runs")
        sp.add_argument(
            "--models", required=True, help="JSON file naming a command per model"
        )
        sp.add_argument("--repeats", type=int, default=models.MIN_REPEATS)
        sp.add_argument("--per-type", type=int, default=12)
        sp.add_argument("--cache")
        sp.add_argument(
            "--only-model",
            help="run only the jobs this model adds to an earlier run",
        )
        sp.add_argument(
            "--candidates-from",
            help="journal of the earlier run, read-only; supplies the other "
            "models' rewrites for --only-model",
        )
        sp.add_argument("--rules-dir", default=str(DEFAULT_RULES_DIR))
        sp.set_defaults(func=cmd_models)
    run_parser = msub.choices["run"]
    run_parser.add_argument("--engine", default="python")
    run_parser.add_argument("--confirm-calls", type=int, required=True)
    run_parser.add_argument("--workers", type=int, default=4)
    run_parser.add_argument(
        "--max-invocations",
        type=int,
        required=True,
        help="ceiling on model-command spawns, retries included, across resumes",
    )
    run_parser.add_argument("--out")
    sm = msub.add_parser("summarize", help="summarise a results file")
    sm.add_argument(
        "results", nargs="+", help="results files of one task set, joined in order"
    )
    sm.add_argument("--out")
    sm.set_defaults(func=cmd_models)
    rc = msub.add_parser(
        "recommend",
        help="write the reviewer rows that feed the agents crate and the docs",
    )
    rc.add_argument("results")
    rc.add_argument(
        "--publish",
        action="store_true",
        help="write the rows; without it the file is pending re-evaluation, with no rows",
    )
    rc.add_argument("--out")
    rc.set_defaults(func=cmd_models)
    cs = msub.add_parser(
        "cost",
        help="tokens and estimated cost per model from a results file or journal",
    )
    cs.add_argument("path", help="a results .json or a .journal.jsonl")
    cs.add_argument("--prices", default=str(models.PRICES_PATH))
    cs.add_argument("--out")
    cs.set_defaults(func=cmd_models)

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
