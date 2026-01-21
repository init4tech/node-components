use crate::hot::{
    model::{HotKvRead, HotKvWrite},
    tables,
};
use alloy::primitives::{Address, B256, U256};
use reth::primitives::{Account, Bytecode, Header, SealedHeader, StorageEntry};
use reth_db::{BlockNumberList, models::BlockNumberAddress};
use reth_db_api::models::ShardedKey;
use std::fmt;

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

/// Trait for database read operations on standard hot tables.
///
/// This is a high-level trait that provides convenient methods for reading
/// common data types from predefined hot storage tables. It builds upon the
/// lower-level [`HotKvRead`] trait, which provides raw key-value access.
///
/// Users should prefer this trait unless customizations are needed to the
/// table set.
pub trait HotDbRead: HotKvRead + sealed::Sealed {
    /// Read a block header by its number.
    fn get_header(&self, number: u64) -> Result<Option<Header>, Self::Error> {
        self.get::<tables::Headers>(&number)
    }

    /// Read a block number by its hash.
    fn get_header_number(&self, hash: &B256) -> Result<Option<u64>, Self::Error> {
        self.get::<tables::HeaderNumbers>(hash)
    }

    /// Read contract Bytecode by its hash.
    fn get_bytecode(&self, code_hash: &B256) -> Result<Option<Bytecode>, Self::Error> {
        self.get::<tables::Bytecodes>(code_hash)
    }

    /// Read an account by its address.
    fn get_account(&self, address: &Address) -> Result<Option<Account>, Self::Error> {
        self.get::<tables::PlainAccountState>(address)
    }

    /// Read a storage slot by its address and key.
    fn get_storage(&self, address: &Address, key: &B256) -> Result<Option<U256>, Self::Error> {
        self.get_dual::<tables::PlainStorageState>(address, key)
    }

    /// Read a [`StorageEntry`] by its address and key.
    fn get_storage_entry(
        &self,
        address: &Address,
        key: &B256,
    ) -> Result<Option<StorageEntry>, Self::Error> {
        let opt = self.get_storage(address, key)?;
        Ok(opt.map(|value| StorageEntry { key: *key, value }))
    }

    /// Read a block header by its hash.
    fn header_by_hash(&self, hash: &B256) -> Result<Option<Header>, Self::Error> {
        let Some(number) = self.get_header_number(hash)? else {
            return Ok(None);
        };
        self.get_header(number)
    }
}

impl<T> HotDbRead for T where T: HotKvRead {}

/// Trait for database write operations on standard hot tables.
///
/// This trait is low-level, and usage may leave the database in an
/// inconsistent state if not used carefully. Users should prefer
/// [`HotHistoryWrite`] or higher-level abstractions when possible.
pub trait HotDbWrite: HotKvWrite + sealed::Sealed {
    /// Write a block header. This will leave the DB in an inconsistent state
    /// until the corresponding header number is also written. Users should
    /// prefer [`Self::put_header`] instead.
    fn put_header_inconsistent(&mut self, header: &Header) -> Result<(), Self::Error> {
        self.queue_put::<tables::Headers>(&header.number, header)
    }

    /// Write a block number by its hash. This will leave the DB in an
    /// inconsistent state until the corresponding header is also written.
    /// Users should prefer [`Self::put_header`] instead.
    fn put_header_number_inconsistent(
        &mut self,
        hash: &B256,
        number: u64,
    ) -> Result<(), Self::Error> {
        self.queue_put::<tables::HeaderNumbers>(hash, &number)
    }

    /// Write contract Bytecode by its hash.
    fn put_bytecode(&mut self, code_hash: &B256, bytecode: &Bytecode) -> Result<(), Self::Error> {
        self.queue_put::<tables::Bytecodes>(code_hash, bytecode)
    }

    /// Write an account by its address.
    fn put_account(&mut self, address: &Address, account: &Account) -> Result<(), Self::Error> {
        self.queue_put::<tables::PlainAccountState>(address, account)
    }

    /// Write a storage entry by its address and key.
    fn put_storage(
        &mut self,
        address: &Address,
        key: &B256,
        entry: &U256,
    ) -> Result<(), Self::Error> {
        self.queue_put_dual::<tables::PlainStorageState>(address, key, entry)
    }

    /// Write a sealed block header (header + number).
    fn put_header(&mut self, header: &SealedHeader) -> Result<(), Self::Error> {
        self.put_header_inconsistent(header.header())
            .and_then(|_| self.put_header_number_inconsistent(&header.hash(), header.number))
    }

    /// Delete a header by block number.
    fn delete_header(&mut self, number: u64) -> Result<(), Self::Error> {
        self.queue_delete::<tables::Headers>(&number)
    }

    /// Delete a header number mapping by hash.
    fn delete_header_number(&mut self, hash: &B256) -> Result<(), Self::Error> {
        self.queue_delete::<tables::HeaderNumbers>(hash)
    }

    /// Commit the write transaction.
    fn commit(self) -> Result<(), Self::Error>
    where
        Self: Sized,
    {
        HotKvWrite::raw_commit(self)
    }
}

impl<T> HotDbWrite for T where T: HotKvWrite {}

/// Trait for history read operations.
///
/// These tables maintain historical information about accounts and storage
/// changes, and their contents can be used to reconstruct past states or
/// roll back changes.
///
/// This is a high-level trait that provides convenient methods for reading
/// common data types from predefined hot storage history tables. It builds
/// upon the lower-level [`HotDbRead`] trait, which provides raw key-value
/// access.
///
/// Users should prefer this trait unless customizations are needed to the
/// table set.
pub trait HotHistoryRead: HotDbRead {
    /// Get the list of block numbers where an account was touched.
    /// Get the list of block numbers where an account was touched.
    fn get_account_history(
        &self,
        address: &Address,
        latest_height: u64,
    ) -> Result<Option<BlockNumberList>, Self::Error> {
        self.get_dual::<tables::AccountsHistory>(address, &latest_height)
    }
    /// Get the account change (pre-state) for an account at a specific block.
    ///
    /// If the return value is `None`, the account was not changed in that
    /// block.
    fn get_account_change(
        &self,
        block_number: u64,
        address: &Address,
    ) -> Result<Option<Account>, Self::Error> {
        self.get_dual::<tables::AccountChangeSets>(&block_number, address)
    }

    /// Get the storage history for an account and storage slot. The returned
    /// list will contain block numbers where the storage slot was changed.
    fn get_storage_history(
        &self,
        address: &Address,
        slot: B256,
        highest_block_number: u64,
    ) -> Result<Option<BlockNumberList>, Self::Error> {
        let sharded_key = ShardedKey::new(slot, highest_block_number);
        self.get_dual::<tables::StorageHistory>(address, &sharded_key)
    }

    /// Get the storage change (before state) for a specific storage slot at a
    /// specific block.
    ///
    /// If the return value is `None`, the storage slot was not changed in that
    /// block. If the return value is `Some(value)`, the value is the pre-state
    /// of the storage slot before the change in that block. If the value is
    /// `U256::ZERO`, that indicates that the storage slot was not set before
    /// the change.
    fn get_storage_change(
        &self,
        block_number: u64,
        address: &Address,
        slot: &B256,
    ) -> Result<Option<U256>, Self::Error> {
        let block_number_address = BlockNumberAddress((block_number, *address));
        self.get_dual::<tables::StorageChangeSets>(&block_number_address, slot)
    }

    /// Get the last (highest) header in the database.
    /// Returns None if the database is empty.
    fn last_header(&self) -> Result<Option<Header>, Self::Error> {
        let mut cursor = self.traverse::<tables::Headers>()?;
        Ok(cursor.last()?.map(|(_, header)| header))
    }

    /// Get the first (lowest) header in the database.
    /// Returns None if the database is empty.
    fn first_header(&self) -> Result<Option<Header>, Self::Error> {
        let mut cursor = self.traverse::<tables::Headers>()?;
        Ok(cursor.first()?.map(|(_, header)| header))
    }

    /// Get the current chain tip (highest block number and hash).
    /// Returns None if the database is empty.
    fn get_chain_tip(&self) -> Result<Option<(u64, B256)>, Self::Error> {
        let mut cursor = self.traverse::<tables::Headers>()?;
        let Some((number, header)) = cursor.last()? else {
            return Ok(None);
        };
        let hash = header.hash_slow();
        Ok(Some((number, hash)))
    }

    /// Get the execution range (first and last block numbers with headers).
    /// Returns None if the database is empty.
    fn get_execution_range(&self) -> Result<Option<(u64, u64)>, Self::Error> {
        let mut cursor = self.traverse::<tables::Headers>()?;
        let Some((first, _)) = cursor.first()? else {
            return Ok(None);
        };
        let Some((last, _)) = cursor.last()? else {
            return Ok(None);
        };
        Ok(Some((first, last)))
    }

    /// Check if a specific block number exists in history.
    fn has_block(&self, number: u64) -> Result<bool, Self::Error> {
        self.get_header(number).map(|opt| opt.is_some())
    }

    /// Get headers in a range (inclusive).
    fn get_headers_range(&self, start: u64, end: u64) -> Result<Vec<Header>, Self::Error> {
        let mut cursor = self.traverse::<tables::Headers>()?;
        let mut headers = Vec::new();

        if cursor.lower_bound(&start)?.is_none() {
            return Ok(headers);
        }

        loop {
            match cursor.read_next()? {
                Some((num, header)) if num <= end => {
                    headers.push(header);
                }
                _ => break,
            }
        }

        Ok(headers)
    }
}

impl<T> HotHistoryRead for T where T: HotDbRead {}

/// Trait for history write operations.
///
/// These tables maintain historical information about accounts and storage
/// changes, and their contents can be used to reconstruct past states or
/// roll back changes.
pub trait HotHistoryWrite: HotDbWrite + HotHistoryRead {
    /// Maintain a list of block numbers where an account was touched.
    ///
    /// Accounts are keyed
    fn write_account_history(
        &mut self,
        address: &Address,
        latest_height: u64,
        touched: &BlockNumberList,
    ) -> Result<(), Self::Error> {
        self.queue_put_dual::<tables::AccountsHistory>(address, &latest_height, touched)
    }

    /// Write an account change (pre-state) for an account at a specific
    /// block.
    fn write_account_prestate(
        &mut self,
        block_number: u64,
        address: Address,
        pre_state: &Account,
    ) -> Result<(), Self::Error> {
        self.queue_put_dual::<tables::AccountChangeSets>(&block_number, &address, pre_state)
    }

    /// Write storage history, by highest block number and touched block
    /// numbers.
    fn write_storage_history(
        &mut self,
        address: &Address,
        slot: B256,
        highest_block_number: u64,
        touched: &BlockNumberList,
    ) -> Result<(), Self::Error> {
        let sharded_key = ShardedKey::new(slot, highest_block_number);
        self.queue_put_dual::<tables::StorageHistory>(address, &sharded_key, touched)
    }

    /// Write a storage change (before state) for an account at a specific
    /// block.
    fn write_storage_prestate(
        &mut self,
        block_number: u64,
        address: Address,
        slot: &B256,
        prestate: &U256,
    ) -> Result<(), Self::Error> {
        let block_number_address = BlockNumberAddress((block_number, address));
        self.queue_put_dual::<tables::StorageChangeSets>(&block_number_address, slot, prestate)
    }

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
}

impl<T> HotHistoryWrite for T where T: HotDbWrite + HotKvWrite {}

mod sealed {
    use crate::hot::model::HotKvRead;

    /// Sealed trait to prevent external implementations of HotDbReader and HotDbWriter.
    #[allow(dead_code, unreachable_pub)]
    pub trait Sealed {}
    impl<T> Sealed for T where T: HotKvRead {}
}
