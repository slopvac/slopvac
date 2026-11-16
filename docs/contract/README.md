# Slopvac 3.0 contract

This directory holds the specs for the 3.0 contract. The schemas live in `schemas/v3/`.
The schemas and a conformance corpus are the lasting golden. At cutover, schemas
generated from the Rust types must accept and reject the same corpus. They need not
match the hand-written text.

## Layout

- `schemas/v3/<artifact>.schema.json` uses kebab-case names. The artifacts are:
  `common`, `lint-report`, `rule`, `category`, `config`, `frontmatter`,
  `pack-manifest`, `style`, `genre`, `tagger`, `blocklist`, `terms`, `packs-lock`,
  `pack-index` and `plugin-protocol`.
- `schemas/v3/conformance/<artifact>/valid/` and `invalid/` hold instances in native
  TOML, YAML or JSON.
- `invalid/cases.toml` maps each invalid file to a JSON pointer where it must fail.
- `docs/contract/<topic>.md` holds one spec per bead. A spec states semantics and links
  `$defs` by pointer. Field descriptions live in the schema `description`, not in tables.

## Specs

- [pack-index.md](pack-index.md) specifies the pack index and `packs search`.
- [policies.md](policies.md) sets the network, accessibility and API lifecycle policies.
- [packs.md](packs.md) specifies data packs, styles, genres, taggers, terms and the lockfile.
- [profiles.md](profiles.md) maps 2.x profiles and tiers onto the 3.0 model.
- [compat/](compat/) holds the 2.x to 3.0 compatibility matrix and its dispositions.

## Conventions

- Schemas use the JSON Schema 2020-12 dialect.
- `$id` is `https://srobroek.github.io/slopvac/schemas/v3/<file>`. It never changes.
- Every root instance carries `schema_version` set to 3. Lint JSON jumps from 1 to 3.
  Minor changes are additive only. `slopvac_api` in pack manifests is a separate version.
- Shared shapes live in `common.schema.json`. A change to a shared def links the owner
  bead named in that def's `description`.
- A schema without a root shape, such as `common`, takes instances named
  `<Def>--<name>.<ext>`. The harness validates them against `#/$defs/<Def>`.

## Add an artifact

Add the schema file and its conformance files. The harness in
`tests/contract/test_schemas.py` discovers them, so the harness needs no code change. Run
`just contract`.

## Prose

Spec files here are deliverable prose. Lint each with
`uvx --from ./packages/slopvac-lint slopvac lint <file> --format json`. Exit 0 passes.
