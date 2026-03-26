//! Error types for the signet namespace.

use std::borrow::Cow;

/// Errors that can occur in the `signet` namespace.
#[derive(Debug, thiserror::Error)]
pub enum SignetError {
    /// The transaction cache was not provided.
    #[error("transaction cache not provided")]
    TxCacheNotProvided,
    /// Block resolution failed.
    #[error("block resolution error")]
    Resolve(String),
    /// EVM execution halted for a non-revert reason.
    #[error("execution halted: {reason}")]
    EvmHalt {
        /// The halt reason.
        reason: String,
    },
    /// Bundle simulation timed out.
    #[error("timeout during bundle simulation")]
    Timeout,
}

impl ajj::IntoErrorPayload for SignetError {
    type ErrData = ();

    fn error_code(&self) -> i64 {
        match self {
            Self::TxCacheNotProvided | Self::Resolve(_) | Self::EvmHalt { .. } | Self::Timeout => {
                -32000
            }
        }
    }

    fn error_message(&self) -> Cow<'static, str> {
        match self {
            Self::TxCacheNotProvided => "transaction cache not provided".into(),
            Self::Resolve(_) => "block resolution error".into(),
            Self::EvmHalt { reason } => format!("execution halted: {reason}").into(),
            Self::Timeout => "timeout during bundle simulation".into(),
        }
    }

    fn error_data(self) -> Option<Self::ErrData> {
        None
    }
}
