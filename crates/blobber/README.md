# Block Extractor

The [`BlockExtractor`] retrieves blobs from host chain blocks and parses them
into [`ZenithBlock`]s. It is used by the node during notification processing
when a [`Zenith::BlockSubmitted`] event is extracted from a host chain block.

## Data Sources

The following sources can be configured:

- The local EL node transaction pool.
- The local CL node via RPC.
- A blob explorer.
- Signet's Pylon blob storage system.

[`ZenithBlock`]: signet_zenith::ZenithBlock
[`Zenith::BlockSubmitted`]: signet_zenith::Zenith::BlockSubmitted
