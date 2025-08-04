# Block Extractor

The [`BlobFetcher`] retrieves blobs from host chain blocks and parses them
into [`ZenithBlock`]s. It is used by the node during notification processing
when a [`Zenith::BlockSubmitted`] event is extracted from a host chain block.

The [`BlobCacher`] is a wrapper around the [`BlobFetcher`] that caches
blobs in an in-memory cache. It is used to avoid fetching the same blob and to
manage retry logic during fetching.

## Data Sources

The following sources can be configured:

- The local EL node transaction pool.
- The local CL node via RPC.
- A blob explorer.
- Signet's Pylon blob storage system.

[`ZenithBlock`]: signet_zenith::ZenithBlock
[`Zenith::BlockSubmitted`]: signet_zenith::Zenith::BlockSubmitted
