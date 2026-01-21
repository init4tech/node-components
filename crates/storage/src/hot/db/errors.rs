use alloy::primitives::B256;
use std::fmt;

/// A result type for history operations.
pub type HistoryResult<T, E> = Result<T, HistoryError<E>>;

/// Error type for history operations.
///
/// This error is returned by methods that append or unwind history,
/// and includes both chain consistency errors and database errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryError<E> {
    /// Block number doesn't extend the chain contiguously.
    NonContiguousBlock {
        /// The expected block number (current tip + 1).
        expected: u64,
        /// The actual block number provided.
        got: u64,
    },
    /// Parent hash doesn't match current tip or previous block in range.
    ParentHashMismatch {
        /// The expected parent hash.
        expected: B256,
        /// The actual parent hash provided.
        got: B256,
    },
    /// Empty header range provided to a method that requires at least one header.
    EmptyRange,
    /// Database error.
    Db(E),
}

impl<E: fmt::Display> fmt::Display for HistoryError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonContiguousBlock { expected, got } => {
                write!(f, "non-contiguous block: expected {expected}, got {got}")
            }
            Self::ParentHashMismatch { expected, got } => {
                write!(f, "parent hash mismatch: expected {expected}, got {got}")
            }
            Self::EmptyRange => write!(f, "empty header range provided"),
            Self::Db(e) => write!(f, "database error: {e}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for HistoryError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Db(e) => Some(e),
            _ => None,
        }
    }
}
