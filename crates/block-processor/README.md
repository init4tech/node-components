# Signet Block Processor

Block processing logic for the Signet Node. This crate extracts and processes
Signet blocks from host chain commits using the EVM, reading rollup state from
hot storage.

# Significant Types

- `SignetBlockProcessor<H, Alias>` — The block processor. Reads state from
  `HotKv` storage, runs the EVM via `signet_evm`, and returns an
  `ExecutedBlock`.
