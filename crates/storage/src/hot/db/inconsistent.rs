use crate::hot::{db::HotHistoryRead, model::HotKvWrite, tables};
use alloy::primitives::{Address, B256, U256};
use reth::primitives::{Account, Bytecode, Header, SealedHeader};
use reth_db::{BlockNumberList, models::BlockNumberAddress};
use reth_db_api::models::ShardedKey;
use trevm::revm::{
    database::states::{PlainStateReverts, PlainStorageRevert},
    state::AccountInfo,
};

/// Trait for database write operations on standard hot tables.
///
/// This trait is low-level, and usage may leave the database in an
/// inconsistent state if not used carefully. Users should prefer
/// [`HotHistoryWrite`] or higher-level abstractions when possible.
pub trait UnsafeDbWrite: HotKvWrite + super::sealed::Sealed {
    /// Write a block header. This will leave the DB in an inconsistent state
    /// until the corresponding header number is also written. Users should
    /// prefer [`Self::put_header`] instead.
    fn put_header_inconsistent(&self, header: &Header) -> Result<(), Self::Error> {
        self.queue_put::<tables::Headers>(&header.number, header)
    }

    /// Write a block number by its hash. This will leave the DB in an
    /// inconsistent state until the corresponding header is also written.
    /// Users should prefer [`Self::put_header`] instead.
    fn put_header_number_inconsistent(&self, hash: &B256, number: u64) -> Result<(), Self::Error> {
        self.queue_put::<tables::HeaderNumbers>(hash, &number)
    }

    /// Write contract Bytecode by its hash.
    fn put_bytecode(&self, code_hash: &B256, bytecode: &Bytecode) -> Result<(), Self::Error> {
        self.queue_put::<tables::Bytecodes>(code_hash, bytecode)
    }

    /// Write an account by its address.
    fn put_account(&self, address: &Address, account: &Account) -> Result<(), Self::Error> {
        self.queue_put::<tables::PlainAccountState>(address, account)
    }

    /// Write a storage entry by its address and key.
    fn put_storage(&self, address: &Address, key: &B256, entry: &U256) -> Result<(), Self::Error> {
        self.queue_put_dual::<tables::PlainStorageState>(address, key, entry)
    }

    /// Write a sealed block header (header + number).
    fn put_header(&self, header: &SealedHeader) -> Result<(), Self::Error> {
        self.put_header_inconsistent(header.header())
            .and_then(|_| self.put_header_number_inconsistent(&header.hash(), header.number))
    }

    /// Delete a header by block number.
    fn delete_header(&self, number: u64) -> Result<(), Self::Error> {
        self.queue_delete::<tables::Headers>(&number)
    }

    /// Delete a header number mapping by hash.
    fn delete_header_number(&self, hash: &B256) -> Result<(), Self::Error> {
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
        &self,
        address: &Address,
        latest_height: u64,
        touched: &BlockNumberList,
    ) -> Result<(), Self::Error> {
        self.queue_put_dual::<tables::AccountsHistory>(address, &latest_height, touched)
    }

    /// Write an account change (pre-state) for an account at a specific
    /// block.
    fn write_account_prestate(
        &self,
        block_number: u64,
        address: Address,
        pre_state: &Account,
    ) -> Result<(), Self::Error> {
        self.queue_put_dual::<tables::AccountChangeSets>(&block_number, &address, pre_state)
    }

    /// Write storage history, by highest block number and touched block
    /// numbers.
    fn write_storage_history(
        &self,
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
        &self,
        block_number: u64,
        address: Address,
        slot: &B256,
        prestate: &U256,
    ) -> Result<(), Self::Error> {
        let block_number_address = BlockNumberAddress((block_number, address));
        self.queue_put_dual::<tables::StorageChangeSets>(&block_number_address, slot, prestate)
    }

    /// Write a pre-state for every storage key that exists for an account at a
    /// specific block.
    fn write_wipe(&self, block_number: u64, address: &Address) -> Result<(), Self::Error> {
        // SAFETY: the cursor is scoped to the transaction lifetime, which is
        // valid for the duration of this method.
        let mut cursor = self.traverse_dual::<tables::PlainStorageState>()?;

        let Some(start) = cursor.next_dual_above(address, &B256::ZERO)? else {
            // No storage entries at or above this address
            return Ok(());
        };

        if start.0 != *address {
            // No storage entries for this address
            return Ok(());
        }

        self.write_storage_prestate(block_number, *address, &start.1, &start.2)?;

        while let Some((k, k2, v)) = cursor.next_k2()? {
            if k != *address {
                break;
            }

            self.write_storage_prestate(block_number, *address, &k2, &v)?;
        }

        Ok(())
    }

    /// Write a block's plain state revert information.
    fn write_plain_revert(
        &self,
        block_number: u64,
        accounts: &[(Address, Option<AccountInfo>)],
        storage: &[PlainStorageRevert],
    ) -> Result<(), Self::Error> {
        for (address, info) in accounts {
            let account = info.as_ref().map(Account::from).unwrap_or_default();

            if let Some(bytecode) = info.as_ref().and_then(|info| info.code.clone()) {
                let code_hash = account.bytecode_hash.expect("info has bytecode; hash must exist");
                let bytecode = Bytecode(bytecode);
                self.put_bytecode(&code_hash, &bytecode)?;
            }

            self.write_account_prestate(block_number, *address, &account)?;
        }

        for entry in storage {
            if entry.wiped {
                return self.write_wipe(block_number, &entry.address);
            }
            for (key, old_value) in entry.storage_revert.iter() {
                self.write_storage_prestate(
                    block_number,
                    entry.address,
                    &B256::from(key.to_be_bytes()),
                    &old_value.to_previous_value(),
                )?;
            }
        }

        Ok(())
    }

    /// Write multiple blocks' plain state revert information.
    fn write_plain_reverts(
        &self,
        first_block_number: u64,
        PlainStateReverts { accounts, storage }: &PlainStateReverts,
    ) -> Result<(), Self::Error> {
        accounts.iter().zip(storage.iter()).enumerate().try_for_each(|(idx, (acc, sto))| {
            self.write_plain_revert(first_block_number + idx as u64, acc, sto)
        })
    }
}

impl<T> UnsafeHistoryWrite for T where T: UnsafeDbWrite + HotKvWrite {}
