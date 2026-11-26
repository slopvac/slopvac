# MegaLinter and Trunk

Slopvac ships a MegaLinter plugin descriptor and a Trunk linter definition. Both install the slopvac wheel from PyPI, run `slopvac lint` on prose files, and fail on exit 1 and exit 2. The files live in `integrations/` in this repository.

Both integrations pin version 3.0.0, the first release with the `slopvac lint` command they call. Until that release is on PyPI, neither descriptor can install slopvac, so neither has run end to end. The descriptor tests in `tests/test_integration_descriptors.py` are not integration acceptance. They validate the descriptor structure, check the MegaLinter descriptor against the upstream schema, and keep the version pins in step. `test_descriptor_command_is_accepted_by_the_cli` runs the built slopvac CLI once per descriptor with the arguments that descriptor builds, and expects exit 0 or 1 and a SARIF report. `test_lint_help_lists_every_descriptor_flag` runs only `slopvac lint --help` and checks that three flag names, `--format`, `--out` and `--config`, appear in the output. No test runs MegaLinter or Trunk, installs the wheel, or checks how either tool reads the exit code or the SARIF report. Real runs of both tools wait for 3.0.0 on PyPI and are tracked in `slopvac-8l4.13.47`.

## Windows

Slopvac 3.0 has no native Windows build. On Windows, run MegaLinter and Trunk inside WSL, where both integrations use the Linux build. The Trunk definition lists `supported_platforms: [linux, macos]` ([definitions](https://docs.trunk.io/code-quality/overview/getting-started/configuration/lint/definitions)), so Trunk on native Windows skips the linter. The project intends to add native Windows support in a 3.x release.

## Exit codes

| Exit | Meaning | MegaLinter | Trunk |
| --- | --- | --- | --- |
| 0 | The files pass. | Success. | Success. |
| 1 | The check found problems. | Error. | Findings from the SARIF report. |
| 2 | The check was incomplete. | Error. | Tool failure. |

MegaLinter counts any nonzero exit as an error. Trunk accepts only the exit codes in `success_codes`, and the definition lists 0 and 1. In MegaLinter, leave `DISABLE_ERRORS` and `DISABLE_ERRORS_IF_LESS_THAN` unset for this linter, including the `PROSE_SLOPVAC_` prefixed forms, because they turn a failed or incomplete check into a pass.

## MegaLinter

The descriptor is `integrations/megalinter/mega-linter-plugin-slopvac/slopvac.megalinter-descriptor.yml`. MegaLinter loads plugins from a URL in the `PLUGINS` list of `.mega-linter.yml`. The URL must contain `/mega-linter-plugin-` and the file name must end in `.megalinter-descriptor.yml` ([MegaLinter plugins](https://megalinter.io/latest/plugins/)). Pin the URL to a release tag:

```yaml
PLUGINS:
  - https://raw.githubusercontent.com/slopvac/slopvac/v3.0.0/integrations/megalinter/mega-linter-plugin-slopvac/slopvac.megalinter-descriptor.yml
```

The linter is `PROSE_SLOPVAC`. It runs `slopvac lint` on `.md`, `.markdown`, `.rst`, `.txt` and `.html` files, and passes `slopvac.toml` as `--config` when the repository has one. When MegaLinter's SARIF reporter is on, it adds `--format sarif --out <file>` and counts findings from the report.

MegaLinter plugins can install only through `RUN` instructions in `install.dockerfile` ([plugin limitations](https://megalinter.io/latest/plugins/#limitations)). The descriptor creates a virtual environment, installs `slopvac==3.0.0` from a binary wheel, and links the executable into `/usr/local/bin`. The `--only-binary` option stops pip from building from source. The musllinux x86_64 wheel from `wheels.yml` serves the Alpine-based image, so the descriptor supports `linux/amd64` only.

## Trunk

The definition is `integrations/trunk/linters/slopvac/plugin.yaml`. It uses the layout of [trunk-io/plugins](https://github.com/trunk-io/plugins), `linters/<name>/plugin.yaml`. Trunk installs the `slopvac` package with its managed Python runtime ([linter dependencies](https://docs.trunk.io/code-quality/overview/getting-started/configuration/lint/dependencies)) and parses the SARIF report that `--out` writes ([output types](https://docs.trunk.io/code-quality/overview/getting-started/configuration/lint/output)). Trunk recommends SARIF over other output types.

The definition declares its own file type, `slopvac-prose`, for `md`, `markdown`, `rst`, `txt` and `html`, so it does not depend on Trunk's built-in file type names.

A Trunk plugin source needs a root `plugin.yaml`, which this repository does not have. Until the definition is merged upstream, copy the `tools` and `lint` sections into your `.trunk/trunk.yaml` or into your own plugin repository, then enable the linter:

```yaml
lint:
  enabled:
    - slopvac@3.0.0
```

## Update the pin

Each release changes three version strings: `slopvac==` in the MegaLinter descriptor and the two `known_good_version` fields in the Trunk definition. A test fails when they disagree. They are not in release-please `extra-files`, because the root version is still 2.x and a marker would rewrite the 3.0.0 pin to a 2.x number at the next 2.x release.

## Upstream contributions

After the 3.0 release, the maintainers plan two upstream pull requests: the MegaLinter descriptor, which would move into the `SPELL` descriptor next to Vale and proselint, and the Trunk definition in `trunk-io/plugins`.
