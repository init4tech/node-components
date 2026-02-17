# Signet Node Components

> **Note:** The `main` branch is in maintenance and bug-fix mode. The
> `develop` branch contains current work.

A collection of components for building the Signet node. These components
implement core node functionality, but are potentially indepedently useful.

## What's in the Components?

- **signet-node-types** - Shim types wrapping reth's internal node types
  system to make it more usable in Signet.
- **signet-blobber** - Blob retrieval and parsing, using blob explorers,
  Signet's Pylon, and the local node transaction API.
- **signet-rpc** - An Ethereum JSON-RPC Server for Signet nodes. Makes heavy
  use of reth internals.
- **signet-db** - An extension of reth's database, providing a Signet-specific
  database schema and utilities for working with Signet blocks and transactions.

### Contributing to the Node Components

Please see [CONTRIBUTING.md](CONTRIBUTING.md).

[Signet docs]: https://signet.sh/docs

## Note on Semver

This repo is UNPUBLISHED and may NOT respect semantic versioning between tagged
versions. In general, it is versioned to match the signet-sdk version with
which it is compatible. I.e. `node-components@0.8.x` is expected to be
compatible with any signet-sdk `0.8.x` version. However, a release of
`node-components@0.8.1` may have breaking changes from `node-components@0.8.0`.

## What's new in Signet?

Signet is a pragmatic Ethereum rollup that offers a new set of ideas and aims
to radically modernize rollup technology.

- No proving systems or state roots, drastically reducing computational
  overhead.
- Market-based cross-chain transfers for instant asset movement.
- A controlled block inclusion mechanism to combat block construction
  centralization.
- Conditional transactions for secure, atomic cross-chain operations.

Signet extends the EVM, and is compatible with all existing Ethereum tooling.
Using Signet does not require smart contract modifications, or Signet-specific
knowledge. Signet does not have a native token.

Signet is just a rollup.

See the [Signet docs] for more info.
