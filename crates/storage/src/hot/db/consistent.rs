use crate::hot::db::{HistoryError, UnsafeDbWrite, UnsafeHistoryWrite};
use reth::primitives::SealedHeader;
use trevm::revm::database::BundleState;

/// Trait for database write operations on hot history tables. This trait
/// maintains a consistent state of the database.
pub trait HistoryWrite: UnsafeDbWrite + UnsafeHistoryWrite {
    /// Validate that a range of headers forms a valid chain extension.
    ///
    /// Headers must be in order and each must extend the previous.
    /// The first header must extend the current database tip (or be the first
    /// block if the database is empty).
    ///
    /// Returns `Ok(())` if valid, or an error describing the inconsistency.
    fn validate_chain_extension<'a, I>(&self, headers: I) -> Result<(), HistoryError<Self::Error>>
    where
        I: IntoIterator<Item = &'a SealedHeader>,
    {
        let headers: Vec<_> = headers.into_iter().collect();
        if headers.is_empty() {
            return Err(HistoryError::EmptyRange);
        }

        // Validate first header against current DB tip
        let first = headers[0];
        match self.get_chain_tip().map_err(HistoryError::Db)? {
            None => {
                // Empty DB - first block is valid as genesis
            }
            Some((tip_number, tip_hash)) => {
                let expected_number = tip_number + 1;
                if first.number != expected_number {
                    return Err(HistoryError::NonContiguousBlock {
                        expected: expected_number,
                        got: first.number,
                    });
                }
                if first.parent_hash != tip_hash {
                    return Err(HistoryError::ParentHashMismatch {
                        expected: tip_hash,
                        got: first.parent_hash,
                    });
                }
            }
        }

        // Validate each subsequent header extends the previous
        for window in headers.windows(2) {
            let prev = window[0];
            let curr = window[1];

            let expected_number = prev.number + 1;
            if curr.number != expected_number {
                return Err(HistoryError::NonContiguousBlock {
                    expected: expected_number,
                    got: curr.number,
                });
            }

            let expected_hash = prev.hash();
            if curr.parent_hash != expected_hash {
                return Err(HistoryError::ParentHashMismatch {
                    expected: expected_hash,
                    got: curr.parent_hash,
                });
            }
        }

        Ok(())
    }

    /// Append a range of blocks and their associated state to the database.
    fn append_blocks(
        &self,
        blocks: &[(SealedHeader, BundleState)],
    ) -> Result<(), HistoryError<Self::Error>> {
        self.validate_chain_extension(blocks.iter().map(|(h, _)| h))?;

        let Some(first_num) = blocks.first().map(|(h, _)| h.number) else { return Ok(()) };
        let last_num = blocks.last().map(|(h, _)| h.number).expect("non-empty; qed");
        self.append_blocks_inconsistent(blocks)?;

        self.update_history_indices_inconsistent(first_num..=last_num)
    }
}

impl<T> HistoryWrite for T where T: UnsafeDbWrite + UnsafeHistoryWrite {}
