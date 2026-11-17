# Testing the Rust workspace

This page sets the conventions for tests in `crates/`. Behaviour tests go through the public interface. A refactor leaves them alone. Delete a test when the behaviour it covers is removed.

Worked examples of each tool live in [`crates/slopvac/tests/testing_toolkit.rs`](../crates/slopvac/tests/testing_toolkit.rs).

## Choose the tool

| Need | Tool | Where |
|---|---|---|
| Run tests | `cargo nextest run` | Everywhere. CI runs `--profile ci`. |
| Doctests | `cargo test --doc` | nextest does not run them. |
| Command-line behaviour | snapbox | `crates/<crate>/tests/*.rs`, driving the built binary. |
| Large or structured output | insta | Beside the test, in `snapshots/`. |
| Same behaviour, different data | rstest | Name each case with `#[case::name]`. |
| An invariant over any input | proptest | Parsers, config merge, span arithmetic. |

## Rules

- One behaviour per test. Work out expected values apart from the code under test.
- Parametrize cases that differ only in data. Do not copy a test for each input.
- Use the lowest level that catches the fault. Unit tests sit next to the code; the binary is tested from `tests/`.
- Never edit a snapshot or an expected value just to pass. Read the diff, decide which side is wrong, then change that side.
- Do not snapshot text that changes with every edit, such as timestamps or absolute paths. Redact it first.

## Snapshots

Review pending snapshots with `cargo insta review`, or accept them with `cargo insta accept`. CI never writes snapshots: insta treats a missing or different snapshot as a failure when the `CI` variable is set. Commit the `.snap` files.

The `--help` snapshot in `testing_toolkit.rs` changes whenever the command tree changes. The change that edits the CLI updates the snapshot in the same pull request.

## Property tests

proptest saves failing cases to a `<test file>.proptest-regressions` file next to an integration test, for example `crates/slopvac/tests/cli.proptest-regressions`. For a test inside `src/`, it uses a `proptest-regressions/` directory. Commit these files so a fixed bug stays fixed. Keep generators small: a property that needs thousands of cases to find a fault belongs in the nightly fuzzing job instead.

## Fuzzing

The `fuzz/` crate holds cargo-fuzz targets for the parsers that read untrusted text: `frontmatter_spans`, `config_load` and `parse_document` (Markdown, MDX and HTML). Each target must not panic on any input. The crate is excluded from the workspace and needs the nightly toolchain, so stable builds never compile it.

```sh
just fuzz frontmatter_spans 120   # target, then seconds (default 60)
```

The scheduled workflow `rust-fuzz.yml` runs every target for 5 minutes each night and uploads crashing inputs as artifacts. Reproduce one with `cargo +nightly fuzz run <target> <artifact>`. Fix the bug, then add a regression test that uses the minimised input.

## Mutation testing

`just mutants` runs cargo-mutants, which changes the code and reports changes that no test catches. Pass a crate name to limit the run: `just mutants slopvac-text`. Settings live in `.cargo/mutants.toml`. A full run takes hours, so `rust-mutants.yml` runs it weekly and on demand. It does not gate pull requests, and it uploads `mutants.out`. Read `missed.txt` there: each line is behaviour a test should pin down.

## Dependencies

Declare test tools once under `[workspace.dependencies]` and use `.workspace = true` in a crate's `[dev-dependencies]`. `cargo shear` fails on an unused one.

## Coverage

Install `cargo-llvm-cov` 0.9.1 with `cargo install --locked cargo-llvm-cov@0.9.1`. Add the LLVM tools with `rustup component add llvm-tools-preview`. Then run `just coverage`.

The recipe runs the nextest suite under instrumentation. It writes `target/llvm-cov/lcov.info` and prints a per-file summary. Run `cargo llvm-cov nextest --workspace --html` for an HTML report at `target/llvm-cov/html/index.html`. Run doctests separately with `cargo test --doc`.

The `Rust coverage` workflow runs weekly and on demand. It uploads the lcov file as the `rust-coverage-lcov` artifact. It sets no threshold, so a pull request never waits on coverage.
