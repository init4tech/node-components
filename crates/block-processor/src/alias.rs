use alloy::{
    consensus::constants::KECCAK_EMPTY,
    primitives::{Address, map::HashSet},
};
use eyre::OptionExt;
use reth::providers::{StateProvider, StateProviderFactory};

/// Simple trait to allow checking if an address should be aliased.
pub trait AliasOracle {
    /// Returns true if the given address is an alias.
    fn should_alias(&self, address: Address) -> eyre::Result<bool>;
}

impl AliasOracle for Box<dyn StateProvider> {
    fn should_alias(&self, address: Address) -> eyre::Result<bool> {
        let Some(acct) = self.basic_account(&address)? else { return Ok(false) };
        let bch = match acct.bytecode_hash {
            Some(hash) => hash,
            None => return Ok(false),
        };
        if bch == KECCAK_EMPTY {
            return Ok(false);
        }
        let code = self
            .bytecode_by_hash(&bch)?
            .ok_or_eyre("code not found. This indicates a corrupted database")?;

        // Check for 7702 delegations.
        if code.len() != 23 || !code.bytecode().starts_with(&[0xef, 0x01, 0x00]) {
            return Ok(true);
        }
        Ok(false)
    }
}

impl AliasOracle for &HashSet<Address> {
    fn should_alias(&self, address: Address) -> eyre::Result<bool> {
        Ok(self.contains(&address))
    }
}

/// Factory trait to create new [`AliasOracle`] instances.
pub trait AliasOracleFactory: Send + Sync + 'static {
    /// The [`AliasOracle`] type.
    type Oracle<'a>: AliasOracle;

    /// Create a new [`AliasOracle`].
    fn create(&self, block_height: u64) -> eyre::Result<Self::Oracle<'_>>;
}

impl AliasOracleFactory for Box<dyn StateProviderFactory> {
    type Oracle<'a> = Box<dyn StateProvider>;

    fn create(&self, block_height: u64) -> eyre::Result<Self::Oracle<'_>> {
        self.state_by_block_number_or_tag(block_height.into()).map_err(Into::into)
    }
}

impl AliasOracleFactory for HashSet<Address> {
    type Oracle<'a> = &'a HashSet<Address>;

    fn create(&self, _block_height: u64) -> eyre::Result<Self::Oracle<'_>> {
        Ok(self)
    }
}
