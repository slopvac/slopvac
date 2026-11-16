It's not a linter, it's a review partner.

The linter reports 61 rules and scores the document.

It's less about speed and more about determinism.

The gate produces the same findings on every run.

The Harness Measures, It Does Not Judge

The Harness Measures.

The parser reads JSON; it does not read YAML.

The parser reads JSON.

The client supports HTTP; it does not support FTP.

The client supports HTTP.

The control acknowledges the run; it does not confirm that it started.

The control acknowledges the run.

Testing is not a phase. It is a habit.

Run the tests on every push.

The monorepo is a coordination boundary. It is not a requirement to combine services into one deployment.

The monorepo holds the four services; each still deploys on its own.

This isn't about speed. It's about correctness.

The gate rejects malformed input before it reaches the parser.

The question is not whether the cache is sophisticated. It is whether freshness justifies the cost.

Freshness decides the design; the cache is invalidated on every catalog event.

The plugin is not loaded at all; it is replaced, not merged.

The project plugin replaces the user plugin of the same name.

Not because it is fast, but because it is deterministic.

It is deterministic, so repeated runs produce the same result.

Documentation is not an afterthought. It is a feature.

The docs build runs in the same pipeline as the code.

The limit is a ceiling. It is not a target to aim for.

The limit is 100 requests per minute. It is not configurable.

Security is not a checkbox. It is a process.

The lock is a lease. It is not a mutex.

Not a bug. Not a feature. A design flaw.

The loader drops the last row, which is a design flaw.

fast, reliable, and secure

The gate reports every finding with a line number.

The system is fast, reliable, and battle-tested.

The gate reports every finding with a line number.

The guide is clear, concise, and easy to follow.

The release covers planning, implementation, and testing.

Here's the thing: the loader reads the lockfile.

The loader reads the lockfile.

The result? Devastating.

The migration dropped two rows.

So why does this matter?

A false status claim ships a document that lies.

In this guide, we'll explore the loader.

The loader resolves every path in the manifest.

In conclusion, the gate reads one config file.

The gate reads one config file.

Happy migrating!

Restart the worker after the config is written.

Three pillars support this: speed, safety, and cost.

The gate is fast, refuses an unresolvable style, and runs in CI.

The only way to fix this is to re-run the sync.

One way to fix this is to re-run the sync when the cache is stale.

Every Project Needs A Lockfile.

A project with more than one direct dependency needs a lockfile.

Tooling not pinned: versions may drift.

The lockfile pins the formatter version used by CI.

Excluded from this review: browser checks and visual polish.

Browser checks are not run; use the smoke command before release.

| Coverage | Status |
|---|---|
| Outside tooling scope | Not included |

The parser test suite covers TOML comments; browser behavior is not run here.

Config is recorded elsewhere.

The deployment manifest records the region and is the source of truth.

- Excluded: browser checks and visual polish.

Trust cannot restore a capability excluded from the child's set.

Visual polish is excluded from this release.

Dependencies must not be pinned to a branch.

Out of scope: visual polish.

Do not pin the formatter to a floating tag.

Excluded from verification: accessibility checks.

Excluded: clients MUST NOT access telemetry.

The deploy helper itself never creates a branch, a tag, a release or a webhook.

The deploy helper writes one file, `.deploy/state.json`; branch protection rejects its token on `main`.

| `hooks/` | Advisory reminders on each edit. They never stop a command. |

| `hooks/` | Advisory reminders on each edit; each hook exits 0, so the command runs. |

The release agent never merges a pull request. It runs no deploy step before the release gate resolves.

A maintainer merges the release pull request. CI starts the deploy job after the release gate resolves.

The importer appends rows; it never deletes one.

The importer appends rows. The database role it connects as has no DELETE grant.

The client never retries a request that returned 409.

The client MUST NOT retry a request that returned 409.

The agent never edits the lockfile.

Never edit the lockfile by hand.

Outside a run the plugin spawns no process.

Outside a run the plugin only registers its slash commands.

Think of it like a highway system for data.

The bus fans one message out to every registered handler.

Experts agree that lockfiles reduce drift.

The npm RFC 0012 measured a 40% drop in resolution variance with a lockfile.

Whether you're a beginner or a seasoned pro, this guide has something for you.

This guide assumes you have run `apm install` once.
