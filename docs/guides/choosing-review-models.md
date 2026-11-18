# Choosing models for review

Slopvac review has two models: the author that wrote the text and the reviewer that checks it. The reviewer must be a different model from the author, because a reviewer on the author's model tends to repeat the author's blind spots.

`slopvac setup`, `slopvac prime` and `slopvac review` are not wired into the `slopvac` binary yet. The commands below describe the interface the agents library provides. They work as written once the binary mounts them.

## What is deterministic

These steps give the same result on the same input:

- `slopvac lint` finds rule violations.
- `slopvac claims` lists the changed, added and dropped claims between two versions.
- `slopvac review --dry-run` picks the review type and prints the exact text a provider would receive.

These steps are not deterministic, and slopvac does not control them:

- The author agent. You pick its model in your harness.
- The reviewer. It is a model in your harness or a model behind a provider plugin.

Slopvac can compare model names only. It reads them from the config, from flags and from what a provider reports. It cannot see which model a harness actually ran.

## Where each model is set

| Model | Where you set it |
| --- | --- |
| Author | The model of your harness session. Optionally `--author-model` on `slopvac review`, or `author_model` under `[review]` in the config. |
| Reviewer subagent | `slopvac setup <harness> --agent --reviewer-model <model>`. It writes the `model:` line of the reviewer agent file. |
| Reviewer through a provider plugin | `provider` and `model` under `[review]` in the config, or `--reviewer-model` on `slopvac review`. |

For the name that slopvac compares, a flag wins over the config, and the config wins over the name a provider reports. For the author, the flag wins over `author_model`. When no source names the author, the author is unknown.

## Harnesses

`slopvac setup` supports `agents`, `codex`, `omp`, `kiro`, `claude` and `opencode`. Only `claude` and `opencode` have a reviewer agent file. `setup --agent` on any other harness is a usage error. For those harnesses, slopvac installs steering and the skill. The reviewer is then whatever fresh-context reviewer your harness starts, and you choose its model with that harness's own settings.

### OpenCode

`setup opencode --agent` writes `.opencode/agents/slopvac-reviewer.md`.

- The `model:` value is `provider/model`, with an optional `#variant`. Example: `anthropic/claude-sonnet-5-5#high`.
- A subagent with no `model:` inherits the model of the parent session. That is usually the author's model.
- OpenCode has no `inherit` value. `setup` treats `--reviewer-model inherit` as no model.

Source: <https://opencode.ai/docs/agents>.

### Claude Code

`setup claude --agent` writes `.claude/agents/slopvac-reviewer.md`.

- The `model:` value is an alias (`sonnet`, `opus`, `haiku` or `fable`), a full model ID that starts with `claude-`, or `inherit`. Slopvac rejects any other value with the error code `setup-reviewer-model-format`.
- `inherit` means the model of the main conversation.
- When several sources set a model, Claude Code uses the first in this list:
  1. The `model` parameter that Claude passes when it starts the subagent.
  2. The `model` field in the agent file.
  3. The `CLAUDE_CODE_SUBAGENT_MODEL` environment variable.
  4. The model of the main conversation.
- With `CLAUDE_CODE_SUBAGENT_MODEL_FORCE=1`, Claude Code ignores the `model` field. A forced model can silently make the reviewer the author's model, so check your settings files for it.

Source: <https://code.claude.com/docs/en/sub-agents#choose-a-model>. The full page is <https://code.claude.com/docs/en/sub-agents>.

## The unset-model warning

Without `--reviewer-model`, or with `--reviewer-model inherit`, the reviewer agent has no model of its own, so the harness picks one:

- OpenCode uses the model of the parent session.
- Claude Code uses `CLAUDE_CODE_SUBAGENT_MODEL` when it is set, and the model of the main conversation otherwise.

Either way, the reviewer can be the author's model. Setup then does the following:

- It writes a commented line, `# model: <a model from a different family than the author's>`, in the agent file. The line has no effect until you replace it with a `model:` line.
- It reports the warning `setup-reviewer-model-unset` and exits 0.
- `slopvac setup <harness> --agent --check` reports the same warning. The file counts as current.

## What slopvac checks

Slopvac compares the author name and the reviewer name and refuses a match with the error `review-same-model`. The comparison ignores the provider prefix and the `#variant` suffix, so `anthropic/claude-sonnet-5-5#high` matches `claude-sonnet-5-5`.

When either name is unknown, slopvac cannot run the check. The plan then carries the notice `review-model-unchecked`, which names the missing side, and `reviewer_model_checked` stays `false`. To close the gap, pass `--author-model` and `--reviewer-model`, or set `author_model` and `model` under `[review]`.

The verdict block that `slopvac prime review` asks an agent for has one line for this:

```text
Reviewer: <model the harness used, or unknown>; author <model or unknown>; different: yes | no | not checked
```

The agent reports the model its harness actually ran, or `unknown`.

## Inspect a run

```sh
slopvac review revised.md --original original.md --dry-run
```

The command prints the review type and why, the scope, the exact payload, the models it resolved, and any notices. A dry run sends nothing to a provider.

## Model recommendations

[Reviewer model recommendations](../model-recommendations.md) ranks models by eval results. Those recommendations are pending re-evaluation. Treat them as a starting point, not as a verdict.
