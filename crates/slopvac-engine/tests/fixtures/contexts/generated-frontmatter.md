<!-- rule: core:cat.gated -->
<!-- synthetic: {"match": {"kind": "tokens", "tokens": ["leverage"]}, "contexts": {"skip": ["generated"]}} -->

<!-- silent: a boolean generated flag -->
---
generated: true
---
We leverage the cache.

<!-- silent: a yes generated flag -->
---
generated: yes
---
We leverage the cache.

<!-- fires: a false generated flag -->
---
generated: false
---
We leverage the cache.

<!-- fires: a numeric generated value -->
---
generated: 1
---
We leverage the cache.

<!-- fires: a quoted string that is not a flag -->
---
generated: "maybe"
---
We leverage the cache.

<!-- silent: a quoted string that is a flag -->
---
generated: "true"
---
We leverage the cache.
