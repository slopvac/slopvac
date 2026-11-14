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

A rule needs 20 adjudicated true positives before the precision gate trusts it. `stats.json` lists the count per rule.

## Model evals

These evals compare models on the three review types: `mechanical`, `update` and `rewrite`. Their results feed the dated recommendation table in prime and the docs.

```sh
python -m evals models plan --models models.json            # count the calls and input size
python -m evals models run --models models.json --confirm-calls <N> --max-invocations <M>
python -m evals models summarize evals/results/models/<date>.json
```

`models.json` names each model with a `family`, a `tier` and a `command` that reads the prompt on stdin and writes the answer on stdout. The committed `evals/models.json` calls Bedrock models through `python -m evals.opencode_model`, which runs `opencode run` with a bare agent (no tools, one-line prompt) in an empty directory. The adapter passes opencode an allow-listed environment and its own XDG directories, so your global opencode config and plugins stay out. Credentials come from the AWS profile files under `HOME`; set `AWS_PROFILE` and `AWS_REGION` (or pass `--aws-profile` and `--aws-region`). On a timeout the harness kills the whole process group. Nothing in the harness calls a provider itself. `plan` prints the number of calls and a rough input size. `run` refuses to start until `--confirm-calls` equals that number, so the person who pays approves it first. Ask the lead before you run a paid provider.

`--max-invocations` is a hard ceiling on spawned model commands, retries included. The harness writes and syncs a reservation to `<out>.ledger.jsonl` before it starts each process. A crash cannot hide a spawn. A lock on the ledger stops a second run from starting on the same ceiling. On resume, every earlier reservation counts as spent. Each finished call goes to a journal next to the output (`<out>.journal.jsonl`) as it completes. Rerun the same command to resume: the journal supplies finished jobs by a stable id, failed jobs are asked again, and attempts already spent count against the ceiling. `--workers` sets the thread count.

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
