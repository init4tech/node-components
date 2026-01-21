use crate::hot::{db::HotHistoryRead, model::HotKvWrite, tables};
use alloy::primitives::{Address, B256, U256};
use reth::primitives::{Account, Bytecode, Header, SealedHeader};
use reth_db::{BlockNumberList, models::BlockNumberAddress};
use reth_db_api::models::ShardedKey;

/// Trait for database write operations on standard hot tables.
///
/// This trait is low-level, and usage may leave the database in an
/// inconsistent state if not used carefully. Users should prefer
/// [`HotHistoryWrite`] or higher-level abstractions when possible.
pub trait UnsafeDbWrite: HotKvWrite + super::sealed::Sealed {
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

impl<T> UnsafeDbWrite for T where T: HotKvWrite {}

/// Trait for history write operations.
///
/// These tables maintain historical information about accounts and storage
/// changes, and their contents can be used to reconstruct past states or
/// roll back changes.
pub trait UnsafeHistoryWrite: UnsafeDbWrite + HotHistoryRead {
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
}

impl<T> UnsafeHistoryWrite for T where T: UnsafeDbWrite + HotKvWrite {}
