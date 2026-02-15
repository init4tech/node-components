# Signet Database

Extensions and modifications to reth's Database system for use in the Signet
Node.

This library contains the following:

- Traits for reading and writing Signet events
- Table definitions for Signet Events and Headers
- Helpers for reading, writing, reverting, Signet EVM blocks and headers

## Significant Traits

- `RuWriter` - Encapsulates logic for reading and writing Signet events, state,
  headers, etc.
- `DbProviderExt` - Extends the reth `DatabaseProviderRW` with a scope-guarded
  `update` method.
- `DataCompat` - Provides methods for converting between Signet and reth data
  structures, such as `ExecutionOutcome` and `Receipt`.
