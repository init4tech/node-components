use reth::providers::errors::lockfile::StorageLockError;

use crate::hot::{
    DeserError,
    model::{HotKvError, HotKvReadError},
};

/// Error type for reth-libmdbx based hot storage.
#[derive(Debug, thiserror::Error)]
pub enum MdbxError {
    /// Inner error
    #[error(transparent)]
    Mdbx(#[from] reth_libmdbx::Error),

    /// Error when a raw value does not conform to expected fixed size.
    #[error("Error with dup fixed value size: expected {expected} bytes, found {found} bytes")]
    DupFixedErr {
        /// Expected size
        expected: usize,
        /// Found size
        found: usize,
    },

    /// Tried to invoke a DUPSORT operation on a table that is not flagged
    /// DUPSORT
    #[error("tried to invoke a DUPSORT operation on a table that is not flagged DUPSORT")]
    NotDupSort,

    /// Key2 size is unknown, cannot split DUPSORT value.
    /// This error occurs when using raw cursor methods on a DUP_FIXED table
    /// without first setting the key2/value sizes via typed methods.
    /// Use typed methods instead of raw methods when working with dual-key tables.
    #[error(
        "fixed size for DUPSORT value is unknown. Hint: use typed methods instead of raw methods when working with dual-key tables"
    )]
    UnknownFixedSize,

    /// Table not found
    #[error("table not found: {0}")]
    UnknownTable(&'static str),

    /// Storage lock error
    #[error(transparent)]
    Locked(#[from] StorageLockError),

    /// Deser.
    #[error(transparent)]
    Deser(#[from] DeserError),
}

impl trevm::revm::database::DBErrorMarker for MdbxError {}

impl HotKvReadError for MdbxError {
    fn into_hot_kv_error(self) -> HotKvError {
        match self {
            MdbxError::Deser(e) => HotKvError::Deser(e),
            _ => HotKvError::from_err(self),
        }
    }
}
