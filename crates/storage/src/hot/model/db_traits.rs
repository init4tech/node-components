use crate::hot::{
    model::{HotKvRead, HotKvWrite},
    tables,
};
use alloy::primitives::{Address, B256, U256};
use reth::primitives::{Account, Bytecode, Header, SealedHeader, StorageEntry};
use reth_db::{BlockNumberList, models::BlockNumberAddress};
use reth_db_api::models::ShardedKey;

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
}

impl<T> HotHistoryRead for T where T: HotDbRead {}

/// Trait for history write operations.
///
/// These tables maintain historical information about accounts and storage
/// changes, and their contents can be used to reconstruct past states or
/// roll back changes.
pub trait HotHistoryWrite: HotDbWrite {
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
    fn write_account_change(
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
    fn write_storage_change(
        &mut self,
        block_number: u64,
        address: Address,
        slot: &B256,
        value: &U256,
    ) -> Result<(), Self::Error> {
        let block_number_address = BlockNumberAddress((block_number, address));
        self.queue_put_dual::<tables::StorageChangeSets>(&block_number_address, slot, value)
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
