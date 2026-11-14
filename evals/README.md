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

The runner reads committed documents first and the cache second. Run `python -m evals.fetch` and `python -m evals.build_derived --materialise` to fill the cache. Model twins with status `pending` are skipped.

### Results

A results file holds the engine name, command and version, the manifest hash, and the findings per document (rule, line, severity). The offline tier needs only that file and the manifest, so CI can re-score stored outputs without a Vale install or an engine build.

### Scorecard

`score` reports, for each rule and for a split (`all`, `tuning` or `holdout`):

- `recall`: seeded defects where the rule fires within one line of the insertion.
- `control_fpr`: clean-control sentences where the rule fires within one line.
- `human_per_100_words`: findings on pre-2022 human text, a floor on false positives.

Each rate carries a Wilson interval and a bootstrap interval that resamples whole documents. Findings in one document are correlated, so the document is the unit of resampling.

### Splits

A document lands in `holdout` or `tuning` by a hash of its root parent, about 20% holdout. Seeded documents, controls, rewrite pairs and twins follow their parent, so no held-out text reaches rule tuning.

### A/B

`ab` resamples documents once and scores both arms on each draw. The interval is for B minus A. It lists a rule as a regression when the interval for recall falls below zero, or the interval for control false positives or human findings rises above zero.
