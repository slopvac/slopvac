# Git hooks

Slopvac ships hooks for pre-commit and prek, and the same command works in lefthook and hk. All of them run the `slopvac` binary on staged prose files. The hooks use `language: system`, so they need no Python environment and no build step.

## Install the binary

Put `slopvac` on `PATH` before the first commit. Any install method works:

```sh
brew install srobroek/tap/slopvac
uv tool install slopvac
```

The shell installer attached to each GitHub release also works.

## pre-commit and prek

```yaml
repos:
  - repo: https://github.com/srobroek/slopvac
    rev: v3.0.0
    hooks:
      - id: slopvac
```

prek reads the same file, so `prek run` and `pre-commit run` behave the same.

| Hook id | Command |
| --- | --- |
| `slopvac` | `slopvac lint <files>` |
| `slopvac-strict` | `slopvac lint --config slopvac.strict.toml <files>` |

The hook has no pinned tool version. The `rev` selects the hook definition, and the binary on `PATH` selects the rules. Pin the binary version in your tool manager, for example with `mise`.

`--profile` is removed in 3.0, so `slopvac-strict` reads its style from `slopvac.strict.toml` at the repository root. Create that file with the style presets you want for reference material. Scope the hook with `files:` so it runs only on reference material:

```yaml
- id: slopvac-strict
  files: ^docs/reference/
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
