# Slopvac 3.0 plugins

Status: contract for 3.0. Plan: [slopvac-3.md](../design/slopvac-3.md). Decision: [0001-slopvac-3-architecture.md](../design/0001-slopvac-3-architecture.md), decision 9. Plugin protocol: [plugin-protocol.md](plugin-protocol.md). Plugin index: [pack-index.md](pack-index.md).

A plugin is a folder or an OCI artifact with a `slopvac-pack.toml` manifest. "Plugin" covers both kinds:

- A **data plugin** holds files that declare rules, styles, genres, terms and taggers. It holds no code.
- A **code plugin** also holds a command that the host runs as a subprocess (plugin-protocol.md).

Each file type has a schema in `schemas/v3/`, and the schema holds the field descriptions. One command group, `slopvac plugins`, manages both kinds (section 8).

2.x has no plugins. Its rules, genres and profiles are built into the Python package. Each statement below is a 3.0 requirement.

## 1. Files

| File | Schema | Notes |
| --- | --- | --- |
| `slopvac-pack.toml` | [`pack-manifest`](../../schemas/v3/pack-manifest.schema.json) | One per plugin, at the plugin root. |
| category files | [`category`](../../schemas/v3/category.schema.json) | Rule categories. |
| style files | [`style`](../../schemas/v3/style.schema.json) | Ordered presets. |
| genre files | [`genre`](../../schemas/v3/genre.schema.json) | Genres with detection signals. |
| tagger files | [`tagger`](../../schemas/v3/tagger.schema.json) | Declarative context taggers. |
| terms files | [`terms`](../../schemas/v3/terms.schema.json) | Also the format of `.slopvac/terms.toml`. |
| blocklist files | [`blocklist`](../../schemas/v3/blocklist.schema.json) | YAML, TOML or JSON (`.yml`, `.yaml`, `.toml` or `.json`; the suffix picks the format). The opt-in vocabulary check reads them. |
| `.slopvac/packs.lock` | [`packs-lock`](../../schemas/v3/packs-lock.schema.json) | Written by `slopvac plugins sync`. |

Every file carries `schema_version = 3`. Unknown keys are errors, and keys with the prefix `x-` are ignored. The manifest lists its files under `provides`. A path that leaves the plugin, by an absolute path, a drive letter, a backslash or a `..` segment, is invalid.

## 2. Manifest

- `slopvac_api` is the extension API the plugin targets, apart from `schema_version`. A host that implements an earlier API exits 2 and does not load the plugin.
- `kind` is `data` or `code`. `kind = "data"` forbids the `[plugin]` table. `kind = "code"` requires it. The lockfile, `slopvac plugins status` and the doctor report use the same two values.
- `requires` lists other plugins with a version requirement. The host resolves them first and reports a cycle as an error.
- The `[plugin]` table embeds the [`PluginDeclaration`](../../schemas/v3/plugin-protocol.schema.json#/$defs/PluginDeclaration) fields `network`, `hooks` and `features` flat, next to `command`, `args` and `env`. The host reads the declaration from the same bytes that `slopvac plugins trust` hashes (plugin-protocol.md section 4.3). A change to any field in the table changes the hash. The manifest closes the hook entries and the `features` table, so they take only the declared keys and `x-` keys.
- Rule ids take the plugin id as their prefix: `<plugin>:<category>.<rule>`. The id `core` belongs to the built-in plugin, and a manifest with the id `core` is invalid (section 9).

## 3. Styles

A style sets levels for categories and rules, and may bring terms. A style has no id: its name is the file name without the extension, qualified by the plugin id. A category entry is a bare level or a table with `severity`, `minimum_severity`, `max_per_100_words` and `weight`. `max_per_100_words = "none"` clears a budget that a lower layer set. 2.x had no way to unset a field, and 3.0 adds this one because a profile override must be able to drop a budget. The host applies `extends` in the order listed. Then it applies the style. When a config lists two or more styles, the last one wins. A rule entry beats a category entry in the same style.

`conflicts` names styles that must not be active with this one. When both are active, the run reports `style-conflict` as a notice with both names and uses the order rule above. Both styles stay active, and the notice is the only effect.

## 4. Genres

A genre may extend other genres, so `how-to` can build on `consumer`. Settings merge from the lowest extended genre up to the genre itself. A cycle in `extends` is an error.

Detection scores each genre by summing the weights of the signals that match a document. A genre is a candidate when its score reaches `min_score`. The highest score wins. A tie goes to the genre listed last in the config, and then to the one with the lexically smaller qualified id. A document with no candidate gets the genre `consumer`, the 2.x default. A `[[genres]]` block in the config or a front matter genre overrides detection, and the report names the source (lint-report.md).

Signal kinds are `path` (glob over the project-relative path), `front-matter` (a key, with an optional exact value), `heading` and `text` (PCRE2 patterns). Patterns follow the PCRE2 subset of the rule schema.

The 2.x genres map to genre files with these profiles: `consumer` normal, `reference` strict, `change-comms` normal, `internal` normal, `informal` relaxed.

## 5. Taggers

A declarative tagger sets one context tag. Output equals what a code tagger returns: a span, the tag as `kind`, `confidence` and `evidence`. Taggers are deterministic. A tagger has `match` entries or a `when` object, never both.

A tagger with `match` sets the tag on each text unit where a `match` entry matches and no `unless` entry matches. The tag span is the whole unit, and the tagger reads nothing but the unit text.

A tagger with `when` reads document structure. `when` takes these conditions, and all given conditions must hold:

- `path` matches the file path with forward slashes.
- `front_matter` names a `key` and optionally a `matches` pattern for its value. Without `matches` the key only has to exist.
- `heading_path` matches when any enclosing heading matches.
- `admonition_title` matches the title of the enclosing admonition.
- `block_kind` is one of `heading`, `paragraph`, `list-item`, `table-row`, `code-block` and `quote`. Blocks of kind `code-block` are skipped unless `block_kind` names them.
- `sentence` matches the text of a sentence.

`scope` sets how far the tag spreads: `sentence` (the default) tags each matching sentence, `paragraph` the matching block, `section` the enclosing heading section and `document` the whole document. `scope` needs `when`, and `when` excludes `unit`. The built-in structural taggers use this shape.

A genre lists `contexts` to give every document of the genre a context tag for the whole document, at confidence 1. Genres that extend it inherit the tags. A context such as `security` comes only from a declaration like this one and never from vocabulary.

## 6. Terms and blocklists

A terms entry needs `term`. `variants` lists spellings that checks may accept or flag and correct to `term`. `kind` is an optional kebab-case label.

Casing has three modes:

- Default: the check matches `term` and its variants without regard to case. A match with a casing that is neither the canonical spelling nor a declared variant is a finding.
- `ignore_case = true`: any casing passes. The entry works as an allow-list entry.
- `case_sensitive = true`: only the exact spelling of `term` and its variants match. Other casings are different words and never flag. Use it for names that are also common words, such as Go, Rust, React, Windows and REST, so that `go` in running text is not a finding.

The two flags cannot combine. The project file `.slopvac/terms.toml` and the terms files from plugins and styles merge. A term from the project file wins over a plugin term with the same text.

A blocklist file holds an `entries` list (`[[entries]]` in TOML) with `word`, `pos`, `reason` and an optional `replacement`. The loader trims `word` and folds it to lowercase. It rejects a repeated word and part-of-speech pair. An entry with no reason is invalid. A style or config names blocklists under `terms.blocklists`.

The `pos` value is case-sensitive and matches exactly, with no trimming. It loads as its long form. These values are valid:

- Short forms: `n`, `v`, `adj`, `adv`, `prep`, `conj`, `pron`, `art`.
- Long forms: `noun`, `verb`, `adjective`, `adverb`, `preposition`, `conjunction`, `pronoun`, `article`.
- spaCy tags: `NOUN`, `VERB`, `ADJ`, `ADV`, `ADP`, `CCONJ`, `SCONJ`, `PRON`, `DET`.

Any other value, such as `Noun`, `adp`, or `noun` with a leading space, is invalid.

## 7. Lockfile

`slopvac plugins sync` writes `.slopvac/packs.lock` sorted by plugin id. An entry pins `version`, `source` and `digest`, and records `kind` as `data` or `code`.

- The built-in core is never locked. A lock entry with the id `core` makes the lockfile invalid.

- A `git+https` source ends in `@` and a full 40-digit commit hash. A branch or tag name is invalid.
- The `digest` of a git source is the tree hash of the files at that commit, the hash a project directory gets. `sync` fetches only that commit and reads its files without a checkout, so hooks, filters and attributes in the repository do nothing. It refuses symbolic links and submodules, and it installs nothing when the tree hash differs from the lock.
- An `https://` release archive is a valid source. `http://` is invalid.
- A path source is relative to the project root.
- `signature` appears only when the host verified a minisign signature. It holds `key_id` and `not_before`, the `notBefore` time of the signed statement in UTC with a `Z` suffix. The schema rejects impossible months, days and clock values. The host rejects 29 February of a non-leap year when it parses the time. `plugins update` refuses an older `not_before` than the lockfile holds unless the user passes the downgrade flag (threat-model.md, section 5.3).
- The lockfile records no trust. `slopvac plugins trust` keeps trust apart, per content hash.
- A run recomputes the digest of each loaded plugin. A mismatch is an error with exit 2.

## 8. Commands

`slopvac plugins` is the one command group for data plugins and code plugins.

| Command | Purpose |
| --- | --- |
| `plugins list` | Lists the built-in core and each plugin that the config names, with its kind, version and lock state. Reads local files only. |
| `plugins search <query>` | Searches the plugin index (pack-index.md). |
| `plugins sync` | Installs what the lockfile pins and verifies each digest (section 7). |
| `plugins status` | Reports each locked plugin with its kind, install state and revocation state, and the trust state of each code plugin. |
| `plugins outdated` | Lists newer versions and digests from the pinned sources. |
| `plugins update` | Moves the lock to the newest versions that the config allows. Only `plugins trust` records trust. |
| `plugins trust <id>`, `plugins untrust <id>` | Records or removes trust for the content hash of a code plugin. |
| `plugins pin <namespace> <fingerprint> [--registry <host>]` | Pins a publisher key for one namespace on one registry host. |
| `plugins rotate <namespace> [--registry <host>]` | Accepts a key rotation for a pinned key and replaces the pin (threat-model.md, section 5.3). |
| `plugins new`, `validate`, `test`, `build`, `publish` | Author a plugin: create its files, check them against their schemas, run its tests, build the artifact and push it. |

- `plugins trust` and `plugins untrust` apply to code plugins only. A data plugin has no executable content and needs no trust, so either command on a data plugin exits 2 with the code `plugin-not-code`.
- A code plugin that the config lists starts only after `plugins trust` records its hash. Until then the run reports it with its trust code (`plugin-untrusted`, or `plugin-hash-mismatch` for changed files) and exits 2 (plugin-protocol.md, section 10).
- `--registry` names the registry host, such as `ghcr.io`, and defaults to `ghcr.io`. A pin binds a key to the pair of registry host and namespace, and only to that pair.
- `plugins pin` and `plugins rotate` refuse the namespaces `slopvac` and `core` with exit 2 (section 9).
- No command in the group prompts. Only `plugins trust` records trust, and only `plugins pin` and `plugins rotate` change a key pin.
- The index model, `plugins build` output and `plugins publish` are specified later.

The group replaces the command names that earlier drafts of this contract used:

| Earlier name | Name now |
| --- | --- |
| `slopvac packs search` | `slopvac plugins search` |
| `slopvac pack new`, `pack validate`, `pack test`, `pack publish` | `slopvac plugins new`, `plugins validate`, `plugins test`, `plugins publish` |
| `slopvac plugins trust --key <fingerprint>`, and the data-pack pinning command | `slopvac plugins pin <namespace> <fingerprint>` |
| The pinning command that confirms a rotation | `slopvac plugins rotate <namespace>` |
| (none) | `slopvac plugins list`, `plugins untrust`, `plugins build` |
| `plugins sync`, `status`, `outdated`, `update`, `trust` | Unchanged |
| `kind = "plugin"` in the manifest, the lockfile, the index and the doctor report | `kind = "code"` |
| `[[packs]]` and `[[plugins]]` in `slopvac.toml` | One `[[plugins]]` array |
| Pack, data pack, plugin pack | Plugin, data plugin, code plugin |

These names stay as they are: the file names `slopvac-pack.toml` and `.slopvac/packs.lock`, the schema names `pack-manifest`, `packs-lock` and `pack-index`, the `packs` keys inside the lockfile, the index and the doctor report, and the error and budget codes that contain `pack`.

## 9. Namespaces and the built-in core

A namespace is the owner segment of the repository that an artifact is published under (threat-model.md, section 5.1). Two names are special.

- **`slopvac`** is the only first-party namespace. An artifact in it verifies only against the compiled plugins key (section 10), on any registry. No user pin applies to it: `plugins pin` and `plugins rotate` refuse it. An index may list an entry under `slopvac`. Search shows such an entry as an unverified claim (pack-index.md, section 3).
- **`core`** is reserved as an id and as a namespace, and the reservation fails closed. A manifest, lock entry, index entry, config entry, key pin or signed statement that uses `core` as an id or a namespace is refused with exit 2. A reader refuses the whole document and does not skip the entry.

The built-in core is a data plugin compiled into the binary. Its version is the binary's version, and its rules carry the prefix `core:`. It is never in the lockfile, an index or a registry, so `sync`, `update` and `outdated` never fetch it. `plugins list` shows it with the kind `data` and the binary's version.

In `slopvac.toml` an entry in `[[plugins]]` with the id `core` is an error, and so is an id that two entries share (config.md, section 1.3.1).

## 10. Signing keys

The binary compiles in two public keys. Each has one job:

| Key | Signs | Verified by |
| --- | --- | --- |
| Release key (`keys/slopvac-release.pub`) | Release archives, and the revocation list | `self update`, and every read of the revocation list |
| Plugins key | First-party plugins (namespace `slopvac`), and the first-party plugin index | `plugins sync`, `update` and `outdated` for the `slopvac` namespace |

- The release key is the revocation root. It signs no plugin.
- The plugins key signs no binary and no revocation list. A leaked plugins key therefore cannot replace a binary or withdraw a revocation.
- The release key can revoke the plugins key: the revocation list names its key id. From then on no first-party plugin and no first-party index verifies. Both fail closed until a slopvac release ships a new plugins key.
- A binary without a usable compiled plugins key verifies nothing in the `slopvac` namespace.
- A compiled key changes only with a slopvac release.

Threat-model.md sections 5.2 and 5.3 give the key policy and the revocation list format.

## 11. First-party plugins

The slopvac repository holds the source of each first-party plugin under `plugins/<name>/`, where `<name>` is the plugin id. The directory has the layout of any plugin: `slopvac-pack.toml` at its root and the files that `provides` lists. A first-party plugin is in the `slopvac` namespace and is signed with the plugins key. A first-party code plugin still needs `plugins trust`, like any other code plugin. The release flow is specified later.

## 12. Conformance

| ID | Case | Expect |
| --- | --- | --- |
| K1 | Every file in `schemas/v3/conformance/<artifact>/valid/` | The schema accepts it. |
| K2 | Every file in `invalid/` | The schema rejects it at the pointer in `cases.toml`. |
| K3 | A manifest with `kind = "code"` and no `[plugin]` table, or a `data` manifest with one | Rejected. |
| K4 | A `[plugin]` table without `network`, or a hook without `id` | Rejected through `PluginDeclaration`. |
| K5 | A lock entry with an unpinned git source or a non-sha256 digest | Rejected. |
| K6 | The kind `plugin` in a manifest, a lock entry, an index entry or a doctor report | Rejected. The kinds are `data` and `code`. |
| K7 | `core` as a manifest id, a lock id, an index id or namespace, or a config plugin id | Rejected. |
| K8 | A config with a `[[packs]]` array | Rejected as an unknown key. |
| K9 | A config that lists one plugin id twice | The config validator rejects the second entry. JSON Schema cannot express this rule, so it has no schema case. |

The host behaviors in sections 3 to 11 (conflict notices, genre ties, digest mismatch, the command surface and key handling) get tests in the engine and plugin loader, which cite this file.
