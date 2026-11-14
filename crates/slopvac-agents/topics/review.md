<!-- summary: Review prose before hand-off, by review type -->
# Prime: review

Run before you hand off prose. Someone other than the author checks the change.
{% if review_type == "auto" %}
## Pick the type

`slopvac review <file> --original <before> --dry-run` picks the type and prints why.

- `mechanical`: the change rewords sentences. No literal, negation or fact changes. Lint and the claims diff are enough.
- `update`: a bounded change that touches facts. Check the changed claims.
- `rewrite`: half the document or more changed, or no original exists. Check the whole document.
- A genre with contract weight (reference, specification, API, decision record) never runs as `mechanical`.

Run `slopvac prime review --type <type>` for the steps of one type.
{% else %}
## Steps for {{ review_type }}

1. Lint: `slopvac lint <file> --format json`. Exit {{ exit_findings }} means a gate failed. Exit {{ exit_incomplete }} means the run is incomplete.
2. List the changes: `slopvac claims <file> --original <before>`.
{% if review_type == "mechanical" %}
3. Confirm that no claim lists a changed literal or negation and that `dropped` has no `fact_candidate`.
4. If either appears, the change is an `update`. Rerun with `--type update`.
{% elif review_type == "update" %}
3. Check each changed and added claim against code or a source. Skip unchanged claims.
4. For each `fact_candidate` in `dropped`, restore the fact or justify the removal.
5. Reviewer pass: give one fresh-context reviewer the changed passages, not the whole document.
{% else %}
3. List the facts in the original. Confirm that each one survived or was wrong.
4. Check every present-tense claim in the new document against code or a source.
5. Reviewer pass: give one fresh-context reviewer the whole document and the original.
{% endif %}
{% endif %}
## Scope

- Fact checking covers changed claims and lost-fact candidates. Nothing else is in scope.
- You may add 5 checks, plus half the required checks rounded up, without asking.
- Above that, ask the user to confirm the exact scope. Then rerun with `--scope-approved`.
{% if review_approval == "auto" %}
- `review_approval = "auto"` is set, so go ahead without asking.
{% else %}
- `review_approval = "auto"` in the config skips the question.
{% endif %}

## Reviewer

- The reviewer model must differ from the author's. `slopvac review --reviewer-model <model>` rejects the author's model.
- A model provider is a plugin. It runs only after `slopvac plugins trust` and a `[review] provider` entry.
- `slopvac review --dry-run` prints the exact text a provider would get and calls nothing. Offline mode blocks every provider.
- Providers receive changed claims for `mechanical`, changed passages for `update` and the whole document for `rewrite`.
{% if level != "minimal" %}

## Verdict

```text
VERDICT: PASS | REVISE
Gate:    lint exit <code>, unchecked <entries or none>
Type:    <review type>, scope <required> checks, <extra> extra
Claims:  <verified, or the claim that does not hold>
Facts:   <all kept, or the facts the revision lost>
Reviewer: <model>, different from the author's
Action:  <the single highest-value change, or none>
```

Report REVISE for lint exit 1 or 2, a failed claim, a lost fact or an open reviewer finding.
{% endif %}
