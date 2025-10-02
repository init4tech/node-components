# Signet Genesis

Genesis configuration and utilities for the Signet Node.

This library contains the following:

- `GenesisSpec` - An enum representing different genesis specifications, either
  Pecorino, Test, or a custom genesis file path, which can be used to load
  genesis data.
- `PECORINO_GENESIS` - The Pecorino genesis data.
- `TEST_GENESIS` - A local test genesis for testing purposes.
- `GenesisError` - Errors that can occur when loading or parsing genesis data.

## Example

```
# use signet_genesis::GenesisSpec;
# fn _main() -> Result<(), Box<dyn std::error::Error>> {
let genesis = GenesisSpec::Pecorino.load_genesis()?;
# Ok(())
# }
```
