# Migrate from 2.x to 3.0

Slopvac 3.0 has no 2.x compatibility mode. A 2.x config makes every command that reads config exit 2 with `config-legacy-version`. `slopvac migrate` is the only path.

A config with no `schema_version`, or with `schema_version = 2`, needs migrating. One with `schema_version = 3` is reported as already migrated.

## 1. Find the 2.x surfaces
<!-- prime: include -->

Look for:

- `.slopvac.toml`, or a `slopvac.toml` with no `schema_version = 3`.
- A `[tool.slopvac]` table in `pyproject.toml`.
- `slopvac-allow` comments whose `rule=` value has no pack prefix, such as `rule=prose-format.no-unicode-dash`.
- Steering blocks between `<!-- slopvac:begin -->` and `<!-- slopvac:end -->` in agent instruction files.

## 2. Preview
<!-- prime: include -->

```sh
slopvac migrate
```

The command prints a diff of every file it would change, lists the rules whose finding counts change, and exits 0. It writes nothing.

## 3. Resolve blockers
<!-- prime: include -->

Some 2.x settings have no 3.0 equivalent. The command stops with exit 2 and prints one `config-legacy-dropped` line for each, with the file, line, key and reason. It writes nothing.

Causes:

- `[vale]` with `enabled = true`, or with `binary`, `config` or `styles` and no `enabled = false`. Delete the table or set `enabled = false`.
- A rule or category name that is not in the rename table.
- An unknown key, or a `guidance` table inside an override.

Edit the file and preview again.

A 2.x category with no rules is dropped with `migrate-category-removed`; migrate exits 0.

These failures also exit 2 and write nothing:

- `document-too-large`: a document is over 64 MiB. Split it.
- `config-too-large`: a config is over 1 MiB. Shorten it.
- `migrate-run-limit`: too many files or bytes. Use a smaller directory.
- `migrate-not-utf8`: a file is not UTF-8. Convert it.
- `config-invalid-toml`: a config is not valid TOML. The message gives the line.
- `config-invalid-type`: `schema_version` is not an integer. Fix it.
- `migrate-lint-failed`: the lint could not finish. Fix the file named.

## 4. Apply
<!-- prime: link -->

```sh
slopvac migrate --write
```

The command copies each file it replaces to `.slopvac-migrate-backup`, then rewrites it. Config comments and key order stay. It does the following:

- Moves `.slopvac.toml` and `[tool.slopvac]` into `slopvac.toml`, and adds `schema_version = 3`.
- Renames categories and rules to their 3.0 names. A rule with only a `severity` becomes a bare level.
- Moves `locale.default` to `locale`, `locale.allow` to `terms.allow`, and `vocabulary.path` to `terms.blocklists`.
- Deletes a `[vale]` table that has `enabled = false`.
- Qualifies the rule id in each `slopvac-allow` comment. Comments in code fences and code spans stay.
- Replaces each steering block with the 3.0 text.

The command stops if one directory holds more than one of `slopvac.toml`, `.slopvac.toml` and `[tool.slopvac]`. Remove the unused ones.

To keep the 2.x gates, add `--preserve-behavior`. It selects the `core:legacy-<profile>` style. Each override then needs one pattern, such as `docs/api` or `docs/**`. Other patterns fail with `migrate-override-overlap`.

## 5. Verify and undo
<!-- prime: include -->

Run `slopvac lint` and compare it with the preview report.

To undo a write, run `slopvac migrate --restore`. It restores the backed-up files, removes the files the write created, and deletes the backup. To keep the migration, delete `.slopvac-migrate-backup`. A second `slopvac migrate` finds nothing to change.

## 6. Removed in 3.0
<!-- prime: link -->

These 2.x surfaces fail with exit 2. Update scripts, CI and hooks by hand.

| Surface | What to do |
|---|---|
| `--profile` on `lint`, `rules`, `explain` and `init` | Set the style preset in `slopvac.toml`. |
| `--rules-dir` on `lint`, `rules`, `explain`, `prime` and `reference` | Declare a local pack in `slopvac.toml`. |
| `--no-vale`, `doctor --binary` | Delete the flag. Vale is removed. |
| `--comments` | Use `--mode code-comments`. |
| `compile` and its options (`--outdir`, `--profile`, `--config`, `--rules-dir`, `--no-validate`, `--format`) | Delete the call. Compiled Vale styles are removed. |
| `cache` and its options (`--prune`, `--all`) | Delete the call. The Vale cache is removed. |
| Action inputs `profile`, `vale`, `vale-version` and `rules-dir` | Delete the input. Set the style preset and packs in `slopvac.toml`. |
| Hook id `slopvac-no-vale` | Delete the hook. |
| Hook id `slopvac-strict` | Use the `slopvac` hook with `profile = "strict"` and `styles = ["core:legacy-strict"]` in `slopvac.toml`. |
