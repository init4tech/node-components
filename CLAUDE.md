# Signet Node Components

## Branches

The `main` branch contains current work. The `legacy` branch is under
long-term maintenance and may receive active work.

## Commands

- `cargo +nightly fmt` - format
- `cargo clippy -p <crate> --all-features --all-targets` - lint with features
- `cargo clippy -p <crate> --no-default-features --all-targets` - lint without
- `cargo t -p <crate>` - test specific crate

Pre-push: clippy (both feature sets where applicable) + fmt. Never use `cargo check/build`.
These checks apply before any push — new commits, rebases, cherry-picks, etc.

### Pre-push Checks (enforced by Claude hook)

A Claude hook in `.claude/settings.json` runs `.claude/hooks/pre-push.sh`
before every `git push`. The push is blocked if any check fails. The checks:

- `cargo +nightly fmt -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo clippy --workspace --all-targets --no-default-features -- -D warnings`
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`

Clippy and doc warnings are hard failures.

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
