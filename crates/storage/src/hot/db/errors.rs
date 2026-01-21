use alloy::primitives::B256;
use reth_db::models::integer_list::IntegerListError;

/// A result type for history operations.
pub type HistoryResult<T, E> = Result<T, HistoryError<E>>;

/// Error type for history operations.
///
/// This error is returned by methods that append or unwind history,
/// and includes both chain consistency errors and database errors.
#[derive(Debug, thiserror::Error)]
pub enum HistoryError<E: std::error::Error> {
    /// Block number doesn't extend the chain contiguously.
    #[error("non-contiguous block: expected {expected}, got {got}")]
    NonContiguousBlock {
        /// The expected block number (current tip + 1).
        expected: u64,
        /// The actual block number provided.
        got: u64,
    },
    /// Parent hash doesn't match current tip or previous block in range.

    #[error("parent hash mismatch: expected {expected}, got {got}")]
    ParentHashMismatch {
        /// The expected parent hash.
        expected: B256,
        /// The actual parent hash provided.
        got: B256,
    },

    /// Empty header range provided to a method that requires at least one header.
    #[error("empty header range provided")]
    EmptyRange,

    /// Database error.
    #[error("{0}")]
    Db(#[from] E),

    /// Integer List
    #[error(transparent)]
    IntList(IntegerListError),
}

impl<E: std::error::Error> HistoryError<E> {
    /// Helper to create a database error
    pub const fn intlist(err: IntegerListError) -> Self {
        HistoryError::IntList(err)
    }
}
