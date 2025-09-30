# Signet Node Tests

Test scaffolding crate, and integration tests for the Signet Node.

This library contains the following:

- `SignetTestContext` - A running test node with alloy providers and the
  ability to send transactions, query state, and mine blocks.
- `run_test` - A helper function to run async tests with a fresh
  `SignetTestContext`.
- `rpc_test` - A wrapper around `run_test` for testing RPC methods.
- Assorted test utilities and constants

The `tests/` directory contains a number of integration tests for the
Signet Node, using the above scaffolding.
