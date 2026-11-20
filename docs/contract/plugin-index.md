# Slopvac 3.0 plugin index

Status: contract for 3.0. Threat model: `threat-model.md`, section 5.2 and the K1, N11 and N12 entries. Plugins: `plugins.md`.

The plugin index is a JSON document that lists data plugins and code plugins for discovery. `slopvac plugins search` reads it. The index tells a user which plugins exist and where to fetch them. It decides nothing about trust. The shape is [`plugin-index`](../../schemas/v3/plugin-index.schema.json), and the schema holds the field descriptions.

2.x has no plugins and no index. Each statement below is a 3.0 requirement. Sections 1 to 4 specify one index document: its shape, its search and its untrusted input. Sections 5 to 10 specify how a search combines the default first-party index with the extra indexes that a project configures: where each comes from, how results merge, the cache, and failures. The code that fetches indexes belongs to `slopvac-8l4.9.62`, so the four `index-` codes of section 9 are reserved until that code emits them (policies.md, section 3.5).

## 1. Document

The root holds `schema_version = 3` and `plugins`, an array with one entry per plugin. An entry lists `id`, `namespace`, `name`, `description`, `version`, `kind`, `slopvac_api`, `source` and `publisher_key`. The `id`, `kind`, `version` and `slopvac_api` fields have the meaning that they have in the plugin manifest, so `kind` is `data` or `code`. The `slopvac_api` field is the lowest extension API that the host must implement, so a host with a lower API shows the entry and cannot load the plugin.

- Unknown keys are errors, and keys with the prefix `x-` are ignored. A repeated key is an error.
- `slopvac_api` is a whole number from 1 to 4294967295. Major, minor and patch of `version` are at most 999999999. A whole number written with a fraction, such as `2.0`, counts as an integer, as in JSON Schema.
- Entries are sorted by `namespace` and then `id`. A pair of `namespace` and `id` appears once. The same `id` may appear under two namespaces.
- `core` is reserved (plugins.md, section 9). An entry with the id `core` or the namespace `core` makes the document invalid, and a reader refuses the whole document.
- A `source` is an `oci://` reference, an `https://` release archive or a `git+https://` URL pinned to a full commit hash. It carries no user-info. A path source and `http://` are invalid. The lock accepts a path source, because a lock belongs to a project. A public listing has no project root.
- The writer is canonical: fixed key order, two-space indent and a final newline. The same set of entries always gives the same bytes. The document holds no timestamp.

## 2. Build

A scheduled job scans plugins that follow the naming convention, reads each manifest, and builds the document from the manifest fields, the source it found and the signing key. The library function `build_index` does the build and sorts the result. It fails on an invalid entry, a repeated pair, or a canonical document over 32 MiB, so every index that it returns is one that a reader accepts. The job and its workflow belong to the code-quality epic and are not part of this contract.

The manifest has no display name. A build without a name uses the plugin `id`.

## 3. Search

The query is lowercased and split into tokens at every character that is not a letter or digit. An entry matches when every token is a substring of its `id`, `name` or `description`, compared in lowercase. A query with no token matches nothing.

Each token scores its best field: the whole `id` gives 100, the start of the `id` 60, elsewhere in the `id` 40, the `name` 30 and the `description` 10. The scores add up. A query whose tokens, joined by `-`, spell the `id` adds 50. Results sort by score, highest first, then by `namespace` and `id`. When a search reads several indexes, a further key breaks the remaining ties (section 7).

A result shows the namespace, id, version, kind, API, name, description, source and the publisher key with its fingerprint. Every one of these values is a claim by the index, and the result says so in an `unverified` line that names the pinning command. An entry in the first-party namespace `slopvac` gets a further line, `claims first-party namespace; not verified`. Only the plugins key compiled into the binary verifies the `slopvac` namespace (plugins.md, section 10), so a first-party identity comes from that signature check and not from the index.

## 4. Untrusted input

Threat model N11 and TM-41 apply.

- A reader refuses a document over 32 MiB before it parses it. It then streams: it builds no tree of the document, counts entries as it reads, stops at the first bad entry and skips `x-` values without storing them. It refuses more than 100000 entries. Strings have these limits, in characters: id and namespace 64, name 128, description 1024, source 2048.
- Text from the index is escaped before it reaches a terminal: C0 controls (newline and tab too), DEL, C1 controls, line and paragraph separators, and every default-ignorable or format (Cf) code point of Unicode 15. That covers bidirectional controls, zero-width characters, the soft hyphen, Hangul fillers, variation selectors and tag characters. Each becomes `\u{hex}`. The same applies to text in an error message. An error about a source never repeats the source, because a source may carry credentials.
- `publisher_key` is display-only (threat model section 5.2). A first install shows the fingerprint, and nothing pins the key until the user runs `slopvac plugins pin`. A pinned key wins over the index. No code path converts an index key into a verification key or a key policy, and no flag or environment variable does.

## 5. Sources of indexes

A search reads one or more indexes. Each has a name, which results show as `listed by <name>`.

- **The default index** is named `slopvac`. It is a signed OCI artifact at `oci://ghcr.io/slopvac/plugin-index`. The path lies outside `plugins/`, so the artifact cannot share an id with a plugin. A project can turn it off (section 6).
- **Extra indexes** come from the project config. Each has a unique `name` and a `source`. The source is an `oci://` reference or an `https://` URL with no user-info. An `http://` source, a path source and a `git+https://` source are invalid. The default name `slopvac` is not available to an extra index.
- Every index document has the shape of section 1. An extra index may serve a document that lists any namespace, `slopvac` included (section 7 marks such an entry).
- Order is significant. The default index comes first when it is on, and the extra indexes follow in the order of the config. Section 7 uses this order to break ties.

### 5.1 The default index is signed

The default index uses the plugin statement format of [threat-model.md](threat-model.md), section 5.1. Its statement holds the id `plugin-index`, the namespace `slopvac` and the version `1.0.<run_number>`, where `<run_number>` is the run number of the workflow that publishes it, so the version grows with each publication. It carries the artifact type of an index, not of a plugin. One content layer holds the canonical index document of section 1.

- The reader verifies the artifact with `sign::verify` against the plugins key compiled into the binary (plugins.md, section 10). Nothing else supplies a key for it: no pin, no config value and no flag.
- A verifier that expects an index refuses an artifact of a plugin type, and a verifier that expects a plugin refuses an artifact of the index type. A plugin with the id `plugin-index` therefore cannot pass as the index.
- A binary without a usable plugins key, or one whose plugins key the revocation list names, verifies no default index. The default index fails with `index-signature-invalid` and the search uses its cache (section 9). Policy for the key is the one of plugins.md, section 10.
- Statement version and `notBefore` give the rollback floor of section 8.

### 5.2 Extra indexes are unsigned

Third-party indexes are unsigned in 3.0. Slopvac checks no signature on one, and a signature served next to one is ignored. Signing for third-party indexes is out of scope. An extra index relies on transport security alone (HTTPS, or the registry's TLS), on the caps of section 4 and on the rule that an index never grants trust (section 10).

## 6. Configuration

The config sets the indexes (config.md, section 1.3.2).

```toml
[index]
default = true

[[index.extra]]
name = "acme"
source = "https://plugins.acme.example/index.json"
```

- `default` is a boolean and defaults to `true`. `false` removes the default index from the search.
- `[[index.extra]]` is a list. `name` is a lowercase slug of the form of a plugin id, other than `core` and `slopvac`, and each name appears once. `source` follows section 5.
- The table is project-scoped. Only a project config (with its `extends` files) sets it. There is no user-level config layer, and no environment variable, document front matter, override or inline directive sets it.
- `slopvac plugins search --index <name>` limits one search to the index with that name, `slopvac` for the default. A name that no configured index has exits 2 with `index-unknown`. The flag selects among configured indexes and adds none.
- Outside a project, or in a project without `[index]`, the search reads the default index alone.

## 7. Merging results

A search reads every selected index and shows the union of their entries. Each index passes through the checks of section 4 on its own, so a bad entry in one index refuses that index, not the others.

- **Listing.** A listing is an entry with the same `namespace`, `id`, `source` and publisher key fingerprint. Two entries that agree on these four values are the same listing, whatever else they hold. The result shows the listing once, with every index that lists it: `listed by slopvac, acme`. It shows the other fields, such as the version and the description, from the first index in the order of section 5.
- **Conflicting listings.** Entries with the same `namespace` and `id` that differ in `source` or in the key fingerprint are different listings. The search shows each one as its own result and marks all of them `conflicting listings`. It never merges them and never picks one. Each shows its own source and key fingerprint, so a user can tell them apart.
- **Not in the `slopvac` index.** A listing in the namespace `slopvac` that the default index does not list gets the further line `not in the slopvac index`. This covers an entry that only an extra index lists, a conflicting listing from an extra index, and every `slopvac` entry when the default index is off. The existing line `claims first-party namespace; not verified` (section 3) stays on every `slopvac` entry.
- **Ranking.** Score, then `namespace`, then `id`, as in section 3. A remaining tie, which only conflicting listings can produce, goes to the listing whose first index comes earlier in the order of section 5. Equal results are then ordered by `source`.
- **Display only.** Every field of a listing stays a claim of the index that lists it. The result says `listed by <name>` and never says that an index vouches for a plugin.

## 8. Cache and rollback floor

Every index has its own cache file in the per-user cache directory (policies.md, section 1.8), named after the index. The file holds the verified document, the index source and the time of the fetch. A change of `source` for a name discards that name's cache.

- A successful fetch replaces the cache file atomically. A fetch that fails any check never touches the cache. Slopvac never reads a document from the cache that it did not verify and accept first.
- **Floor for the default index.** The cache file of the default index also holds the `version` and `notBefore` of the verified statement. A later fetch whose version is lower, or whose `notBefore` is earlier, is a rollback. The search ignores that document, reports `index-rollback` and uses the cache. The floor rises with each accepted fetch and never falls through a flag. A user who deletes the cache file drops the floor with it, as with the lockfile floor (threat-model.md, section 5.3).
- **No floor for extra indexes.** An unsigned document gives no value that can serve as a floor. The cache of an extra index holds the last document that passed the checks of section 4.
- Online, a search fetches each selected index. The cache has no expiry and serves only as the fallback of section 9.

## 9. Failure and offline behaviour

The exit codes are those of policies.md, section 3.5. A search that could not read every selected index is incomplete and exits 2, because the results cannot show that no plugin is missing. The four `index-` codes in this table are reserved and nothing emits them yet.

| Situation | Behaviour |
|---|---|
| Online, an index cannot be fetched: a network error, an HTTP error, a document over the cap, or a document that fails the checks of section 4 | The search shows the cached results of that index, marked as cached with the time of the fetch. It prints one notice with the code `index-fetch-failed` that names the index and not its source. It reads the other indexes as usual. Exit 2. |
| Online, the default index fails the signature check | The same, with the code `index-signature-invalid`. Exit 2. |
| Online, the default index is a rollback | The same, with the code `index-rollback`. Exit 2. |
| Online, a failed index has no cache | The notice stands, and the index adds no results. Exit 2. |
| Offline (`--offline`, `SLOPVAC_OFFLINE=1` or `[network] offline = true`) | The search opens no connection and reads the caches only, marked as cached. An index with no cache adds no results and gets the code `offline-network-needed` (policies.md, section 1.3). Exit 2 when any selected index has none, and exit 0 otherwise. |
| Offline, no selected index has a cache | No results. The error `offline-network-needed` and exit 2. |
| `--index` names an index that is not configured | The error `index-unknown` and exit 2. |
| Every selected index is read and no entry matches | No results. Exit 0. |

A notice or error never repeats a source or text from the index (section 4).

## 10. An index never grants trust

Adding an index grants nothing. An entry in an index, in any index, is a claim.

- An index adds no key to the key policy, pins no key and relaxes no verification. The publisher key of an entry is for display (section 4). A pinned key wins over every index, and the compiled keys win for `slopvac` on any registry.
- A pin binds to a pair of registry host and namespace (plugins.md, section 8). It does not depend on which index listed the plugin, so a second index cannot rebind a namespace.
- Listing a plugin in a search result installs nothing and writes no config. A plugin enters a project only through `[[plugins]]` and `plugins sync`, and a code plugin needs `plugins trust`.
- An extra index that lists the namespace `slopvac` gets no first-party standing. Only an artifact that verifies against the compiled plugins key is first-party (plugins.md, section 9).

## 11. Conformance

| ID | Case | Expect |
| --- | --- | --- |
| I1 | Every file in `schemas/v3/conformance/plugin-index/valid/` | The schema and the Rust reader accept it. |
| I2 | Every file in `invalid/` | The schema rejects it at the pointer in `cases.toml`. The reader rejects it at that pointer or below it. |
| I3 | A document over 32 MiB | Refused before parsing. |
| I4 | A description with escape sequences, newlines and bidi controls | Printed escaped, on one line. |
| I5 | The same listings in two orders | The same bytes. |
| I6 | A query matching an exact id, an id prefix and a description | Ranked as in section 3. |
| I7 | An entry in the `slopvac` namespace | Rendered with the claim warning and the `unverified` line. |
| I9 | An entry with the id `core` or the namespace `core` | The schema and the reader refuse the document. |
| I8 | Every string field at its cap and one character past it | Accepted, then refused. |
| I10 | The default index artifact with a statement signed by the compiled plugins key | Verified. The id is `plugin-index`, the namespace `slopvac` and the version `1.0.<run_number>`. |
| I11 | A default index signed by another key, and a plugin artifact presented as the index | Both refused with `index-signature-invalid`. The search uses the cache. Exit 2. |
| I12 | A default index with a lower version, and one with an earlier `notBefore`, against a cached floor | Each is `index-rollback`. The cache stays and is used. Exit 2. |
| I13 | A config with `[index] default = false` and two `[[index.extra]]` entries | The search reads the two extras in config order and not the default. |
| I14 | `--index acme`, then `--index missing` | The first reads only `acme`. The second exits 2 with `index-unknown`. |
| I15 | Two indexes that list the same namespace, id, source and key | One result, `listed by slopvac, acme`. |
| I16 | Two indexes that list one namespace and id with different sources, and again with different keys | Two results in each case, all marked `conflicting listings`. Neither is dropped. |
| I17 | A `slopvac` entry from an extra index only, and one when `default = false` | Marked `not in the slopvac index`, with the first-party claim line. |
| I18 | Two conflicting listings with equal scores, from the default index and an extra index | The default index's listing comes first. With two extras, the one listed first in the config comes first. |
| I19 | A pin for the namespace, and an extra index that lists a different key for a plugin in it | Verification uses the pin. The displayed key changes nothing. |
| I20 | A fetch error, an over-cap document and a document with a bad entry, each online with a cache | The cached results show, marked as cached. One `index-fetch-failed` notice names the index. Exit 2. |
| I21 | Offline with a cache for every selected index, then with one missing, then with none | Exit 0 with cached results. Exit 2 with results and `offline-network-needed` for the missing one. Exit 2, no results and `offline-network-needed`. |
| I22 | An unsigned extra index that serves a signature next to its document | The signature is ignored. The document is read under the checks of section 4. |
| I23 | An extra `source` that is `http://`, a path, `git+https://` or holds user-info | The config is invalid. No fetch starts. |
| I24 | A cached extra index after its `source` changes in the config | The old cache is discarded and not shown. |

The schema harness runs I1 and I2 in `tests/contract/test_schemas.py`. The Rust tests in `crates/slopvac-plugins/tests/index.rs` run I3 to I9 and repeat I1 and I2 against the reader. The config parts of I13 and I23 run in `crates/slopvac-config` (`schemas/v3/conformance/config/`). I10 to I12 and I14 to I24 wait for the index fetch code (`slopvac-8l4.9.62`), which adds their tests.
