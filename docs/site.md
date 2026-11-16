# Documentation site

The reference documentation is generated, so it cannot drift from the code and data it describes. This page records where it is published and how to build it.

## Hosting

The site is served from GitHub Pages. The address, including the base path, is one value: `site_url` in `docs/site.toml`. A custom domain later changes that value only. No generator or test writes the host name.

The committed schemas under `schemas/v3/` carry an author-side `$id` prefix. Every schema must use the same prefix and end its `$id` with its own file name. Publishing rewrites that prefix, in `$id` and in cross-schema `$ref` values, to `site_url` followed by `schemas/v3/`. A published schema file is therefore at the path that its `$id` names. `just docs-check` and the test suite fail when a published `$id` does not equal the site URL plus the file path.

The site generator is Starlight (Astro), in `site/`. The build first writes plain Markdown with no framework front matter into a neutral directory. `--starlight site` then adapts that tree: the title moves into front matter, and links become route-relative so that they work under any base path. Generated content goes to `site/src/content/docs/v3/` and raw schemas to `site/public/schemas/v3/`. Both are ignored by git.

The base path comes from `site_url`. `DOCS_BASE` overrides it for a preview build.

## URL layout

Everything below the API major lives under `/v3/`, so a future major version gets its own tree.

| Path | Content |
| --- | --- |
| `/v3/rules/<category>/<rule>/` | One page per rule. Outputs link here. |
| `/v3/rules/<category>/` | Category index. |
| `/v3/styles/`, `/v3/genres/` | Style presets and genres. |
| `/v3/plugins/protocol/` | Plugin protocol v1 reference. |
| `/v3/plugins/packs/` | Pack reference. |
| `/schemas/v3/<name>.schema.json` | Published JSON Schema files. |
| `/api/` | Rustdoc for the workspace crates. |

## Build

Run `just docs` to write the Markdown source tree to `build/site-src`. Nothing generated is committed. CI generates the pages, lints that Markdown with slopvac, builds the site from it, and publishes that same artifact.

Run `just docs-check` locally after `just docs` to check that `build/site-src` matches a fresh build. It builds the tree in memory, compares paths and contents, and reports each missing, changed, or obsolete file. It changes nothing. It is a local consistency check, not protection against drift in committed output, because no generated output is committed. Both commands fail on a broken link or a schema with the wrong `$id`.

Run `just docs-site` to build the Starlight site in `site/dist` and copy the rustdoc output to `site/dist/api`. It needs Node from `mise.toml`. Dependabot watches `site/package.json`.

GitHub Pages publishes the site from `.github/workflows/docs-deploy.yml` on pushes to `main`. It calls `.github/workflows/docs-site.yml`, which also runs from `test.yml` on pull requests when the `docs-site` path class matches. A pull request builds everything and deploys nothing. The build steps are:

1. Run the generator tests.
2. Generate the pages into a temporary directory.
3. Lint the generated Markdown with slopvac. `docs/site-lint.toml` holds the gate settings.
4. Build the Starlight site.
5. Build rustdoc with warnings denied and copy it to `dist/api`.
6. Check internal links with lychee.
7. On `main` only, upload the site as the Pages artifact and deploy that same artifact.

The repository publishes with the GitHub Actions source. Changing the source in the repository settings stops deployment.

The styles, genres, and pack pages are placeholders until the style, genre, and pack code exists. The rules page generator reads the Python rule data until the Rust rule loader replaces it.
