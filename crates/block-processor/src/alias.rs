use alloy::primitives::{Address, map::HashSet};
use core::future::Future;
use std::sync::{Arc, Mutex};

/// Simple trait to allow checking if an address should be aliased.
pub trait AliasOracle {
    /// Returns true if the given address is an alias.
    fn should_alias(&self, address: Address) -> impl Future<Output = eyre::Result<bool>> + Send;
}

impl AliasOracle for HashSet<Address> {
    fn should_alias(&self, address: Address) -> impl Future<Output = eyre::Result<bool>> + Send {
        let result = Ok(self.contains(&address));
        async move { result }
    }
}

/// Factory trait to create new [`AliasOracle`] instances.
///
/// See `signet-node` for the reth-backed implementation that creates
/// [`AliasOracle`] instances backed by the host chain state provider.
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
