# Signet Block Processor

Block processing logic for the Signet Node. This crate takes a reth `Chain`,
runs the Signet EVM, and commits the results to a database.

# Significant Types

- A few convenience type aliases:
  - `PrimitivesOf<Host>` - The primitives type used by the host.
  - `Chain<Host>` - A reth `Chain` using the host's primitives.
  - `ExExNotification<Host>` - A reth `ExExNotification` using the host's
    primitives.
- `SignetBlockProcessorV1<Host, Db>` - The first version of the block processor.
