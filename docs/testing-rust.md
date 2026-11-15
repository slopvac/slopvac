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

## Dependencies

Declare test tools once under `[workspace.dependencies]` and use `.workspace = true` in a crate's `[dev-dependencies]`. `cargo shear` fails on an unused one.
