# Split summary

Dumps: parity-golden-en-US.jsonl, parity-golden-en-GB.jsonl, parity-docs-en-US.jsonl, parity-docs-en-GB.jsonl, parity-k8s-en-US.jsonl, parity-k8s-en-GB.jsonl, parity-eval-en-US.jsonl, parity-eval-en-GB.jsonl, parity-golden-rst-en-US.jsonl, parity-golden-rst-en-GB.jsonl, parity-fuzz.jsonl, parity-fuzz-directed.jsonl

| Source | Patterns | compiles+identical | compiles+differs | no compile | exercised (calls>0) |
|---|---|---|---|---|---|
| core rules (pattern) | 111 | 80 | 0 | 31 | 80 |
| core rules (substitution) | 18 | 14 | 0 | 4 | 14 |
| core rules (tokens) | 17 | 17 | 0 | 0 | 17 |
| engine exceptions/matching | 743 | 726 | 0 | 17 | 726 |
| other | 13 | 12 | 1 | 0 | 13 |
| slopvac-context | 17 | 15 | 1 | 1 | 16 |
| slopvac-text built-ins | 38 | 17 | 0 | 21 | 17 |
| total | 957 | 881 | 2 | 74 | 883 |

No-compile reasons: lookaround 73, backreference 1

## Differs (first example each)
- [other] `/rustc/58cf8f9c35b355199727627d07ae34a4e2846d67/library/core/src/ops/function.rs:250` diffs=0 edge=0 fuzz=60 calls=130: `^(?:true|yes)$`
  - find_all: pcre=[0..3] rust=[] text[..300]="YES\n" len=4
- [slopvac-context] `crates/slopvac-context/src/builtin/sentence.rs:55` diffs=0 edge=0 fuzz=10 calls=345516: `\b(?:not|no|never|without|neither|nor)\s+(?:\w+\s+){0,3}(?:`[^`]+`|--?[a-z][\w-]+|(?:\.{0,2}/)[\w./-]+)`
  - find_all: pcre=[0..11] rust=[0..15] text[..300]="never\r\t..//ſ-0" len=15

## PCRE2-only (no compile in regex)
- [core rules (pattern)] `ai-tells-agentic.formulaic-universal-heading` lookaround: `^every\s+[\w-]+\s+(?:is|are|was|were|has|have|does|do|can|cannot|will|would|should|must|[a-z][\w-]*(?<!s)s)(?:\s+[\w-]+){2,8}$`
- [core rules (pattern)] `ai-tells-content-shape.fake-specificity` lookaround: `\b(?:over|more than|upwards of|nearly|almost)\s+\d+\+|\b\d+\+\s+(?:different\s+)?(?!(?:to|of|and|or|the|a|an|is|are|was|were|for|in|on|at|by)\b)\w{3,}|\ba wide `
- [core rules (pattern)] `ai-tells-formatting.italicised-copula` lookaround: `(?<![\w*])\*(?:is|are|was|were|not|the|do|does|did|can|will)\*(?![\w*])|(?<![\w_])_(?:is|are|was|were|not|the|do|does|did|can|will)_(?![\w_])`
- [core rules (pattern)] `ai-tells-formatting.title-case-heading` lookaround: `^(?=\S+[ \t]+(?:\S+[ \t]+)*?(?!(?i:(?!))[,:;.!?)]*(?:[ \t]|$))[A-Z][a-z]+[,:;.!?)]*(?:[ \t]|$))(?=(?:.*\b[A-Z][A-Za-z0-9]*(?:[-'’][A-Za-z0-9]+)*\b){2,})(?:(?:[A`
- [core rules (pattern)] `ai-tells-structure.absolute-assertion-core` lookaround: `(?i:\b(?:the only way to|the single most important|make no mistake|there is no substitute for|the one thing you need|without exception the|always the right|neve`
- [core rules (pattern)] `ai-tells-structure.contrastive-appositive` lookaround: `(?:^|[.!?][ \t]+)(?:the|a|an|this|that|these|those|our|your|my|his|her|its|their)\s+[\w-]+(?:\s+[\w-]+){0,3}\s+(?:is|are|was|were|does|do|did|has|have|had|can|c`
- [core rules (pattern)] `ai-tells-structure.negative-guarantee-core` lookaround: `(?:^|(?<=[.!?;:]\s)|(?<=[.!?]\s\s)|(?<=—)|(?<=—\s)|\b(?=(?:It|They|The|This|These|Those|That|Each|Every|Its|Their)\s))(?!(?:If|When|Whenever|Because|Although|Th`
- [core rules (pattern)] `ai-tells-structure.negative-inventory-core` lookaround: `(?:\b(?:does|did)\s+not\s+pin\b|(?<!^)(?<!\n)(?<!^[-*+>]\s)(?<![.!?:;,]\s)(?<![.!?:;,])(?<!please\s)(?<!then\s)(?<!and\s)(?<!also\s)(?<!always\s)(?<!but\s)\bdo\`
- [core rules (pattern)] `ai-tells-structure.tricolon-abuse-core` lookaround: `\b((?:fast|faster|quick|quicker|reliable|dependable|secure|safe|safer|robust|resilient|stable|scalable|seamless|smooth|streamlined|efficient|effective|clear|cle`
- [core rules (pattern)] `docs-discipline.history-internal-reference` lookaround: `\b(?:commit|cherry-picked from|reverted in|fixed in|introduced in)\s+[0-9a-f]{7,40}\b|\b(?:PRs?|MRs?|pull requests?)\s+#?\d+\b|\bissues?\s+#\d+\b|\b(?i:see|per|`
- [core rules (pattern)] `docs-discipline.status-language` lookaround: `\b(under construction|work[- ]in[- ]progress|WIP|coming soon|Draft(?!\s+(?-i:a|an|the|your)\b)|being (specified|designed|implemented|built)|not yet (implemented`
- [core rules (pattern)] `prose-agency.agentless-passive` lookaround: `\bit (?!is assumed\s+that\s+(?:the\s+)?(?:reader|readers|audience|you)\b)(?:is|was) (?:believed|thought|felt|assumed|expected|understood|considered|argued|repor`
- [core rules (pattern)] `prose-craft.dead-opener` lookaround: `(?m)(?:^|(?<=[.!?;]\s)|(?<=^[-*]\s)|(?<=^>\s))\s*[Tt]here\s+(?:is|are|was|were|will be|has been|have been)\b|(?:^|(?<=[.!?;]\s))\s*[Ii]t\s+(?:is|was)\s+(?:impor`
- [core rules (pattern)] `prose-craft.first-person-plural` lookaround: `\b(?:[Ww]e|[Ww]e'(?:ve|re|ll|d))\s+(?=\w)|\b[Oo]ur\s+(?=\w)|\b[Ll]et's\b`
- [core rules (pattern)] `prose-craft.future-tense` lookaround: `\b(?!(?:(?<=\bthat\s{1,3})|(?<=\b(?:which|where|when|what)(?:\s{1,3}[\w-]{1,40}){0,2}\s{1,3}))(?:will|shall)\s+(?:not\s+)?be\b)(?:will|shall)\s+(?:not\s+)?(?=[a`
- [core rules (pattern)] `prose-craft.ordinals` lookaround: `\b(?:firstly|secondly|thirdly|fourthly|fifthly|lastly)\b|(?<!\w)\d{1,2}(?:st|nd|rd|th)\b`
- [core rules (pattern)] `prose-craft.politeness` lookaround: `(?<![\w-])please\b|\bthank(?:s| you)\b|\bkindly\s+(?=\w)|\b(?:sorry|we(?:'re| are) sorry|apologies)\b|\bunfortunately\b|\bfeel free to\b`
- [core rules (pattern)] `prose-craft.spacing` lookaround: `(?<!\d)[a-z][.?!] {2,}[A-Z]|(?<!\d)[a-z][.?!][A-Z][a-z]`
- [core rules (pattern)] `prose-craft.unclear-antecedent` lookaround: `(?m)(?:^\s*|(?<=[.!?;]\s)|(?<=^[-*]\s)|(?<=^>\s))(?:This|That|These|Those)\s+(?:is|are|was|were|means?|makes?|gives?|lets?|causes?|happens?|works?|matters?|help`
- [core rules (pattern)] `prose-discipline.frozen-verb` lookaround: `(?<![\w-])(?:perform|conduct|carry out|undertake|execute|make|do|give|provide|effect|achieve|obtain|initiate|implement)(?:s|ed|ing)?\s+(?:(?:a|an|the)\s+)?(?!(?`
- [core rules (pattern)] `prose-discipline.hedged-hedge` lookaround: `(?<![\w-])(?:may|might|could|can|should|would)\s+(?:\w+\s+)?(?:help(?:s)?\s+(?:to\s+)?|serve\s+to\s+|work\s+to\s+|tend\s+to\s+)?\w+\s+(?:some|certain|various|se`
- [core rules (pattern)] `prose-inflation.intensifier` lookaround: `\b(?:very|extremely|exceedingly|remarkably|surprisingly|incredibly|unbelievably|tremendously|immensely|enormously|vastly|hugely|terribly|awfully|wildly|insanely`
- [core rules (pattern)] `prose-scope.epigram` backreference: `\b[Tt]he \w+ (\w+s) [\w']+\. [Tt]he \w+ \1 [\w']+\.|\b[Tt]he \w+ (?:is|are|does|has) [\w' ]{2,20}\. [Tt]he \w+ (?:is|are|does|has) [\w' ]{2,20}\.|[,;] not (?:yo`
- [core rules (pattern)] `prose-scope.formulaic-subject-verb-slogan` lookaround: `^(?:the|a|an|this|that|these|those|its|our|your|their)\s+[\w-]+(?:\s+[\w-]+)?\s+(?:is|are|was|were|has|have|does|do|can|cannot|will|would|should|must|[a-z][\w-]`
- [core rules (pattern)] `prose-scope.implementation-leak` lookaround: `(?<!<dd>)(?<![Tt]imeout of )(?<![Bb]udget of )(?<![Dd]eadline of )(?<![Ll]imit of )(?<![Ww]ithin )(?<![Aa]fter )(?<![Ee]very )\b\d+(?:\.\d+)?\s?(?:ms|millisecon`
- [core rules (pattern)] `ste-practices.omitted-conjunction-that` lookaround: `\b(?:(?:make\s+sure|makes\s+sure|ensure[s]?|mean[s]?)\s+(?:the|this|these|a|an|your|our|it|you|we|they)\b|(?:verify|verifies|confirm[s]?)\s+(?:the|this|these|a|`
- [core rules (pattern)] `ste-practices.possessive-form-unclear` lookaround: `\b\w+s['’](?=\s+\w)|\b\w+['’]s\s+\w+['’]s\b`
- [core rules (pattern)] `ste-procedural.condition-after-command` lookaround: `^\s*(?:(?:(?:For|To|In|On|At|With|Without|During|Using|From|By)\b[^,.!?]{0,80},\s+|(?:Or|Then|Next|First|Second|Third|Finally|Also|Otherwise|Alternatively|Optio`
- [core rules (pattern)] `ste-punctuation.hyphen-missing-in-compound-modifier` lookaround: `\b(?:read|write|command|line|end|user|client|server|side|open|closed|long|short|high|low|first|second|third|multi|single|cross|zero|non|self|pre|post|re|semi|we`
- [core rules (pattern)] `ste-sentences.missing-article-or-determiner` lookaround: `^\s*(?:[A-Z][a-z]+)\s+(?:config|manifest|worker|queue|token|endpoint|repository|branch|cache|index|schema|dashboard|runner|scheduler|cluster|node|pod|container|`
- [core rules (pattern)] `ste-verbs.passive-voice` lookaround: `\b(?:am|is|are|was|were|be|been|being)\s+(?:(?:\w+ly|often|also|always|never|still|then|now|already)\s+)?(?:(?:\w{3,}(?:ed|en|wn)|set|put|cut|run|read|sent|buil`
- [core rules (substitution)] `ai-tells-content-shape.durable-vocabulary-habits` lookaround: `(?:\brepresents(?=\s+(?:a|an)\s+(?:\w+\s+)?(?:significant|substantial|major|important|pivotal|key|fundamental|dramatic|bold|huge|giant|big|step|shift|leap|miles`
- [core rules (substitution)] `prose-craft.latinisms` lookaround: `(?:\b(?:e\.g\.|eg\.|e\.g)(?=[\s,]))|(?:\b(?:i\.e\.|ie\.|i\.e)(?=[\s,]))|(?:\betc\.?(?=[\s,)]|$))|(?:\bvs\.?(?=\s))|(?:\bad hoc\b)|(?:\bper se\b)|(?:\bN\.B\.)|(?`
- [core rules (substitution)] `prose-inclusive.device-assumption` lookaround: `(?:\bsee the\b(?= (?:icon|button|badge|indicator)))|(?:\bclick(?:ing|ed)? (?:on |the |this ))|(?:\btap(?:ping|ped)? (?:on |the |this ))|(?:\bdouble[- ]click (?:`
- [core rules (substitution)] `prose-inclusive.exclusive` lookaround: `(?:\bmaster(?! branch| boot record| broker| key| password| node| subscription agreement| serviced? agreement| licen[cs]e agreement| agreement| data)\b)|(?:\b(?:`
- [engine exceptions/matching] `crates/slopvac-engine/src/exceptions.rs:74` lookaround: `(?<![\w"”])"[^"\n]{1,200}(?<!\s)"(?!\w)|(?<![\w“])“[^”\n]{1,200}”|(?<![\w‘])‘[^’\n]{1,200}’`
- [engine exceptions/matching] `crates/slopvac-engine/src/matching.rs:214` lookaround: `\A(?:\boffers(?!\s+(?:you\s+)?(?:a|an|the)\s+(?:[\w-]+\s+)?(?:path|way|approach|means|route|balance|trade-?off|alternative|compromise|middle)\b))\z`
- [engine exceptions/matching] `crates/slopvac-engine/src/matching.rs:214` lookaround: `\A(?:\bvs\.?(?=\s))\z`
- [engine exceptions/matching] `crates/slopvac-engine/src/matching.rs:214` lookaround: `\A(?:\bslave(?! broker)\b)\z`
- [engine exceptions/matching] `crates/slopvac-engine/src/matching.rs:214` lookaround: `\A(?:\boffers(?!\s+(?:you\s+)?(?:a|an|the)\s+(?:[\w-]+\s+)?(?:path|way|approach|means|route|balance|trade-?off|alternative|compromise|middle)\b)(?=\s+(?:a|an|th`
- [engine exceptions/matching] `crates/slopvac-engine/src/matching.rs:214` lookaround: `\A(?:\b(?:e\.g\.|eg\.|e\.g)(?=[\s,]))\z`
- [engine exceptions/matching] `crates/slopvac-engine/src/matching.rs:214` lookaround: `\A(?:\bmaster(?! branch| boot record| broker| key| password| node| subscription agreement| serviced? agreement| licen[cs]e agreement| agreement| data)\b)\z`
- [engine exceptions/matching] `crates/slopvac-engine/src/matching.rs:214` lookaround: `\A(?:\betc\.?(?=[\s,)]|$))\z`
- [engine exceptions/matching] `crates/slopvac-engine/src/matching.rs:214` lookaround: `\A(?:\b(?:i\.e\.|ie\.|i\.e)(?=[\s,]))\z`
- [engine exceptions/matching] `crates/slopvac-engine/src/matching.rs:214` lookaround: `\A(?:(?<!such )\bacts as\b)\z`
- [engine exceptions/matching] `crates/slopvac-engine/src/matching.rs:214` lookaround: `\A(?:\bfeatures(?=\s+(?:a|an|the|\d)\b))\z`
- [engine exceptions/matching] `crates/slopvac-engine/src/matching.rs:214` lookaround: `\A(?:\bsee the\b(?= (?:icon|button|badge|indicator)))\z`
- [engine exceptions/matching] `crates/slopvac-engine/src/matching.rs:214` lookaround: `\A(?:\brepresents(?=\s+(?:a|an)\s+(?:\w+\s+)?(?:significant|substantial|major|important|pivotal|key|fundamental|dramatic|bold|huge|giant|big|step|shift|leap|mil`
- [engine exceptions/matching] `crates/slopvac-engine/src/matching.rs:224` lookaround: `\betc\.?(?=[\s,)]|$)`
- [engine exceptions/matching] `crates/slopvac-engine/src/matching.rs:224` lookaround: `\bmaster(?! branch| boot record| broker| key| password| node| subscription agreement| serviced? agreement| licen[cs]e agreement| agreement| data)\b`
- [engine exceptions/matching] `crates/slopvac-engine/src/matching.rs:224` lookaround: `\bslave(?! broker)\b`
- [engine exceptions/matching] `crates/slopvac-engine/src/matching.rs:224` lookaround: `\boffers(?!\s+(?:you\s+)?(?:a|an|the)\s+(?:[\w-]+\s+)?(?:path|way|approach|means|route|balance|trade-?off|alternative|compromise|middle)\b)`
- [slopvac-context] `crates/slopvac-context/src/builtin/sentence.rs:53` lookaround: `\b(?:must(?:\s+not)?|required|requires?|cannot|can['’]t|can\s+not|shall(?:\s+not)?|should\s+not)\b|(?<![\w-])only(?![\w-])|(?:^\W*|[.!?:;]\s+)never\s+(?=[a-z])`
- [slopvac-text built-ins] `crates/slopvac-text/src/classify.rs:51` lookaround: `^(?:please\s+)?(?:do\s+not\s+|do\s+)?(?:disconnect|configure|downgrade|consider|identify|navigate|remember|schedule|confirm|connect|disable|execute|inspect|inst`
- [slopvac-text built-ins] `crates/slopvac-text/src/classify.rs:54` lookaround: `^(?:please\s+)?(?:(?:disconnect|configure|downgrade|consider|identify|navigate|remember|schedule|confirm|connect|disable|execute|inspect|install|monitor|perform`
- [slopvac-text built-ins] `crates/slopvac-text/src/classify.rs:64` lookaround: `^(?:please\s+)?(?:don't|never|do\s+not)\s+[A-Za-z]+(?![\p{L}\p{Nd}_])`
- [slopvac-text built-ins] `crates/slopvac-text/src/classify.rs:67` lookaround: `^to\s+(?:disconnect|configure|downgrade|consider|identify|navigate|remember|schedule|confirm|connect|disable|execute|inspect|install|monitor|perform|prepend|pro`
- [slopvac-text built-ins] `crates/slopvac-text/src/classify.rs:68` lookaround: `^(?:remember|make\s+sure)\s+to\s+(?:disconnect|configure|downgrade|consider|identify|navigate|remember|schedule|confirm|connect|disable|execute|inspect|install|`
- [slopvac-text built-ins] `crates/slopvac-text/src/classify.rs:71` lookaround: `^\s*(?:>\s*)?(?:\*{0,2})?(?P<marker>WARNING|CAUTION|DANGER|NOTICE|ATTENTION|IMPORTANT)(?![\p{L}\p{Nd}_])(?:\*{0,2})?\s*[:.!]?`
- [slopvac-text built-ins] `crates/slopvac-text/src/classify.rs:78` lookaround: `^\s*(?:>\s*)?(?:\*{0,2})?(?:NOTE(?:\*{0,2})?\s*:\s*|(?:TIP|HINT|INFO)(?![\p{L}\p{Nd}_])(?:\*{0,2})?\s*[:.!]?)`
- [slopvac-text built-ins] `crates/slopvac-text/src/measure.rs:17` lookaround: `;|\s+(?:--|—|–)\s+|,\s*(?:and|but|or|so|yet|then|while|whereas|although)\s+(?=\w+\s+\w)|,\s*which\b|,\s*(?:however|therefore|meanwhile|nevertheless|consequently`
- [slopvac-text built-ins] `crates/slopvac-text/src/measure.rs:36` lookaround: `(?<!\*)\*\*(?!\s)[^*\n]+?(?<!\s)\*\*(?!\*)|__(?!\s)[^_\n]+?(?<!\s)__`
- [slopvac-text built-ins] `crates/slopvac-text/src/measure.rs:39` lookaround: `—|(?<![-\w])--(?![-\w>])`
- [slopvac-text built-ins] `crates/slopvac-text/src/measure.rs:43` lookaround: `(?s)(?<!`)(?P<ticks>`+)(?P<body>.*?)(?<!`)(?P=ticks)(?!`)`
- [slopvac-text built-ins] `crates/slopvac-text/src/tokens.rs:38` lookaround: `(?<!`)`{1,}(?P<body>[^`\n]*?)`{1,}(?!`)`
- [slopvac-text built-ins] `crates/slopvac-text/src/tokens.rs:43` lookaround: `(?:https?://|ftp://)[^\s<>]+|(?<!\w)/(?:[\p{L}\p{Nd}_.-]+/)+[\p{L}\p{Nd}_.-]*[\p{L}\p{Nd}_]|(?<!\w)(?:\./)?(?:[\p{L}\p{Nd}_.-]+/)+[\p{L}\p{Nd}_.-]*[\p{L}\p{Nd}_`
- [slopvac-text built-ins] `crates/slopvac-text/src/tokens.rs:47` lookaround: `(?<!\w)[\p{L}_][\p{L}\p{Nd}_]*(?:(?:\.|::)[\p{L}\p{Nd}_]+)+(?!\w)`
- [slopvac-text built-ins] `crates/slopvac-text/src/tokens.rs:49` lookaround: `(?<!\w)(?=[A-Za-z0-9_]*[A-Za-z])(?=[A-Za-z0-9_]*\d)[A-Za-z][A-Za-z0-9_]*\d[A-Za-z0-9_]*(?!\w)`
- [slopvac-text built-ins] `crates/slopvac-text/src/tokens.rs:55` lookaround: `(?<![\w'])"[^"\n]{1,200}"(?!\w)|(?<![\w'])'[^'\n]{2,200}'(?!\w)|(?<!\w)“[^”\n]{1,200}”(?!\w)|(?<!\w)‘[^’\n]{1,200}’(?!\w)`
- [slopvac-text built-ins] `crates/slopvac-text/src/tokens.rs:65` lookaround: `^\s*(?:(?:step\s+\d+(?:\.\d+)*[.:)]?)|(?:\([A-Za-z0-9ivxIVX]+\)|(?:[A-Za-z]|[ivxIVX]+|\d+)[.)])|(?:\d+(?:\.\d+)+\.?)\s*)(?=\s|$)`
- [slopvac-text built-ins] `crates/slopvac-text/src/tokens.rs:73` lookaround: `(?<!\w)(?:no|number|fig|figure|sec|section|ref)\.\s+\d+(?!\w)`
- [slopvac-text built-ins] `crates/slopvac-text/src/tokens.rs:74` lookaround: `(?<!\w)(?:[A-Za-z]\.){2,}(?!\w)`
- [slopvac-text built-ins] `crates/slopvac-text/src/tokens.rs:76` lookaround: `(?<!\w)[+-]?\d+(?:[.,]\d+)*(?:\.\d+)*(?:\s+(?:degrees?\s+(?:Celsius|Fahrenheit)|degrees?))?(?:\s+(?:(?:°[CF]?|K|[numkKMGTP]?(?:m|g|s|V|W|J|N|Pa|Hz|B|bps|bit|byt`
- [slopvac-text built-ins] `crates/slopvac-text/src/tokens.rs:81` lookaround: `(?<!\w)(?:[A-Z][A-Za-z0-9'’-]*|of|and|for|the)(?:\s+(?:[A-Z][A-Za-z0-9'’-]*|of|and|for|the)){1,}(?!\w)`

## Parity allowlist size (identical and exercised)
881
- parity-golden-en-US.jsonl: limit_hits=0
- parity-golden-en-GB.jsonl: limit_hits=0
- parity-docs-en-US.jsonl: limit_hits=0
- parity-docs-en-GB.jsonl: limit_hits=0
- parity-k8s-en-US.jsonl: limit_hits=0
- parity-k8s-en-GB.jsonl: limit_hits=0
- parity-eval-en-US.jsonl: limit_hits=0
- parity-eval-en-GB.jsonl: limit_hits=0
- parity-golden-rst-en-US.jsonl: limit_hits=0
- parity-golden-rst-en-GB.jsonl: limit_hits=0
- parity-fuzz.jsonl: limit_hits=0
- parity-fuzz-directed.jsonl: limit_hits=0
