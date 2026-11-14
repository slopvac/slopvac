# Migrate from 2.x to 3.0

Slopvac 3.0 has no 2.x compatibility mode. A 2.x config makes every command that reads config exit 2 with `config-legacy-version`. Nothing is linted until you migrate. `slopvac migrate` is the only path.

Migrations chain from the version it detects. One step exists: 2.x to 3.0. A config with no `schema_version`, or with `schema_version = 2`, starts at that step. A config with `schema_version = 3` needs no migration, and `slopvac migrate` reports it as already migrated.

## 1. Find the 2.x surfaces

Look for these in the project:

- `.slopvac.toml`, or a `slopvac.toml` with no `schema_version = 3`.
- A `[tool.slopvac]` table in `pyproject.toml`.
- `slopvac-allow` comments whose `rule=` value has no pack prefix, such as `rule=prose-format.no-unicode-dash`.
- Steering blocks between `<!-- slopvac:begin -->` and `<!-- slopvac:end -->` in agent instruction files.
- Scripts, CI jobs and hooks that use a removed flag, command, action input or hook id (section 6).

## 2. Preview

```sh
slopvac migrate
```

The command prints a diff of every file it would change and exits 0. It writes nothing. It lints the project before and after, and lists the rules whose finding counts change.

## 3. Resolve blockers

Some 2.x settings have no 3.0 equivalent. The command stops with exit 2 and prints one `config-legacy-dropped` line for each, with the file, line, key and reason. It lists all of them in one run and writes nothing.

Common causes:

- `[vale]` with `enabled = true`, or with `binary`, `config` or `styles` and no `enabled = false`. Vale is removed in 3.0. Delete the table or set `enabled = false`.
- A rule or category name that is not in the rename table.
- A key 3.0 does not know, or a `guidance` table inside an override.

Edit the file, then run the preview again.

## 4. Apply

```sh
slopvac migrate --write
```

The command copies each file it replaces to `.slopvac-migrate-backup` and then rewrites it. Config comments and key order stay. It does the following:

- Moves `.slopvac.toml` and `[tool.slopvac]` into `slopvac.toml`, and adds `schema_version = 3`. Other content in `pyproject.toml` stays.
- Renames categories and rules to their 3.0 names, and writes a rule that had only a `severity` as a bare level.
- Moves `locale.default` to `locale`, `locale.allow` to `terms.allow`, and `vocabulary.path` to `terms.blocklists`.
- Deletes a `[vale]` table that has `enabled = false`.
- Qualifies the rule id in each `slopvac-allow` comment. Comments inside code fences and code spans are examples and stay.
- Replaces each steering block with the 3.0 text.

The command stops if one directory holds more than one of `slopvac.toml`, `.slopvac.toml` and `[tool.slopvac]`. 2.x read only one of them. Remove the ones you do not use and run again.

To keep the 2.x vocabulary behavior, add `--preserve-behavior`. It adds the STE style to `styles`.

## 5. Verify and undo

Run `slopvac lint` on the project. Compare the result with the per-rule report from the preview.

To undo a write, run `slopvac migrate --restore`. It puts the backed-up files back, removes the files the write created, and deletes the backup directory. If you keep the migration, delete `.slopvac-migrate-backup`.

A second `slopvac migrate` finds nothing to change.

## 6. Removed in 3.0

These 2.x surfaces fail with exit 2. Update scripts, CI and hooks by hand. `slopvac migrate` does not edit them.

| Surface | What to do |
|---|---|
| `--profile` on `lint`, `rules`, `explain` and `init` | Set the profile in `slopvac.toml`. |
| `--rules-dir` on `lint`, `rules`, `explain`, `prime` and `reference` | Declare a local pack in `slopvac.toml`. |
| `--no-vale`, `doctor --binary` | Delete the flag. Vale is removed. |
| `--comments` | Use `--mode code-comments`. |
| `compile` and its options (`--outdir`, `--profile`, `--config`, `--rules-dir`, `--no-validate`, `--format`) | Delete the call. Compiled Vale styles are removed. |
| `cache` and its options (`--prune`, `--all`) | Delete the call. The Vale cache is removed. |
| Action inputs `profile`, `vale`, `vale-version` and `rules-dir` | Delete the input. Set the profile and packs in `slopvac.toml`. |
| Hook id `slopvac-no-vale` | Delete the hook. |
