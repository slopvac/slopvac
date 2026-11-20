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
