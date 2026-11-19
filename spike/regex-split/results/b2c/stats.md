## Shape check by source
| Source | Patterns | flag (reject) | warn | ok | parse error |
|---|---|---|---|---|---|
| core rules (pattern) | 111 | 3 | 10 | 98 | 0 |
| core rules (substitution) | 18 | 0 | 0 | 18 | 0 |
| core rules (tokens) | 17 | 0 | 0 | 17 | 0 |
| engine exceptions/matching | 743 | 0 | 0 | 743 | 0 |
| other | 13 | 0 | 0 | 13 | 0 |
| plugin | 22 | 16 | 1 | 5 | 0 |
| slopvac-context | 17 | 0 | 2 | 15 | 0 |
| slopvac-text built-ins | 38 | 1 | 5 | 32 | 0 |

Flagged non-plugin patterns:
- flag ['POLY', 'NEST', 'POLY2'] `ai-tells-formatting.title-case-heading` 
- flag ['POLY', 'POLY2'] `ai-tells-structure.tricolon-abuse-core` 
- flag ['POLY'] `ste-procedural.condition-after-command` 
- flag ['POLY'] `crates/slopvac-text/src/tokens.rs:76` 

Plugin patterns:
- `(a+)+$` (exp): flag ['NEST']
- `(a|aa)*$` (exp): flag ['ALT-OVERLAP']
- `(\w|\d)+$` (exp): flag ['ALT-OVERLAP']
- `(a*)*\1` (exp): flag ['NEST']
- `(?=(a+))+b` (poly): flag ['NEST', 'LOOK-IN-REP']
- `((?=a)a+)+$` (exp): flag ['NEST']
- `(?<=a{0,200})(?<!b{1,200})x` (poly): warn ['LONG-LOOKBEHIND']
- `(?:(?<=a)a)+$` (poly): ok []
- `(?:a+\K)+b` (exp): flag ['NEST', 'K-IN-REP']
- `a*\K` (poly): ok []
- `(\w+\s?)+$` (exp): flag ['NEST']
- `(.*a){12}` (poly): flag ['NEST-COUNTED']
- `(x+x+)+y` (exp): flag ['NEST', 'POLY2']
- `([a-zA-Z]+)*$` (exp): flag ['NEST']
- `(\s*)*$` (exp): flag ['NEST']
- `(?:a|a)*$` (exp): flag ['ALT-OVERLAP']
- `\((?:[^()]|(?R))*\)` (depth): flag ['ALT-OVERLAP', 'RECURSION']
- `\w*\w*\w*!` (poly): flag ['POLY']
- `\s+\S+\s+$` (poly): ok []
- `(?i)(k|K|\x{212A})+$` (exp): flag ['ALT-OVERLAP']
- `(?>a+)+b` (safe): ok []
- `(a++)+b` (safe): ok []

## recheck
- ('safe', 'safe'): 879
- ('safe', 'linear'): 41
- ('vulnerable', 'polynomial'): 26
- ('unknown', None): 11

recheck vulnerable vs shape check (non-plugin):
- vulnerable 26, shape-flagged 4, both 0
- `crates/slopvac-engine/src/exceptions.rs:74` polynomial deg=2 attack_len=94870 shape=ok
- `crates/slopvac-text/src/measure.rs:45` polynomial deg=2 attack_len=109548 shape=ok
- `crates/slopvac-text/src/tokens.rs:55` polynomial deg=2 attack_len=94875 shape=ok
- `crates/slopvac-text/src/tokens.rs:74` polynomial deg=2 attack_len=77461 shape=ok
- `crates/slopvac-text/src/measure.rs:17` polynomial deg=2 attack_len=54773 shape=ok
- `ai-tells-structure.definitional-negation-pair` polynomial deg=2 attack_len=77480 shape=ok
- `crates/slopvac-text/src/measure.rs:26` polynomial deg=2 attack_len=54774 shape=ok
- `crates/slopvac-text/src/tokens.rs:43` polynomial deg=2 attack_len=77461 shape=warn
- `crates/slopvac-text/src/measure.rs:31` polynomial deg=2 attack_len=77461 shape=ok
- `ste-procedural.note-gives-instruction` polynomial deg=2 attack_len=54783 shape=warn
- `prose-craft.optional-plural` polynomial deg=2 attack_len=77466 shape=ok
- `prose-craft.command-prompt` polynomial deg=2 attack_len=134171 shape=ok
- `prose-inflation.nominalized-verb` polynomial deg=2 attack_len=54781 shape=warn
- `crates/slopvac-text/src/measure.rs:29` polynomial deg=2 attack_len=77463 shape=ok
- `prose-agency.narrator-distance` polynomial deg=2 attack_len=77461 shape=ok
- `ai-residue.chat-leakage` polynomial deg=2 attack_len=122477 shape=ok
- `ste-verbs.nominalized-action` polynomial deg=2 attack_len=54777 shape=warn
- `crates/slopvac-text/src/tokens.rs:49` polynomial deg=2 attack_len=77461 shape=ok
- `prose-craft.unclear-antecedent` polynomial deg=2 attack_len=77461 shape=ok
- `prose-craft.hyphens` polynomial deg=2 attack_len=77463 shape=ok
- `ste-safety.safety-block-does-not-start-with-command` polynomial deg=2 attack_len=54781 shape=warn
- `prose-craft.link-text` polynomial deg=2 attack_len=122476 shape=warn
- `ste-procedural.multiple-instructions-per-sentence` polynomial deg=2 attack_len=54777 shape=warn
- `ste-practices.possessive-form-unclear` polynomial deg=2 attack_len=77462 shape=ok
- `ai-tells-structure.rhetorical-question-transition` polynomial deg=3 attack_len=2081 shape=warn
- `prose-craft.dead-opener` polynomial deg=2 attack_len=77461 shape=ok

recheck unknown/error reasons: {'timeout: ': 4, 'invalid: parsing failure (at 94)': 1, 'invalid: parsing failure (at 174)': 1, 'invalid: parsing failure (at 599)': 1, 'invalid: parsing failure (at 206)': 1, 'invalid: parsing failure (at 453)': 1, 'invalid: parsing failure (at 2244)': 1, 'invalid: parsing failure (at 45)': 1}
