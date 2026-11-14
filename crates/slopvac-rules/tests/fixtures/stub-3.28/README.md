# STUB(slopvac-8l4.3.28)

These files are copies from PR #281 (branch `feat/schema-rule`, commit `b5ae01f80c`), which
defines the v3 rule and category schemas:

- `schemas/rule.schema.json` and `schemas/category.schema.json`
- `conformance/rule` and `conformance/category`
- `convert_rules_v2_to_v3.py`

The slopvac-rules tests use them until #281 lands. Bead slopvac-8l4.3.28 deletes this
directory and points the tests at `schemas/v3`.
