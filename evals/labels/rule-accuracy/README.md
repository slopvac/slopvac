# Rule accuracy labels

Hand judgments of rule findings on the committed human eval corpus. One file per rule, named `<category>.<rule>.jsonl`. These are single-annotator labels: one person judged each finding once, with no second annotator and no model judge.

## Format

Each line is one finding:

- `rule`, `document` (manifest id), `line`, `text`: the finding.
- `verdict`: `tp` when the matched text narrates a change to the documented artifact in a document body, `fp` otherwise.
- `silenced_by_change_note_gate`: true when the rule's `change-note` context gate removes the finding.
- `note`: the reason.

## Rubric

A finding is `tp` when the text tells the reader what the artifact was before, for example a rename or a refactor. It is `fp` when the word describes runtime state ("previously bound"), the reader's earlier step, a person's history, or a changelog page whose purpose is to narrate deltas.

## Sample

`docs-discipline.history-narration`: every finding (38) on the 219 Markdown documents of the committed corpus, from a run of the engine without the gate. The sample is exhaustive, so it has no seed. Reproduce it with `python -m evals run` and an engine built without the gate.

## ste-procedural.condition-after-command

Rows add `genre`, `contexts` (the context tags the engine reported on the finding) and `kept_after_change` (true when the finding still appears after the pattern was tightened and the rule skipped the `change-note` context). The change added no findings, so no row needed a new label.

- Sample: every finding of the rule, ungated, on the committed `human` and `stated-llm` corpus documents (diagram, JSON and INI formats excluded). That is 188 findings, so no sampling applies and the sample needs no seed. The engine was the release build of the Rust engine at the commit that added these files.
- Rubric: `tp` when the sentence is an instruction to the reader and its trailing `if`, `when`, `once`, `unless`, `after` or `before` clause is a condition or ordering of the action, so that moving the clause first reads naturally. `fp` when the sentence describes a change (a changelog or pull request line), describes how the software behaves rather than telling the reader what to do, starts with "Note that", uses the clause as an object or purpose ("select when you want"), or is not an instruction.
- Labels do not depend on the context tags, so the same file scores any gate.

| Slice | Findings | True | Precision |
| --- | --- | --- | --- |
| Before: ungated | 188 | 32 | 17.0% |
| Before: tagged `procedure` | 7 | 2 | 28.6% |
| Before: not tagged `change-note` | 76 | 22 | 28.9% |
| After: tightened pattern, skips `change-note` | 51 | 22 | 43.1% |
| After: tagged `procedure` | 6 | 2 | 33.3% |

Recall against the 32 labeled true positives is 22 of 32 (69%) after the change, and 10 true positives are lost. Gating on `only: [procedure]` instead would keep 2 of 32 (6%).
