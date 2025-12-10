# Signet Genesis

Genesis configuration and utilities for the Signet Node.

This library contains the following:

- `GenesisSpec` - An enum representing different genesis specifications
  (Mainnet, Parmigiana, Pecorino, Test, or custom file paths), which can be
  used to load genesis data.
- `NetworkGenesis` - A struct containing both rollup and host genesis
  configurations.
- `PARMIGIANA_GENESIS` / `PARMIGIANA_HOST_GENESIS` - The Parmigiana genesis
  data.
- `PECORINO_GENESIS` / `PECORINO_HOST_GENESIS` - The Pecorino genesis data.
- `TEST_GENESIS` / `TEST_HOST_GENESIS` - Local test genesis for testing
  purposes.
- `GenesisError` - Errors that can occur when loading or parsing genesis data.

## Example

```
# use signet_genesis::GenesisSpec;
# fn _main() -> Result<(), Box<dyn std::error::Error>> {
let genesis = GenesisSpec::Parmigiana.load_genesis()?;
// Access rollup (L2/Signet) genesis
let rollup = &genesis.rollup;
// Access host (L1/Ethereum) genesis
let host = &genesis.host;
# Ok(())
# }
```
