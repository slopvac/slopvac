# Documentation site

The reference documentation is generated, so it cannot drift from the code and data it describes. This page records where it is published and how to build it.

## Hosting

The site is served from GitHub Pages at `https://srobroek.github.io/slopvac/`. The address is one constant, `BASE_URL`, in `scripts/docs/build_site.py`. The schema `$id` values under `schemas/v3/` use the same base, and the builder fails when one does not.

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

Run `just docs` to write the Markdown source tree to `build/site-src`. Run `just docs-check` to build it to a temporary directory and fail on a broken link or a schema with the wrong `$id`.

The styles, genres and pack pages are placeholders until the style, genre and pack code exists. The rules page generator reads the Python rule data until the Rust rule loader replaces it.
