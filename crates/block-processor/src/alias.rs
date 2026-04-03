use alloy::primitives::{Address, map::HashSet};
use core::future::{self, Future};
use std::sync::{Arc, Mutex};

/// Error type for [`AliasOracle`] and [`AliasOracleFactory`] operations.
///
/// Implementation-specific errors are wrapped in the [`Internal`] variant.
///
/// [`Internal`]: AliasError::Internal
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AliasError {
    /// An implementation-specific error.
    #[error(transparent)]
    Internal(Box<dyn core::error::Error + Send + Sync>),
}

/// Simple trait to allow checking if an address should be aliased.
pub trait AliasOracle {
    /// Returns true if the given address is an alias.
    fn should_alias(
        &self,
        address: Address,
    ) -> impl Future<Output = Result<bool, AliasError>> + Send;
}

impl AliasOracle for HashSet<Address> {
    fn should_alias(
        &self,
        address: Address,
    ) -> impl Future<Output = Result<bool, AliasError>> + Send {
        future::ready(Ok(self.contains(&address)))
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
    fn create(&self) -> Result<Self::Oracle, AliasError>;
}

/// This implementation is primarily for testing purposes.
impl AliasOracleFactory for HashSet<Address> {
    type Oracle = HashSet<Address>;

    fn create(&self) -> Result<Self::Oracle, AliasError> {
        Ok(self.clone())
    }
}

impl<T> AliasOracleFactory for Mutex<T>
where
    T: AliasOracleFactory,
{
    type Oracle = T::Oracle;

    fn create(&self) -> Result<Self::Oracle, AliasError> {
        let guard = self.lock().map_err(|e| AliasError::Internal(e.to_string().into()))?;
        guard.create()
    }
}

impl<T> AliasOracleFactory for Arc<T>
where
    T: AliasOracleFactory,
{
    type Oracle = T::Oracle;

    fn create(&self) -> Result<Self::Oracle, AliasError> {
        self.as_ref().create()
    }
}
