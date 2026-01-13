# Signet Storage

High-level API for Signet's storage layer

This library contains the following:

- Traits for serializing and deserializing Signet data structures as DB keys/
  value.
- Traits for hot and cold storage operations.
- Relevant KV table definitions.

## Significant Traits

- `HotKv` - Encapsulates logic for reading and writing to hot storage.
- `ColdKv` - Encapsulates logic for reading and writing to cold storage.
- `KeySer` - Provides methods for serializing a type as a DB key.
- `ValueSer` - Provides methods for serializing a type as a DB value.
