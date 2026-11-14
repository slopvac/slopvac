# Slopvac 3.0 pack index

Status: contract for 3.0. Bead `slopvac-8l4.9.8`. Threat model: `threat-model.md`, section 5.2 and the K1, N11 and N12 entries. Packs: `packs.md`.

The pack index is a JSON document that lists packs for discovery. `slopvac packs search` reads it. The index tells a user which packs exist and where to fetch them. It decides nothing about trust. The shape is [`pack-index`](../../schemas/v3/pack-index.schema.json), and the schema holds the field descriptions.

2.x has no packs and no index. Each statement below is a 3.0 requirement.

## 1. Document

The root holds `schema_version = 3` and `packs`, an array with one entry per pack. An entry lists `id`, `namespace`, `name`, `description`, `version`, `kind`, `slopvac_api`, `source` and `publisher_key`. The `id`, `kind`, `version` and `slopvac_api` fields have the meaning that they have in the pack manifest. The `slopvac_api` field is the lowest extension API that the host must implement, so a host with a lower API shows the entry and cannot load the pack.

- Unknown keys are errors, and keys with the prefix `x-` are ignored.
- Entries are sorted by `namespace` and then `id`. A pair of `namespace` and `id` appears once. The same `id` may appear under two namespaces.
- A `source` is an `oci://` reference, an `https://` release archive or a `git+https://` URL pinned to a full commit hash. It carries no user-info. A path source and `http://` are invalid. The lock accepts a path source, because a lock belongs to a project. A public listing has no project root.
- The writer is canonical: fixed key order, two-space indent and a final newline. The same set of entries always gives the same bytes. The document holds no timestamp.

## 2. Build

A scheduled job scans packs that follow the naming convention, reads each manifest, and builds the document from the manifest fields, the source it found and the signing key. The library function `build_index` does the build and sorts the result. It fails on an invalid entry or a repeated pair. The job and its workflow belong to the code-quality epic and are not part of this contract.

The manifest has no display name. A build without a name uses the pack `id`.

## 3. Search

The query is lowercased and split into tokens at every character that is not a letter or digit. An entry matches when every token is a substring of its `id`, `name` or `description`, compared in lowercase. A query with no token matches nothing.

Each token scores its best field: the whole `id` gives 100, the start of the `id` 60, elsewhere in the `id` 40, the `name` 30 and the `description` 10. The scores add up. A query whose tokens, joined by `-`, spell the `id` adds 50. Results sort by score, highest first, then by `namespace` and `id`.

A result shows the namespace, id, version, kind, API, name, description, source and the publisher key with its fingerprint.

## 4. Untrusted input

Threat model N11 and TM-41 apply.

- A reader refuses a document over 32 MiB before it parses it. It also refuses more than 100000 entries. Strings have these limits, in characters: id and namespace 64, name 128, description 1024, source 2048.
- Text from the index is escaped before it reaches a terminal: C0 controls (newline and tab too), DEL, C1 controls, bidirectional controls, zero-width characters, line and paragraph separators and tag characters. Each becomes `\u{hex}`. The same applies to text in an error message.
- `publisher_key` is display-only (threat model section 5.2). A first install shows the fingerprint, and nothing pins the key until the user runs the pinning command. A pinned key wins over the index. No code path converts an index key into a verification key or a key policy, and no flag or environment variable does.

## 5. Conformance

| ID | Case | Expect |
| --- | --- | --- |
| I1 | Every file in `schemas/v3/conformance/pack-index/valid/` | The schema and the Rust reader accept it. |
| I2 | Every file in `invalid/` | The schema rejects it at the pointer in `cases.toml`. The reader rejects it at that pointer or below it. |
| I3 | A document over 32 MiB | Refused before parsing. |
| I4 | A description with escape sequences, newlines and bidi controls | Printed escaped, on one line. |
| I5 | The same listings in two orders | The same bytes. |
| I6 | A query matching an exact id, an id prefix and a description | Ranked as in section 3. |

The schema harness runs I1 and I2 in `tests/contract/test_schemas.py`. The Rust tests in `crates/slopvac-plugins/tests/index.rs` run the rest and repeat I1 and I2 against the reader.
