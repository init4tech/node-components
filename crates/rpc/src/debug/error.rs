use reth::{
    providers::ProviderError,
    rpc::{eth::filter::EthFilterError, server_types::eth::EthApiError},
};
use std::borrow::Cow;

/// Errors that can occur when interacting with the `eth_` namespace.
#[derive(Debug, thiserror::Error, Clone)]
pub enum DebugError {
    /// Provider error: [`ProviderError`].
    #[error("Provider error: {0}")]
    Provider(#[from] ProviderError),
    /// Filter error [`EthFilterError`].
    #[error("Filter error: {0}")]
    Filter(Cow<'static, str>),
    /// Eth API error: [`EthApiError`].
    #[error("Eth API error: {0}")]
    Rpc(Cow<'static, str>),
}

impl DebugError {
    /// Create a new filter error.
    pub const fn filter_error(msg: Cow<'static, str>) -> Self {
        Self::Filter(msg)
    }

    /// Create a new RPC error.
    pub const fn rpc_error(msg: Cow<'static, str>) -> Self {
        Self::Rpc(msg)
    }
}

impl From<EthFilterError> for DebugError {
    fn from(err: EthFilterError) -> Self {
        Self::filter_error(err.to_string().into())
    }
}

impl From<EthApiError> for DebugError {
    fn from(err: EthApiError) -> Self {
        Self::rpc_error(err.to_string().into())
    }
}

impl DebugError {
    /// Turn into a string by value, allows for `.map_err(EthError::to_string)`
    /// to be used.
    pub fn into_string(self) -> String {
        ToString::to_string(&self)
    }
}

impl serde::Serialize for DebugError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
