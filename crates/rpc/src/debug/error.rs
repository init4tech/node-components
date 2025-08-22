use reth::{
    providers::ProviderError,
    rpc::{eth::filter::EthFilterError, server_types::eth::EthApiError},
};

/// Errors that can occur when interacting with the `eth_` namespace.
#[derive(Debug, thiserror::Error, Clone)]
pub enum TraceError {
    /// Provider error: [`ProviderError`].
    #[error("Provider error: {0}")]
    Provider(#[from] ProviderError),
    /// Filter error [`EthFilterError`].
    #[error("Filter error: {0}")]
    Filter(String),
    /// Eth API error: [`EthApiError`].
    #[error("Eth API error: {0}")]
    Rpc(String),
}

impl From<EthFilterError> for TraceError {
    fn from(err: EthFilterError) -> Self {
        TraceError::Filter(err.to_string())
    }
}

impl From<EthApiError> for TraceError {
    fn from(err: EthApiError) -> Self {
        TraceError::Rpc(err.to_string())
    }
}

impl TraceError {
    /// Turn into a string by value, allows for `.map_err(EthError::to_string)`
    /// to be used.
    pub fn into_string(self) -> String {
        ToString::to_string(&self)
    }
}

impl serde::Serialize for TraceError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
