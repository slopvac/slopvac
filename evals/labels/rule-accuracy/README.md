# Rule accuracy labels

Hand judgements of rule findings on the eval corpus, kept for rule accuracy changes. They are single-annotator labels and are not the corpus labels in `evals/labels/`. The evals work owns those.

Each line has the keys that `python -m evals score --labels <file>` reads (`doc`, `rule`, `line`, `label`), plus `annotator` and `reason`. `label` is `tp` or `fp`.

## negative-guarantee-core

The sample is every finding of the rule on the corpus run, because the run produced only 6 (3 distinct sentences), fewer than the 50 a seeded sample would need. No random seed applies. The results came from the Rust binary before and after the `requirement` and `enforced-invariant` context gate; both runs report the same 6 findings, so one file serves both.

Rubric. Label `tp` when the sentence reassures the reader that a component, tool, agent or role never takes an action with a side effect or authority, and nothing in the passage stops it. Label `fp` when the sentence states what a configured rule instructs, gives advice or a directive to the reader, describes a class of input, a language or data invariant, or has no side effect at all.

Check the file:

```sh
python -m evals score results.json --labels evals/labels/rule-accuracy/negative-guarantee-core.jsonl
```
