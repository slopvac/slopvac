# Rules and categories

This spec defines the v3 rule and category files.
The schemas are [`rule.schema.json`](../../schemas/v3/rule.schema.json) and
[`category.schema.json`](../../schemas/v3/category.schema.json).
Field descriptions live in the schemas. This page states the semantics the schemas cannot.

## Files and ids

A category file holds one category and its rules.
A plugin ships many category files.
A rule can also stand alone in its own file, which then carries `schema_version`.

A rule has a local `id` ([`LocalRuleId`](../../schemas/v3/rule.schema.json#/$defs/LocalRuleId)).
Its qualified form is [`RuleId`](../../schemas/v3/common.schema.json#/$defs/RuleId):
`<plugin>:<category>.<rule>` for a rule from a category file, for example `core:prose-craft.acronym-periods`.
The schema accepts any `<plugin>:<id>`, because a plugin can report a rule with a bare id such as `core:passive-voice`.
The plugin id comes from the plugin manifest.
Built-in rules live in the reserved plugin `core`.
The plugin and category come from the manifest and the file, so a rule file does not restate them.

Every output carries the qualified id.
Configuration accepts a short name where it is unambiguous.
An ambiguous short name is a configuration error that lists the candidates.

[`RuleProvenance`](../../schemas/v3/common.schema.json#/$defs/RuleProvenance) records which plugin supplied a rule at run time.
It appears in reports and in `slopvac rules`.
It is not the same as `citation`, which is the rule author's note on where the idea comes from.
2.x called the author's note `provenance`; the converter renames it.

## Severity

[`Severity`](../../schemas/v3/common.schema.json#/$defs/Severity) has three levels: `error`, `warning` and `suggestion`.
The three levels are the only ones in 3.0.
Slopvac has no `off` level: disabling a rule is configuration and removes the rule from the run.
A rule states its shipped level. A category cap or a profile shift can lower it.
A rule cannot raise its own level above what it ships.

A judgement rule produces no finding.
Its severity is `suggestion` when present, and it declares no `exceptions` and no `cases`.
The schema enforces all three.

## The `match` block

`match.kind` selects the one allowed shape. Each kind forbids the payload keys of the others.

- `tokens`: literal phrases matched at word boundaries.
- `pattern`: a PCRE2 regular expression.
- `substitution`: a map from a PCRE2 pattern to its replacement.
- `metric`: a counted measurement compared with a threshold.
- `structure`: a block-level check selected by name.
- `capitalization`: a capitalization style, with words exempt from it.
- `vocabulary`: a controlled-vocabulary lookup selected by name.
- `judgement`: a question for a reviewer.

`ignore_case` and `match_all_caps` belong to `tokens`, `pattern` and `substitution`.
`ignore_case` defaults to true.
`match_all_caps` defaults to false: a match written entirely in capitals is skipped unless the rule is about the capitals.

A `metric` fires when the measured value stands in the stated `comparison` to `threshold`.
The comparison defaults to `gt`.
The 2.x engine measures these names: `sentence_words`, `clause_boundaries`, `paragraph_words`,
`enumerating_paragraph_words`, `lead_in_words`, `paragraph_sentences`, `syllables_per_word`,
`passive_ratio`, `hedge_per_100_words`, `abstraction_density`, `concrete_referents_per_paragraph`,
`paragraph_words_stdev`, `adjectives_per_noun`, `consecutive_bold_colon_bullets`,
`multiword_noun_words`, `coordinated_items`, `bold_spans_per_1000_words` and `dash_per_1000_words`.
A plugin can add a metric. A rule that names a metric nothing measures is reported as unchecked, never as clean.

`structure` and `vocabulary` rules carry no data of their own.
In 2.x the engine picks the check from the rule id.
In 3.0 the rule names it in `check`.

`capitalization` is new in 3.0.
2.x has no producer for it: `title-case-heading` is a `pattern` rule.

## PCRE2 patterns

Patterns use PCRE2 10.43 or later.
The engine runs them with JIT where the platform has it, under a match limit.
The schema ([`Pcre2Pattern`](../../schemas/v3/rule.schema.json#/$defs/Pcre2Pattern)) enforces what a schema can.
The loader compiles every pattern and rejects the rest.

- No `\uXXXX` or `\UXXXXXXXX` escape. Rule data uses the literal character or the PCRE2 form `\x{XXXX}`, never `\u`. The schema rejects `\u` and `\U`.
  The converter does not rewrite 2.x data, because Python `regex` rejects `\x{...}`.
- A lookbehind needs a finite maximum length of at most 255 characters.
  Use `{n,m}` instead of `+`, `*` or `{n,}` inside a lookbehind.
  Top-level alternatives may differ in length.
- Named groups use `(?<name>...)` or `(?P<name>...)`.
- Case folding follows `ignore_case`. An inline `(?i)` or `(?i:...)` also works.

Substitution keys follow the same rules as patterns.

The `ALLOWED_WORDS` placeholder comes from the 2.x rule `title-case-heading`.
The 2.x loader replaced that text in the pattern with the words from the locale allow list.
In 3.0 a pattern declares each placeholder in `match.placeholders`.
The only source is `allowed-words`.
The loader replaces the placeholder with an alternation of the allowed words.
It keeps single ASCII alphabetic words, escapes them and sorts them without regard to case.
With no words the placeholder becomes `(?!)`, which never matches, so nothing is exempt.
Under 2.x the placeholder stayed in the pattern as literal text. The effect is the same.

## Scope, blocks and applicability

`scope` selects the text unit the rule sees: `prose`, `heading`, `sentence`, `body-sentence`,
`paragraph`, `document` or `raw`. The default is `prose`.
`prose` leaves out code fences, inline code, URLs and front matter.
`raw` reaches everything and is for formatting rules.
A `raw` rule matches one line at a time, without the line terminator. A raw pattern therefore cannot match across lines, and `$` (or `(?![\s\S])`) anchors at the end of the line.

A raw HTML block is checked as one plain paragraph. It has no containers and no element kinds, so the `heading`, `list`, `table` and `quote` exclusions do not reach inside it. Only its markup is excluded.

These parts of a Markdown or MDX file are markup, not prose:

- The `:::` directive markers.
- MDX `import` and `export` blocks.
- The `{#id}` attribute of a heading.
- An image-only paragraph, because the text of an image is not read.

## Reference behavior

Where this contract describes the procedural or descriptive `text_type`, the position of a metric finding, a match that spans several lines, or `match_all_caps`, it follows the native engine of 2.x, not the behavior of Vale.

A word, and the class of a character, follow the Python reference as `slopvac-text` implements it. The reference is `packages/slopvac-lint/src/slopvac/analyze.py`, and [`metrics.md`](../../packages/slopvac-lint/docs/metrics.md) states the word-counting phases.

Part-of-speech tags are English and use the Penn Treebank tag set. The tagger is a Rust port of the tagger of the `prose` library, version 3.3.1.

`body-sentence` is a sentence that is not in a heading or a table.
It came with the 2.x change that fixed `ste-procedural.condition-after-command`
([#268](https://github.com/slopvac/slopvac/pull/268)), which was not merged when this spec was written.
It equals `sentence` with `exclude_blocks: [heading, table]`, and the scope stays for that rule's sake.

`exclude_blocks` lists block kinds the rule must not look inside, on top of what the scope already leaves out.

`text_type` limits a rule to sentences of one STE text type. `any`, the default, does not limit it.

`contexts` gates the rule by context tag
([`ContextTag`](../../schemas/v3/common.schema.json#/$defs/ContextTag)).
`only` runs the rule on text that carries at least one listed tag.
`skip` silences it on text that carries any listed tag.
The tags are registered by taggers and are defined with the data plugins (slopvac-8l4.2.4).

`applies_to` narrows the rule by lint `modes` and by `genres`.
An absent key means no restriction.

## Exceptions

An exception is a named, closed reason that a finding may be suppressed.
A suppression annotation must cite one by name.
"Reads better" is never an exception.

With `detector`, the engine drops the finding when the named detector holds at the match.
The detector is a built-in or a context tag of the same name.
Without `detector`, the exception only legitimizes an annotation.
The 2.x engine resolves one detector itself: `quotation`.
The converter sets it for that name and leaves every other exception without a detector.

## Test cases

`cases` follow the style of `vale test`.
The runner lints `input` with only this rule enabled.

- `want` is the finding count, or an inclusive `{min, max}` range.
- `contains` lists strings that each appear in the matched text or message of some finding.
- `absent` lists strings that appear in no finding's matched text or message.

A case needs at least one of the three.

`examples` are before-and-after pairs for the rule's documentation page.
For a mechanical rule the runner also treats `bad` as a case with at least one finding
and `good` as a case with none.

The case runner exits 1 when any case or example of a mechanically checkable rule fails, whatever else it could not check. Otherwise it exits 2 when an active rule of a mechanically checkable kind was not checked, which it reports as the warning notice `cases-rule-unchecked` with the rule ids in `data.rules`. Otherwise it exits 0. Judgement rules are documentation only: they never run as cases and never change the exit code.

A judgement rule has examples and no cases.

## Category

A category is the unit users enable, disable and score by.
`weight` sets its share of the score.
`max_per_100_words` is a density budget keyed by profile name.
The profile mapping (slopvac-8l4.2.10) owns what those keys mean.
Rule ids are unique within a category; the loader checks that.

## Converting 2.x rules

`scripts/contract/convert_rules_v2_to_v3.py` converts a directory of 2.x rule files.

```sh
python scripts/contract/convert_rules_v2_to_v3.py OUT_DIR [--rules-dir DIR] [--tiers-out FILE]
```

It writes one `<category>.json` per category.
`tests/contract/test_rule_conversion.py` converts every shipped rule and validates the result. The converted core-v3 fixtures are committed under `crates/slopvac-rules/tests/fixtures/core-v3/`. The converter regenerates them, and `tests/contract/test_rules_fixtures.py` checks them for drift.

The mapping:

- `kind` and its payload fields move into `match`. The payload keys are `tokens`, `pattern`,
  `substitutions`, `metric`, `threshold`, `comparison` and `judgement_question`.
  `judgement_question` becomes `match.question`.
- `ignore_case` and `match_all_caps` move into `match` for the three lexical kinds.
- A `structure` or `vocabulary` rule gets `match.check` set to its rule id.
- `provenance` becomes `citation`.
- `exceptions` becomes a list of named objects.
- A null `good` in an example is dropped.
- A judgement rule gets severity `suggestion`, as the 2.x loader forced.
- A missing `severity` becomes `warning`, and a missing `scope` becomes `prose`.
- The converter fails, naming the rule id, on a `\u` or `\U` escape and on an unbounded
  lookbehind. It does not rewrite either. The shipped 2.x data has neither since PR #276.
- The `title-case-heading` pattern gets `match.placeholders` for `ALLOWED_WORDS`. No other rule is checked, because 2.x substitutes the placeholder for that rule alone.
- A null member of `provenance` is dropped from `citation`.
- A category id outside `[a-z][a-z0-9-]*` is rejected before any output path is built.

### Tier removal

3.0 removes `tiers`.
The converter drops the field from every instance.
With `--tiers-out` it writes the 2.x per-profile dispositions as one JSON file keyed by `<category>.<rule>`.
That file is the input for the profile mapping, which turns each disposition into a profile threshold or a severity shift.
Slopvac-8l4.2.10 owns that mapping and its spec.
This page does not define it.
