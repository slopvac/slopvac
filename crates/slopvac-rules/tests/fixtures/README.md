# Test fixtures

| Path | What it is |
| --- | --- |
| `core-v3/` | The shipped 2.x rules, converted to v3 category files. Written by `generate.py`. The core rules load from here in the tests. |
| `pattern-parity.json` | A frozen corpus and the spans Python's `regex` module finds in it for every lexical rule. Written by `generate.py`. |
| `divergences.md` | Where PCRE2 and Python differ, and why. |
| `generate.py` | Writes the two files above. `--check` fails when they are stale. |
| `stub-3.28/` | `STUB(slopvac-8l4.3.28)`: copies of `rule.schema.json`, `category.schema.json`, their conformance corpora and the 2.x converter from PR #281. Delete them when #281 lands and point the tests at `schemas/v3`. |

Regenerate from the repository root:

```sh
mise exec -- uv run --extra dev python crates/slopvac-rules/tests/fixtures/generate.py
```

A pytest test in `tests/contract` runs `--check`, so a rule change that moves a span fails
until you regenerate and commit.
