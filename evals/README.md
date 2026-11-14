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

The repository commits each document with a permissive licence and at most 32 KiB under `corpus/`. `NOTICE` credits each repository. Other documents, such as GPL and AGPL text, stay out of the repository and are downloaded to a cache.

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
