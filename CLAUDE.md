# Signet Node Components

## Branches

The `main` branch contains current work. The `legacy` branch is under
long-term maintenance and may receive active work.

## Releases

The `legacy` branch is tagged on the `0.16.x` line. These crates cannot be
published to crates.io.

To release:
1. Run standard pre-push checks (clippy both feature sets + fmt).
2. Create a new signed git tag (`git tag -s v0.16.x`).
3. Push the tag to GitHub.
4. Create a GitHub release from the tag.

## Commands

- `cargo +nightly fmt` - format
- `cargo clippy -p <crate> --all-features --all-targets` - lint with features
- `cargo clippy -p <crate> --no-default-features --all-targets` - lint without
- `cargo t -p <crate>` - test specific crate

Pre-push: clippy (both feature sets where applicable) + fmt. Never use `cargo check/build`.
These checks apply before any push — new commits, rebases, cherry-picks, etc.

## Style

- Functional combinators over imperative control flow
- `let else` for early returns, avoid nesting
- No glob imports; group imports from same crate; no blank lines between imports
- Private by default, `pub(crate)` for internal, `pub` for API only; never `pub(super)`
- `thiserror` for library errors, `eyre` for apps, never `anyhow`
- `tracing` for instrumentation: instrument work items not long-lived tasks; `skip(self)` on methods
- Builders for structs with >4 fields or multiple same-type fields
- Tests: fail fast with `unwrap()`, never return `Result`; unit tests in `mod tests`
- Rustdoc on all public items with usage examples; hide scaffolding with `#`
- `// SAFETY:` comments on all unsafe blocks

## Workspace Crates

All crates use `signet-` prefix. Features exist in:
- `signet-blobber`: `test-utils`
- `signet-node-config`: `test_utils`

Other crates (`signet-node`, `signet-node-types`, `signet-rpc`, `signet-db`, `signet-block-processor`, `signet-genesis`, `signet-node-tests`) have no feature flags — lint with `--all-features` only.
