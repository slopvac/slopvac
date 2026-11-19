# Migrate from 2.x to 3.0

3.0 has no 2.x compatibility mode: a 2.x config makes every command that reads config exit 2 with `config-legacy-version`. `slopvac migrate` is the only path.

A config with no `schema_version`, or `schema_version = 2`, needs migrating. `schema_version = 3` is reported as already migrated.

## 1. Find the 2.x surfaces
<!-- prime: include -->

Look for:

- `.slopvac.toml`, or a `slopvac.toml` with no `schema_version = 3`.
- A `[tool.slopvac]` table in `pyproject.toml`.
- `slopvac-allow` comments whose `rule=` value has no plugin prefix, such as `rule=prose-format.no-unicode-dash`.
- Steering blocks between `<!-- slopvac:begin -->` and `<!-- slopvac:end -->` in agent instruction files.

## 2. Preview
<!-- prime: include -->

```sh
slopvac migrate
```

It prints a diff and rules with changed finding counts, exits 0, and writes nothing.

## 3. Resolve blockers
<!-- prime: include -->

A 2.x setting with no 3.0 equivalent stops the command with exit 2 and one `config-legacy-dropped` line per setting (file, line, key, reason), and writes nothing.

Causes:

- `[vale]` with `enabled = true`, or with `binary`, `config` or `styles` and no `enabled = false`. Delete the table or set `enabled = false`.
- A rule or category name that is not in the rename table.
- An unknown key, or a `guidance` table inside an override.

Edit and preview again.

A 2.x category with no rules is dropped with `migrate-category-removed` and exit 0.

These also exit 2 and write nothing:

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

It copies each replaced file to `.slopvac-migrate-backup`, then rewrites it. Config comments and key order stay. It:

- Moves `.slopvac.toml` and `[tool.slopvac]` into `slopvac.toml` and adds `schema_version = 3`.
- Renames categories and rules to 3.0 names. A rule with only a `severity` becomes a bare level.
- Moves `locale.default` to `locale`, `locale.allow` to `terms.allow`, and `vocabulary.path` to `terms.blocklists`.
- Rewrites each TOML blocklist it names: adds `schema_version = 3` and long-form `pos` values (`Noun` to `noun`).
- Deletes a `[vale]` table that has `enabled = false`.
- Qualifies the rule id in each `slopvac-allow` comment, except in code fences and code spans.
- Replaces each steering block with the 3.0 text.

It stops if one directory holds more than one of `slopvac.toml`, `.slopvac.toml` and `[tool.slopvac]`. Remove the unused ones.

It stops, naming the file and line, for a blocklist that is outside the project, missing, 2.x JSON or YAML, or has a `pos` 2.x rejected.

`--preserve-behavior` keeps the 2.x gates with the `core:legacy-<profile>` style. Each override then needs one pattern, such as `docs/api` or `docs/**`. Others fail with `migrate-override-overlap`.

## 5. Verify and undo
<!-- prime: include -->

Compare `slopvac lint` with the preview report.

Undo a write with `slopvac migrate --restore`. It restores the backed-up files, removes created files, and deletes the backup. To keep the migration, delete `.slopvac-migrate-backup`. A second `slopvac migrate` finds nothing to change.

## 6. Removed in 3.0
<!-- prime: link -->

These 2.x surfaces fail with exit 2. Update scripts, CI and hooks by hand.

| Surface | What to do |
|---|---|
| `--profile` on `lint`, `rules`, `explain` and `init` | Set the style preset in `slopvac.toml`. |
| `--rules-dir` on `lint`, `rules`, `explain`, `prime` and `reference` | Declare a local plugin in `slopvac.toml`. |
| `--no-vale`, `doctor --binary` | Delete the flag. Vale is removed. |
| `--comments` | Use `--mode code-comments`. |
| `compile` and its options (`--outdir`, `--profile`, `--config`, `--rules-dir`, `--no-validate`, `--format`) | Delete the call. Vale styles are removed. |
| `cache` and its options (`--prune`, `--all`) | Delete the call. The Vale cache is removed. |
| Action inputs `profile`, `vale`, `vale-version` and `rules-dir` | Delete the input. Set the style preset and plugins in `slopvac.toml`. |
| Hook id `slopvac-no-vale` | Delete the hook. |
| Hook id `slopvac-strict` | Use the `slopvac` hook with `profile = "strict"` and `styles = ["core:legacy-strict"]` in `slopvac.toml`. |
