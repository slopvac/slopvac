# Git hooks

Slopvac ships hooks for pre-commit and prek, and the same command works in lefthook and hk. All of them run the `slopvac` binary on staged prose files. The hooks use `language: system`, so they need no Python environment and no build step.

## Install the binary

Put `slopvac` on `PATH` before the first commit. Any install method works:

```sh
brew install slopvac/tap/slopvac
uv tool install slopvac
```

The shell installer attached to each GitHub release also works.

## pre-commit and prek

```yaml
repos:
  - repo: https://github.com/slopvac/slopvac
    rev: v3.0.0
    hooks:
      - id: slopvac
```

prek reads the same file, so `prek run` and `pre-commit run` behave the same.

| Hook id | Command |
| --- | --- |
| `slopvac` | `slopvac lint <files>` |

The hook has no pinned tool version. The `rev` selects the hook definition, and the binary on `PATH` selects the rules. Pin the binary version in your tool manager, for example with `mise`.

`--profile` is removed in 3.0, and the CLI has no per-command style option. The `slopvac-strict` hook is removed with it. To keep the 2.x strict gate, add both keys to `slopvac.toml`: `profile = "strict"` sets the thresholds (density 1.5, minimum score 85), and `styles = ["core:legacy-strict"]` sets the 2.x category and rule levels. The style alone leaves the `normal` thresholds (density 3.0, minimum score 70). `slopvac migrate --preserve-behavior` writes both keys from the profile in your 2.x config, which is `normal` when the config sets none. A profile and a style apply to the whole run, so to gate only reference material, keep a second config file with both keys and pass it to the `slopvac` hook with `args` and `files`:

```toml
schema_version = 3
profile = "strict"
styles = ["core:legacy-strict"]
```

```yaml
- id: slopvac
  files: ^docs/reference/
  args: [--config, slopvac-strict.toml]
```

## lefthook

```yaml
pre-commit:
  commands:
    slopvac:
      glob: "*.{md,rst,txt,html}"
      run: slopvac lint {staged_files}
```

## hk

```pkl
amends "package://github.com/jdx/hk/releases/download/v2.12.0/hk@2.12.0#/Config.pkl"

steps {
  ["slopvac"] {
    glob = List("*.md", "*.rst", "*.txt", "*.html")
    check = "slopvac lint {{files}}"
  }
}
```

## Exit codes

Exit 0 passes, 1 fails, and 2 means the check was incomplete. Each runner treats a nonzero exit as a failed hook, so an incomplete check blocks the commit.
