# Slopvac 3.0 data packs

Status: contract for 3.0. Bead `slopvac-8l4.2.4`. Plan: [slopvac-3.md](../design/slopvac-3.md). Decision: [0001-slopvac-3-architecture.md](../design/0001-slopvac-3-architecture.md), decision 9. Plugin protocol: [plugin-protocol.md](plugin-protocol.md).

A pack is a folder or an OCI artifact with a `slopvac-pack.toml` manifest. A data pack holds files that declare rules, styles, genres, terms and taggers. It holds no code. A plugin pack adds a command for the host to run. Each file type has a schema in `schemas/v3/`, and the schema holds the field descriptions.

2.x has no packs. Its rules, genres and profiles are built into the Python package. Each statement below is a 3.0 requirement.

## 1. Files

| File | Schema | Notes |
| --- | --- | --- |
| `slopvac-pack.toml` | [`pack-manifest`](../../schemas/v3/pack-manifest.schema.json) | One per pack, at the pack root. |
| category files | `category` | Owned by `slopvac-8l4.2.3`. |
| style files | [`style`](../../schemas/v3/style.schema.json) | Ordered presets. |
| genre files | [`genre`](../../schemas/v3/genre.schema.json) | Genres with detection signals. |
| tagger files | [`tagger`](../../schemas/v3/tagger.schema.json) | Declarative context taggers. |
| terms files | [`terms`](../../schemas/v3/terms.schema.json) | Also the format of `.slopvac/terms.toml`. |
| blocklist files | [`blocklist`](../../schemas/v3/blocklist.schema.json) | YAML, TOML or JSON (`.yml`, `.yaml`, `.toml` or `.json`; the suffix picks the format). The opt-in vocabulary check reads them. |
| `.slopvac/packs.lock` | [`packs-lock`](../../schemas/v3/packs-lock.schema.json) | Written by `slopvac plugins sync`. |

Every file carries `schema_version = 3`. Unknown keys are errors, and keys with the prefix `x-` are ignored. The manifest lists its files under `provides`. A path that leaves the pack, by an absolute path, a drive letter, a backslash or a `..` segment, is invalid.

## 2. Manifest

- `slopvac_api` is the extension API the pack targets, apart from `schema_version`. A host that implements an earlier API exits 2 and does not load the pack.
- `kind = "data"` forbids the `[plugin]` table. `kind = "plugin"` requires it.
- `requires` lists other packs with a version requirement. The host resolves them first and reports a cycle as an error.
- The `[plugin]` table embeds the [`PluginDeclaration`](../../schemas/v3/plugin-protocol.schema.json#/$defs/PluginDeclaration) fields `network`, `hooks` and `features` flat, next to `command`, `args` and `env`. The host reads the declaration from the same bytes that `slopvac plugins trust` hashes (plugin-protocol.md section 4.3). A change to any field in the table changes the hash. The manifest closes the hook entries and the `features` table, so they take only the declared keys and `x-` keys.
- Rule ids take the pack id as their prefix: `<pack>:<category>.<rule>`. The pack id `core` belongs to the built-in pack.

## 3. Styles

A style sets levels for categories and rules, and may bring terms. A style has no id: its name is the file name without the extension, qualified by the pack id. A category entry is a bare level or a table with `severity`, `minimum_severity`, `max_per_100_words` and `weight`. `max_per_100_words = "none"` clears a budget that a lower layer set. 2.x had no way to unset a field, and 3.0 adds this one because a profile override must be able to drop a budget. The host applies `extends` in the order listed. Then it applies the style. When a config lists two or more styles, the last one wins. A rule entry beats a category entry in the same style.

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

The two flags cannot combine. The project file `.slopvac/terms.toml` and the terms files from packs and styles merge. A term from the project file wins over a pack term with the same text.

A blocklist file holds an `entries` list (`[[entries]]` in TOML) with `word`, `pos`, `reason` and an optional `replacement`. The loader trims `word` and folds it to lowercase, loads short forms of `pos` as their long form, and rejects a repeated word and part-of-speech pair. An entry with no reason is invalid. A style or config names blocklists under `terms.blocklists`.

## 7. Lockfile

`slopvac plugins sync` writes `.slopvac/packs.lock` sorted by pack id. An entry pins `version`, `source` and `digest`.

- A `git+https` source ends in `@` and a full 40-digit commit hash. A branch or tag name is invalid.
- An `https://` release archive is a valid source. `http://` is invalid.
- A path source is relative to the project root.
- `signature` appears only when the host verified a minisign signature. It holds `key_id` and `not_before`, the `notBefore` time of the signed statement in UTC with a `Z` suffix. The schema rejects impossible months, days and clock values. The host rejects 29 February of a non-leap year when it parses the time. `plugins update` refuses an older `not_before` than the lockfile holds unless the user passes the downgrade flag (threat-model.md, section 5.3).
- The lockfile records no trust. `slopvac plugins trust` keeps trust apart, per content hash.
- A run recomputes the digest of each loaded pack. A mismatch is an error with exit 2.

## 8. Conformance

| ID | Case | Expect |
| --- | --- | --- |
| K1 | Every file in `schemas/v3/conformance/<artifact>/valid/` | The schema accepts it. |
| K2 | Every file in `invalid/` | The schema rejects it at the pointer in `cases.toml`. |
| K3 | A manifest with `kind = "plugin"` and no `[plugin]` table, or a `data` manifest with one | Rejected. |
| K4 | A `[plugin]` table without `network`, or a hook without `id` | Rejected through `PluginDeclaration`. |
| K5 | A lock entry with an unpinned git source or a non-sha256 digest | Rejected. |

The host behaviors in sections 3 to 7 (conflict notices, genre ties, digest mismatch) get tests in the engine and pack-loader beads, which cite this file.
