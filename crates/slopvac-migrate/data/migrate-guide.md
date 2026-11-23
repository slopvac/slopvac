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

It prints a diff and the finding count of each rule that changed, from the 3.0 engine on your files before and after migration. On the before side, each 2.x config and `slopvac-allow` rule id reads in its 3.0 form. The counts do not come from 2.x and do not show that 2.x results stay the same. It exits 0 and writes nothing.

## 3. Resolve blockers
<!-- prime: include -->

A 2.x setting with no 3.0 equivalent stops the command with exit 2 and one `config-legacy-dropped` line per setting (file, line, key, reason), and writes nothing.

Causes:

- `[vale]` with `enabled = true`, or with `binary`, `config` or `styles` and no `enabled = false`. Delete the table or set `enabled = false`.
- A rule or category name that is not in the rename table. A `slopvac-allow` comment that ends on a later line does not stop the command, whatever id it names: 2.x rejected it, so migrate leaves it as written and warns `invalid 2.x directive, left as is` (`migrate-directive-invalid`).
- An unknown key, or a `guidance` table inside an override.

Edit and preview again.

A 2.x category with no rules, or a rule that 3.0 removed, is dropped with `migrate-category-removed` or `migrate-rule-removed`. Both exit 0 and point to `removed-rules.toml`.

These also exit 2 and write nothing:

- `document-too-large`: a document is over 64 MiB. Split it.
- `config-too-large`: a config is over 1 MiB. Shorten it.
- `migrate-run-limit`: too many files or bytes. Use a smaller directory.
- `migrate-not-utf8`: a file is not UTF-8. Convert it.
- `config-parse`: a config is not valid TOML. The message gives the line.
- `config-invalid-type`: `schema_version` is not an integer. Fix it.
- `migrate-lint-failed`: the lint could not finish. Fix the file named.

## 4. Apply
<!-- prime: link -->

```sh
slopvac migrate --write
```

It copies each replaced file to a new backup directory, `.slopvac/migrate-backup/<UTC time>/`, then rewrites it. Config comments and key order stay. It:

- Moves `.slopvac.toml` and `[tool.slopvac]` into `slopvac.toml` and adds `schema_version = 3`.
- Renames categories and rules to 3.0 names. A rule with only a `severity` becomes a bare level.
- Moves `locale.default` to `locale`, `locale.allow` to `terms.allow`, and `vocabulary.path` to `terms.blocklists`.
- Rewrites each TOML blocklist it names: adds `schema_version = 3` and long-form `pos` values (`Noun` to `noun`).
- Deletes a `[vale]` table that has `enabled = false`.
- Qualifies the rule id in each `slopvac-allow` comment, except in code fences and code spans. A comment that ends on a later line stays as written, whatever id it names. A comment for a removed rule on its own line becomes `<!-- -->`, so the paragraphs around it stay apart.
- Replaces each steering block with the 3.0 text.

It stops if one directory holds more than one of `slopvac.toml`, `.slopvac.toml` and `[tool.slopvac]`. Remove the unused ones.

It stops, naming the file and line, for a blocklist that is outside the project, missing, 2.x JSON or YAML, or has a `pos` 2.x rejected.

`--preserve-behavior` keeps the 2.x gates with the `core:legacy-<profile>` style. Each override then needs one pattern, such as `docs/api` or `docs/**`. Others fail with `migrate-override-overlap`. An override that sets a profile takes that profile's levels in its own block.

Before it writes, the command loads each new config as 3.0 does. A config that 3.0 refuses stops it with exit 2 and `migrate-output-invalid`, and nothing is written. It also stops that way when an override sets a profile and a category applies above it. Such a category is set in that override, or in another override whose files meet its files. Any top-level category entry counts as well, whatever it sets. The migrated config could not keep the 2.x levels there.

When the 2.x config sets `vocabulary.path`, `--preserve-behavior` also lists the companion style `core:legacy-<profile>-vocabulary`, so the word blocklist keeps reporting at the 2.x level. Two layouts stop with `migrate-output-invalid`, exit 2 and nothing written: a `vocabulary.path` that only an override sets, and a blocklist together with any profile switch in an override. A follow-up will support these layouts.

## 5. Verify and undo
<!-- prime: include -->

Compare `slopvac lint` with the preview report.

To undo a write, run `slopvac migrate --restore`. It uses the backup of the last write, restores backed-up files, removes created files and deletes the backup. It keeps a `.slopvac/` directory you had before. Without `--force`, a file changed since stops it. A write that still has changes to make stops until you restore or remove that backup. To keep the migration, remove the backup directory. A second `slopvac migrate` finds nothing to change.

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
