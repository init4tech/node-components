use crate::hot::{model::HotKvRead, tables};
use alloy::primitives::{Address, B256, U256};
use reth::primitives::{Account, Header, StorageEntry};
use reth_db::{BlockNumberList, models::BlockNumberAddress};
use reth_db_api::models::ShardedKey;
use trevm::revm::bytecode::Bytecode;

/// Trait for database read operations on standard hot tables.
///
/// This is a high-level trait that provides convenient methods for reading
/// common data types from predefined hot storage tables. It builds upon the
/// lower-level [`HotKvRead`] trait, which provides raw key-value access.
///
/// Users should prefer this trait unless customizations are needed to the
/// table set.
pub trait HotDbRead: HotKvRead + super::sealed::Sealed {
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
    fn get_storage(&self, address: &Address, key: &U256) -> Result<Option<U256>, Self::Error> {
        self.get_dual::<tables::PlainStorageState>(address, key)
    }

    /// Read a [`StorageEntry`] by its address and key.
    fn get_storage_entry(
        &self,
        address: &Address,
        key: &U256,
    ) -> Result<Option<StorageEntry>, Self::Error> {
        let opt = self.get_storage(address, key)?;
        Ok(opt.map(|value| StorageEntry { key: B256::new(key.to_be_bytes()), value }))
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

    /// Get the last (highest) account history entry for an address.
    fn last_account_history(
        &self,
        address: Address,
    ) -> Result<Option<(u64, BlockNumberList)>, Self::Error> {
        let mut cursor = self.traverse_dual::<tables::AccountsHistory>()?;

        // Move the cursor to the last entry for the given address
        let Some(res) = cursor.last_of_k1(&address)? else {
            return Ok(None);
        };

        Ok(Some((res.1, res.2)))
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
        slot: U256,
        highest_block_number: u64,
    ) -> Result<Option<BlockNumberList>, Self::Error> {
        let sharded_key = ShardedKey::new(slot, highest_block_number);
        self.get_dual::<tables::StorageHistory>(address, &sharded_key)
    }

    /// Get the last (highest) storage history entry for an address and slot.
    fn last_storage_history(
        &self,
        address: &Address,
        slot: &U256,
    ) -> Result<Option<(ShardedKey<U256>, BlockNumberList)>, Self::Error> {
        let mut cursor = self.traverse_dual::<tables::StorageHistory>()?;

        // Seek to the highest possible key for this (address, slot) combination.
        // ShardedKey encodes as slot || highest_block_number, so seeking to
        // (address, ShardedKey::new(slot, u64::MAX)) positions us at or after
        // the last shard for this slot.
        let target = ShardedKey::new(*slot, u64::MAX);
        let result = cursor.next_dual_above(address, &target)?;

        // Check if we found an exact match for this address and slot
        if let Some((addr, sharded_key, list)) = result
            && addr == *address
            && sharded_key.key == *slot
        {
            return Ok(Some((sharded_key, list)));
        }

        // The cursor is positioned at or after our target. Go backwards to find
        // the last entry for this (address, slot).
        let Some((addr, sharded_key, list)) = cursor.previous_k2()? else {
            return Ok(None);
        };

        if addr == *address && sharded_key.key == *slot {
            Ok(Some((sharded_key, list)))
        } else {
            Ok(None)
        }
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
        slot: &U256,
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

    /// Get the last (highest) block number in the database.
    /// Returns None if the database is empty.
    fn last_block_number(&self) -> Result<Option<u64>, Self::Error> {
        let mut cursor = self.traverse::<tables::Headers>()?;
        Ok(cursor.last()?.map(|(number, _)| number))
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
