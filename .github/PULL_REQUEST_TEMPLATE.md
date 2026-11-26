<!-- Title: a Conventional Commit, `type(scope): summary`. Maintainers squash-merge, and release-please reads the title. -->

Bead: <id>
Flag: none

## Summary

<!-- What changed and why. -->

## Checks

<!-- Record the exit code of each check you ran. Delete the lines that do not apply. -->

| Check | Exit code |
|---|---|
| `cargo fmt --all --check` | |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | |
| `cargo nextest run --workspace` | |
| `cargo test --doc --workspace` | |
| `prek run --all-files` | |
| `pytest` | |
| Prose gate: `slopvac` on the changed prose (exit 2 means incomplete) | |

## Notes for reviewers

- Rust changes must meet [`docs/engineering.md`](../docs/engineering.md). To deviate from a rule, follow "How to request a departure" there.
- Contributions need a signed CLA. CLA Assistant comments on your first PR.
