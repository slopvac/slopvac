# Documentation site

The reference documentation is generated, so it cannot drift from the code and data it describes. This page records where it is published and how to build it.

## Hosting

The site is served from GitHub Pages. The address, including the base path, is one value: `site_url` in `docs/site.toml`. A custom domain later changes that value only. No generator or test writes the host name.

The committed schemas under `schemas/v3/` carry an author-side `$id` prefix. Every schema must use the same prefix and end its `$id` with its own file name. Publishing rewrites that prefix, in `$id` and in cross-schema `$ref` values, to `site_url` followed by `schemas/v3/`. A published schema file is therefore at the path that its `$id` names. `just docs-check` and the test suite fail when a published `$id` does not equal the site URL plus the file path.

The site generator is Starlight. The build first writes plain Markdown with no framework front matter into a neutral directory. A separate step adapts that tree for Starlight.

## URL layout

Everything below the API major lives under `/v3/`, so a future major version gets its own tree.

| Path | Content |
| --- | --- |
| `/v3/rules/<category>/<rule>.html` | One page per rule. Outputs link here. |
| `/v3/rules/<category>/` | Category index. |
| `/v3/styles/`, `/v3/genres/` | Style presets and genres. |
| `/v3/plugins/protocol.html` | Plugin protocol v1 reference. |
| `/v3/plugins/packs.html` | Pack reference. |
| `/schemas/v3/<name>.schema.json` | Published JSON Schema files. |
| `/api/` | Rustdoc for the workspace crates. |

## Build

Run `just docs` to write the Markdown source tree to `build/site-src`. Run `just docs-check` after it. The check builds the tree in memory, compares paths and contents with `build/site-src`, and reports each missing, changed, or obsolete file. It changes nothing. Both commands fail on a broken link or a schema with the wrong `$id`.

The styles, genres, and pack pages are placeholders until the style, genre, and pack code exists. The rules page generator reads the Python rule data until the Rust rule loader replaces it.
