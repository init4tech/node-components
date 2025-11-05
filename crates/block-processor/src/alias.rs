use alloy::{
    consensus::constants::KECCAK_EMPTY,
    primitives::{Address, map::HashSet},
};
use eyre::OptionExt;
use reth::providers::{StateProvider, StateProviderFactory};
use std::sync::{Arc, Mutex};

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

impl AliasOracle for HashSet<Address> {
    fn should_alias(&self, address: Address) -> eyre::Result<bool> {
        Ok(self.contains(&address))
    }
}

/// Factory trait to create new [`AliasOracle`] instances.
///
/// The default implementation on `Box<dyn StateProviderFactory>` creates
/// [`AliasOracle`] instances backed by the state provider for a given block
/// height. It will error if that state provider cannot be obtained.
///
/// This trait is primarily intended to allow injecting test implementations
/// of [`AliasOracle`] into the Signet Node for testing purposes. The test
/// implementation on [`HashSet<Address>`] allows specifying a fixed set of
/// addresses to be aliased.
pub trait AliasOracleFactory: Send + Sync + 'static {
    /// The [`AliasOracle`] type.
    type Oracle: AliasOracle;

    /// Create a new [`AliasOracle`].
    fn create(&self) -> eyre::Result<Self::Oracle>;
}

impl AliasOracleFactory for Box<dyn StateProviderFactory> {
    type Oracle = Box<dyn StateProvider>;

    fn create(&self) -> eyre::Result<Self::Oracle> {
        self.state_by_block_number_or_tag(alloy::eips::BlockNumberOrTag::Latest).map_err(Into::into)
    }
}

/// This implementation is primarily for testing purposes.
impl AliasOracleFactory for HashSet<Address> {
    type Oracle = HashSet<Address>;

    fn create(&self) -> eyre::Result<Self::Oracle> {
        Ok(self.clone())
    }
}

impl<T> AliasOracleFactory for Mutex<T>
where
    T: AliasOracleFactory,
{
    type Oracle = T::Oracle;

    fn create(&self) -> eyre::Result<Self::Oracle> {
        let guard =
            self.lock().map_err(|_| eyre::eyre!("failed to lock AliasOracleFactory mutex"))?;
        guard.create()
    }
}

impl<T> AliasOracleFactory for Arc<T>
where
    T: AliasOracleFactory,
{
    type Oracle = T::Oracle;

    fn create(&self) -> eyre::Result<Self::Oracle> {
        self.as_ref().create()
    }
}
