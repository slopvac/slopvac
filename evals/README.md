# Eval corpus

This directory holds the Slopvac eval corpus: a manifest, a fetch script, generators, and the permissively licensed documents.

## Contents

`manifest.json` has one record per document. Each record lists the `id`, `genre`, `origin`, `format`, `license` (SPDX), `date`, `source`, `sha256`, and whether the text is `committed`.

Origin classes:

- `human`: public text from before November 2022. The source pins a commit, or a pull request number.
- `stated-llm`: answers that the publisher labels with the producing model. The source names the model.
- `seeded-defect`: a human document with rule-flagged sentences inserted. The source lists each rule and line.
- `clean-control`: the same document with the clean counterpart of each inserted sentence.
- `rewrite-pair`: a human document with planted fact edits (dropped sentence, changed number, flipped modal, swapped identifier). The source lists each edit.
- `model-twin`: slots for text that a model writes from a human outline. All slots are `pending`.

Human genres: product, reference, tutorials, changelogs, readmes, decision records (`adrs`), prs, commits, ui-strings, code-comments, diagrams. Stated-LLM text uses the genre `assistant-answers`.

The repository commits each document with a permissive licence and at most 32 KiB under `corpus/` with the suffix `.corpus`, so prose gates skip them. `NOTICE` credits each repository. Other documents, such as GPL and AGPL text, stay out of the repository and are downloaded to a cache.

## Fetch

```sh
python -m evals.fetch                          # download what is not committed
python -m evals.fetch --all                    # also re-download committed documents
python -m evals.fetch --only genre=diagrams    # filter by field (glob), repeatable
```

The default cache is `evals/.cache`. Set `SLOPVAC_EVALS_CACHE` or pass `--cache` to move it. Set `GITHUB_TOKEN` to avoid the anonymous API rate limit.

The script checks every committed file against the manifest. It discards a download whose SHA-256 differs and exits 1. Only the standard library is required.

## Regenerate

Maintainers run these steps in order. The first needs the network and a GitHub token.

```sh
python -m evals.curate                         # sources.toml -> manifest.json and corpus/
uv run python -m evals.build_defect_bank       # rule examples -> defect-bank.json
python -m evals.build_derived --materialise    # defects, controls, rewrite pairs, twin slots
python -m evals.notice                         # NOTICE
```

`curate` pins each repository to its last commit before 2022-11-01 and seeds each choice from the source. `build_derived` is a pure function of the committed documents, the defect bank, and a fixed seed. The test suite regenerates it and compares hashes.

## Model twins

Three steps make a twin. No step calls a model on its own:

1. `python -m evals.twins outline` derives an outline from the human parent. The outline has headings, word budgets, code spans, and numbers, with no human sentence. The manifest pins the outline hash.
2. `python -m evals.twins generate --family anthropic --provider-cmd '<cmd>'` sends the prompt and outline to a command that reads stdin and writes the document to stdout.
3. `python -m evals.twins record --model <name>` hashes the results into the manifest and marks the slots `generated`.

Twins stay in the cache, because model output licences differ. Three families have slots: `anthropic`, `openai`, and `google`.

## Tests

```sh
uv run pytest tests/test_evals_manifest.py
```

The tests need no network. They validate the schema, committed hashes, attribution, and the generator output.

## Run the evals

The harness lints the corpus with an engine, stores the findings, and scores them. An engine is any command that accepts `lint <paths> --format json`. `python` selects the Python reference. After cutover, pass the Rust binary.

```sh
just eval-run python evals/.cache/results/a.json        # tier 1: run an engine
just eval-score evals/.cache/results/a.json holdout     # tier 2: offline re-score
just eval-ab a.json b.json                              # paired A/B between two runs
```

The runner stages each document in a temporary directory with the right extension (`.md`, `.rst`, `.txt`, or a source suffix linted with `--mode code-comments`) and runs the engine from there. Committed `.corpus` files and the repository's own config therefore never reach the engine. A format the engine cannot read, such as diagrams in the Python reference, goes to `skipped` in the results with a reason. A run that exits 2 but prints a report keeps its findings, and the results count the unchecked rules per document.

The runner reads committed documents first, then derived documents in `cache/docs`, then model twins in `cache/twins`. Run `python -m evals.fetch` and `python -m evals.build_derived --materialise` to fill the cache. Model twins with status `pending` are skipped.

### Results

A results file holds the engine name, command and version, the manifest hash, and the findings per document (rule, line, severity). The offline tier needs only that file and the manifest, so CI can re-score stored outputs without a Vale install or an engine build.

### Scorecard

`score` reports, for each rule and for a split (`all`, `tuning` or `holdout`):

- `recall`: seeded defects and adjudicated true positives where the rule fires within one line. `seeded_recall` and `adjudicated_recall` show the two parts. An adjudicated positive that the engine does not report counts as a miss. Pass `--labels evals/labels/labels.jsonl` to include them.
- `precision`: adjudicated findings labelled true over all labelled findings, with `--labels`.
- `control_fpr`: clean-control sentences where the rule fires within one line.
- `human_per_100_words`: findings on pre-2022 human text, a floor on false positives.
- `llm_per_100_words`: findings on `stated-llm` and `model-twin` text, the rate at which the rule fires on LLM writing. `llm_findings` is the count. A twin counts once it has been generated and run.

Each rate carries a Wilson interval and a bootstrap interval that resamples whole documents. Findings in one document are correlated, so the document is the unit of resampling.

`score` and `ab` refuse results recorded against a different manifest (exit 2), and `ab` refuses arms that cover different documents.

### Splits

A document lands in `holdout` or `tuning` by a hash of its root parent, about 20% holdout. Seeded documents, controls, rewrite pairs and twins follow their parent, so no held-out text reaches rule tuning.

### A/B

`ab` resamples documents once and scores both arms on each draw. The interval is for B minus A. It lists a rule as a regression when the interval for recall falls below zero, or the interval for control false positives or human findings rises above zero.

## Labels

Precision needs a trustworthy answer to "was this finding right?". The labelling workflow produces those answers without trusting one model.

```sh
python -m evals label sample results.json                 # pick findings, balanced across documents
python -m evals label judge --family anthropic --model <name> --provider-cmd '<cmd>'
python -m evals label judge --family openai --model <name> --provider-cmd '<cmd>'
python -m evals label merge                               # labels, disputes, stats
python -m evals label resolve <item> tp --reviewer <you> --reason '<why>'
python -m evals score results.json --labels evals/labels/labels.jsonl
```

Files live in `evals/labels/`:

| File | Content |
| --- | --- |
| `items.jsonl` | Sampled findings, frozen once chosen. |
| `judgments/<family>.jsonl` | One verdict per item and option order. |
| `human.jsonl` | Human verdicts with reviewer, reason and date. Append only; the last row wins. |
| `labels.jsonl` | The current label per item and where it came from. |
| `disputes.jsonl` | Items that need a human. |
| `spot-check.json` | Automatic labels that a human should re-check (10%, seeded). |
| `revisions.jsonl` | Every label change: previous label, new label, source, reviewer and reason. |
| `stats.json` | Agreement (Cohen's kappa), unstable judgments, positives per rule. |

How a label forms:

1. Sampling skips seeded documents, whose inserted sentences are positive by construction. It takes up to 40 findings per rule, one document at a time, so no document dominates.
2. Each judge sees every item twice, with the "rule is right" and "rule is wrong" options in swapped positions. A family whose two answers differ, or are both "unsure", is `unstable`.
3. The item gets an automatic label only when both families are stable and agree. Everything else lands in `disputes.jsonl`.
4. A human verdict always wins. Spot-check a sample of automatic labels, and resolve every dispute.

A judge command reads the prompt on stdin and writes the answer on stdout. Ask the lead before you run a paid provider.

A rule needs 20 adjudicated true positives before the precision gate trusts it. The gate lists the precision of a rule below that count as uncertain and never fails it. `stats.json` lists the count per rule.

## Ratchet

The ratchet stops a rule change from making a rule worse. `evals/baselines/python.json` holds the counts per rule and split: recall, precision, false positives on clean controls, and findings on human text. The baseline records no local paths.

```sh
just eval-ratchet            # run the reference engine, compare with the baseline, report only
just eval-ratchet enforce    # exit 1 on a regression
just eval-baseline           # rebuild the baseline after an intended change
```

A rule regresses when its current estimate is worse than the baseline estimate by more than noise:

- For recall and precision, the upper end of the current interval is below the baseline rate.
- For the control false-positive rate, the lower end is above it.
- For findings on human text, the current count exceeds the baseline count, scaled by word count, by more than two Poisson standard deviations plus one.

The interval is the wider of the Wilson interval and the document-cluster bootstrap interval. A rate on fewer than 10 cases is listed and never gated. A rule that disappears from the scorecard is a regression.

Tuning and holdout are compared separately against their own baselines. Tune rules against `tuning`. Use `holdout` to check that a gain generalises.

The ratchet starts in report mode. The workflow `evals-ratchet.yml` runs from `test.yml` on rule, engine and eval changes and writes the table to the job summary. Set the repository variable `SLOPVAC_EVALS_GATE` to `enforce`, or pass `--mode enforce`, to fail the run. The run is deterministic for a given engine and corpus, and a baseline from another manifest is rejected with exit 2. After a corpus or label change, rebuild the baseline in the same pull request and say why.

## LLM-written diagram set

`corpus/diagrams` holds human diagrams, so it only bounds false positives of the diagram rules. `diagrams-llm/` holds 53 diagrams that an LLM wrote, with the prose around them, for recall and precision. It has 28 Mermaid and 14 PlantUML diagrams in Markdown, and 11 draw.io files.

| File | Content |
| --- | --- |
| `diagrams-llm/manifest.json` | Provenance (author model, date, instruction, limits), the sha256 of each sample and its format. |
| `diagrams-llm/labels.json` | The hand labels. A label names a rule and text that the finding should contain. A sample with no label is a clean negative. |
| `diagrams-llm/samples/*.corpus` | The samples. The suffix keeps prose gates off them, and the runner stages them as `.md` or `.drawio`. |

The author model wrote the samples in one session and labelled each one from the rule definitions in `crates/slopvac-diagrams/pack/diagram.json`, never from rule output. No model API was called. The author knew the thresholds, so recall is probably higher than on unprompted model output. The negatives are of three kinds: clean diagrams, boundary cases such as six words against seven, and prose that names every label but adds limits and reasons. draw.io files have no prose, so the two prose rules have no draw.io positives.

```sh
just eval-diagrams                    # build the diagram_eval example of slopvac-engine, run it, score it
just eval-diagrams --details          # list each miss and false positive
just eval-diagrams --findings f.json  # score stored engine output without an engine
```

The runner reads the findings of `crates/slopvac-engine/examples/diagram_eval.rs`, which lints each file with the diagram pack. It reports recall (labels found over labels) and precision (correct findings over findings) per rule, each with a Wilson interval. Within one sample and rule, labels and findings pair one to one (a maximum matching), so one finding never credits two labels and repeated findings for one label count as false positives. `tests/fixtures/diagram_eval_output.json` records the adapter output for one sample per format; set `SLOPVAC_EVALS_LIVE_ENGINE=1` to check it against a live build.

### Results

Measured on 2026-11-16 after the engine counted every `<br>` form as a line break:

| Rule | Labels | Recall | Findings | Precision |
| --- | --- | --- | --- | --- |
| `diagram.label-run-on` | 10 | 1.00 (0.72-1.00) | 10 | 1.00 (0.72-1.00) |
| `diagram.label-title-plus-gloss` | 14 | 0.71 (0.45-0.88) | 10 | 1.00 (0.72-1.00) |
| `diagram.note-long` | 5 | 1.00 (0.57-1.00) | 5 | 1.00 (0.57-1.00) |
| `diagram.meta-reference` | 6 | 1.00 (0.61-1.00) | 6 | 1.00 (0.61-1.00) |
| `diagram.prose-restates-labels` | 4 | 1.00 (0.51-1.00) | 6 | 0.67 (0.30-0.90) |

The misses and false positives show two things:

- Title-cased glosses after a line break cost gloss recall. The exemption for a list of names, such as `Istio Sidecar<br/>Envoy Pilot Proxy`, also skips `Ingest Service<br/>Validates Incoming Events`. Four labels are missed for this reason (two in a Mermaid sample, one each in a PlantUML and a draw.io sample), and without them recall is 10 of 10. The exemption still keeps the list of names quiet in two negative samples. A title-cased gloss after a colon is found.
- `diagram.prose-restates-labels` fires on both hard negatives, where the prose names every label but adds a limit, a reason and a failure mode. It checks token overlap only.

Each count is small, so every interval is wide. Treat the table as a sanity check on the severities, not a ranking.

## Model evals

These evals compare models on the three review types: `mechanical`, `update` and `rewrite`. Their results feed the dated recommendation table in prime and the docs. Reviewer tasks cover rewrites only for now; mechanical and update reviewer tasks are tracked in slopvac-8l4.15.8.

Model runs need Python 3.13 or later on macOS. The harness waits for a model command without reaping it, so it can still kill the command's process group safely, and macOS lacks `os.waitid` before 3.13. On an older Python the run stops with a message before it reserves or spawns anything.

```sh
python -m evals models plan --models models.json            # count the calls and input size
python -m evals models run --models models.json --confirm-calls <N> --max-invocations <M>
python -m evals models summarize evals/results/models/<date>.json
python -m evals models cost evals/results/models/<date>.json --prices evals/prices.json
```

`models.json` names each model with a `family`, a `tier` and a `command` that reads the prompt on stdin and writes the answer on stdout. The committed `evals/models.json` calls Bedrock models through `python -m evals.opencode_model`, which runs `opencode run` with a bare agent (no tools, one-line prompt) in an empty directory. The adapter resolves the AWS profile with `aws configure export-credentials` (set `AWS_PROFILE` and `AWS_REGION`, or pass `--aws-profile` and `--aws-region`), then starts opencode with an empty `HOME`, its own XDG directories and an allow-listed environment. The adapter re-exports credentials on every attempt. If they have under 10 minutes left, it waits up to 330 seconds for the CLI to hand out fresh ones, inside the same attempt, and then gives up. Your opencode config, plugins, rules and auth never load, and opencode finds no AWS profile files. On a timeout the harness kills the whole process group, including opencode and its children. Nothing in the harness calls a provider itself. `plan` prints the number of calls and a rough input size. `run` refuses to start until `--confirm-calls` equals that number, so the person who pays approves it first. A journal without a ledger is refused, because its spent calls are unknown: start with a fresh `--out` path. Ask the lead before you run a paid provider.

`--max-invocations` is a hard ceiling on spawned model commands, retries included. The harness writes and syncs a reservation to `<out>.ledger.jsonl` before it starts each process. A crash cannot hide a spawn. A lock on the ledger stops a second run from starting on the same ceiling. On resume, every earlier reservation counts as spent. Each finished call goes to a journal next to the output (`<out>.journal.jsonl`) as it completes. The journal holds the full model responses, so git ignores it; the results file stores only output hashes. If a lint leaves rules unchecked (for example Vale is missing), the author run is unscored: its row carries `score: null` and an `unscored` reason, and the summary leaves it out. Ctrl-C cancels pending calls and kills running model process groups. Rerun the same command to resume: the journal supplies finished jobs by a stable id, failed jobs are asked again, and attempts already spent count against the ceiling. `--workers` sets the thread count.

Token usage travels on a side channel, so it never mixes with the answer. The adapter writes the answer alone to stdout. After it, the adapter writes one last line to stderr: `slopvac-usage`, a space and a JSON object with `input`, `output`, `reasoning`, `cache_read` and `cache_write`. The object holds only the fields opencode reported. The adapter reads them from the `part.tokens` field of each `step_finish` event in `opencode run --format json` and sums the steps. The harness takes the last such line and drops it from error text. A custom model command opts in by printing the same line. Each journal entry and each results row carries a `usage` object, or `null` when the command reported none. Each journal entry also names its `model`. Every ledger `reserve` and `finish` entry carries a `model` field too. Ledgers and journals from older runs lack these fields, and they still load and resume.

`models cost` takes a results file or a `.journal.jsonl` and prints tokens and an estimated cost per model. `evals/prices.json` holds the list prices in USD per million input and output tokens, with their source, and optional `cache_read` and `cache_write` prices per model. The estimate bills input tokens at the input price, output plus reasoning tokens at the output price, and cache tokens at their cache price. A model with cache usage but no price for that kind of cache token is marked incomplete, so its cost is a lower bound. The shipped list has no cache prices yet, because the Bedrock pricing page does not show them for these models in a form we could verify. Add them when you can check them against the source. The report is marked `"complete": false` when any invocation has no usage, a model has no price, or cache tokens have no cache price. Cost counts executions, not score rows. Each journal line is one execution. A Ctrl-C keeps the ledger reservations but writes no journal line, so the cost report adds every reservation that no journal line covers as an invocation with no usage. That makes the report incomplete. `models cost` on a journal reads the sibling `.ledger.jsonl` for this, and a results file carries the same entries in `executions`. A resumed or superseded job has several lines. Examples are a failed job that ran again, a call cut off by the ceiling between retries, and a review of a candidate that the author later replaced. Two tasks that share a prompt share one execution, so it backs two rows. The results file therefore embeds an `executions` section copied from the journal, and `models cost` reads that section, so the results file and the journal give the same numbers. A results file written before that section existed falls back to one execution per row. That undercounts superseded work, so use the journal for such a run.

Tasks come from the corpus:

| Task | Role | Source | Success |
| --- | --- | --- | --- |
| mechanical | author | A seeded sentence and its rule | The rule does not fire on the output and no fact is lost. |
| update | author | A planted number change | The new number appears, the old one is gone, nothing else moves. |
| rewrite | author | A prose paragraph | No fact is lost and findings per word do not rise. |
| rewrite | reviewer, planted | A rewrite pair with planted damage | The reviewer quotes every planted edit with at most one false alarm. |
| rewrite | reviewer, authored | Another model's rewrite | The reviewer names every fact the lexical alignment finds lost. |

The reviewer is always a different model from the author. Every task repeats at least three times. The summary averages repeats within a task, then resamples tasks for a 95% interval, and ranks models per review type and role.

Fact preservation is lexical: `evals/facts.py` compares numbers, code spans, identifiers and polarity words (must, not, never, only) between the source and the output.

The results file records the date, the prompt version, the task-set hash, the models and one row per run with an output hash. The model text itself is not stored.
