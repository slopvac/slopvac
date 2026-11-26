# slopvac

This repository ships the `slopvac` CLI. Run `slopvac prime` before writing or
reviewing prose; it prints the current lint guidance from the installed version.

Run the relevant package tests with `pytest`. Use
`cargo run -p slopvac -- lint <files...>` for the repository prose gate; it
builds the 3.x binary from the checkout and reads the root `slopvac.toml`. Treat
lint exit 2 as incomplete, and verify documentation claims
against the code rather than inferring correctness from a prose score.

Rust changes must meet `docs/engineering.md`; reviewers check against it.
