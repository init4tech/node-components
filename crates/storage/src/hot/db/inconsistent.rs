use std::{
    collections::{BTreeMap, HashMap, hash_map},
    ops::RangeInclusive,
};

use crate::hot::{
    db::{HistoryError, HistoryRead},
    model::{DualKeyTraverse, DualTableCursor, HotKvWrite, KvTraverse, TableCursor},
    tables,
};
use alloy::primitives::{Address, B256, BlockNumber, U256};
use itertools::Itertools;
use reth::{
    primitives::{Account, Header, SealedHeader},
    revm::db::BundleState,
};
use reth_db::{
    BlockNumberList,
    models::{BlockNumberAddress, sharded_key},
};
use reth_db_api::models::ShardedKey;
use trevm::revm::{
    bytecode::Bytecode,
    database::{
        OriginalValuesKnown,
        states::{PlainStateReverts, PlainStorageChangeset, PlainStorageRevert, StateChangeset},
    },
    state::AccountInfo,
};

/// Bundle state initialization type.
/// Maps address -> (old_account, new_account, storage_changes)
/// where storage_changes maps slot (B256) -> (old_value, new_value)
pub type BundleInit =
    HashMap<Address, (Option<Account>, Option<Account>, HashMap<B256, (U256, U256)>)>;

/// Trait for database write operations on standard hot tables.
///
/// This trait is low-level, and usage may leave the database in an
/// inconsistent state if not used carefully. Users should prefer
/// [`HotHistoryWrite`] or higher-level abstractions when possible.
///
/// [`HotHistoryWrite`]: crate::hot::db::HistoryWrite
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
    fn put_storage(&self, address: &Address, key: &U256, entry: &U256) -> Result<(), Self::Error> {
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
pub trait UnsafeHistoryWrite: UnsafeDbWrite + HistoryRead {
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
        slot: U256,
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
        slot: &U256,
        prestate: &U256,
    ) -> Result<(), Self::Error> {
        let block_number_address = BlockNumberAddress((block_number, address));
        self.queue_put_dual::<tables::StorageChangeSets>(&block_number_address, slot, prestate)
    }

    /// Write a pre-state for every storage key that exists for an account at a
    /// specific block.
    fn write_wipe(&self, block_number: u64, address: &Address) -> Result<(), Self::Error> {
        let mut cursor = self.traverse_dual::<tables::PlainStorageState>()?;

        cursor.for_each_k2(address, &U256::ZERO, |_addr, slot, value| {
            self.write_storage_prestate(block_number, *address, &slot, &value)
        })
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
                    key,
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

    /// Write changed accounts from a [`StateChangeset`].
    fn write_changed_account(
        &self,
        address: &Address,
        account: &Option<AccountInfo>,
    ) -> Result<(), Self::Error> {
        let Some(info) = account.as_ref() else {
            // Account removal
            return self.queue_delete::<tables::PlainAccountState>(address);
        };

        let account = Account::from(info.clone());
        if let Some(bytecode) = info.code.clone() {
            let code_hash = account.bytecode_hash.expect("info has bytecode; hash must exist");
            self.put_bytecode(&code_hash, &bytecode)?;
        }
        self.put_account(address, &account)
    }

    /// Write changed storage from a [`StateChangeset`].
    fn write_changed_storage(
        &self,
        PlainStorageChangeset { address, wipe_storage, storage }: &PlainStorageChangeset,
    ) -> Result<(), Self::Error> {
        if *wipe_storage {
            let mut cursor = self.traverse_dual_mut::<tables::PlainStorageState>()?;

            while let Some((key, _, _)) = cursor.next_k2()? {
                if key != *address {
                    break;
                }
                cursor.delete_current()?;
            }

            return Ok(());
        }

        storage.iter().try_for_each(|(key, value)| self.put_storage(address, key, value))
    }

    /// Write changed contract bytecode from a [`StateChangeset`].
    fn write_changed_contracts(
        &self,
        code_hash: &B256,
        bytecode: &Bytecode,
    ) -> Result<(), Self::Error> {
        self.put_bytecode(code_hash, bytecode)
    }

    /// Write a state changeset for a specific block.
    fn write_state_changes(
        &self,
        StateChangeset { accounts, storage, contracts }: &StateChangeset,
    ) -> Result<(), Self::Error> {
        contracts.iter().try_for_each(|(code_hash, bytecode)| {
            self.write_changed_contracts(code_hash, bytecode)
        })?;
        accounts
            .iter()
            .try_for_each(|(address, account)| self.write_changed_account(address, account))?;
        storage
            .iter()
            .try_for_each(|storage_changeset| self.write_changed_storage(storage_changeset))?;
        Ok(())
    }

    /// Get all changed accounts with the list of block numbers in the given
    /// range.
    ///
    /// Note: This iterates using `next_k2()` which stays within the same k1
    /// (block number). It effectively only collects changes from the first
    /// block number in the range.
    fn changed_accounts_with_range(
        &self,
        range: RangeInclusive<BlockNumber>,
    ) -> Result<BTreeMap<Address, Vec<u64>>, Self::Error> {
        let mut changeset_cursor = self.traverse_dual::<tables::AccountChangeSets>()?;
        let mut result: BTreeMap<Address, Vec<u64>> = BTreeMap::new();

        changeset_cursor.for_each_while_k2(
            range.start(),
            &Address::ZERO,
            |num, _, _| range.contains(num),
            |num, addr, _| {
                result.entry(addr).or_default().push(num);
                Ok(())
            },
        )?;

        Ok(result)
    }

    /// Append account history indices for multiple accounts.
    fn append_account_history_index(
        &self,
        index_updates: impl IntoIterator<Item = (Address, impl IntoIterator<Item = u64>)>,
    ) -> Result<(), HistoryError<Self::Error>> {
        for (acct, indices) in index_updates {
            // Get the existing last shard (if any) and remember its key so we can
            // delete it before writing new shards
            let existing = self.last_account_history(acct)?;
            let mut last_shard =
                existing.as_ref().map(|(_, list)| list.clone()).unwrap_or_default();

            last_shard.append(indices).map_err(HistoryError::IntList)?;

            // Delete the existing shard before writing new ones to avoid duplicates
            if let Some((old_key, _)) = existing {
                self.queue_delete_dual::<tables::AccountsHistory>(&acct, &old_key)?;
            }

            // fast path: all indices fit in one shard
            if last_shard.len() <= sharded_key::NUM_OF_INDICES_IN_SHARD as u64 {
                self.write_account_history(&acct, u64::MAX, &last_shard)?;
                continue;
            }

            // slow path: rechunk into multiple shards
            let chunks = last_shard.iter().chunks(sharded_key::NUM_OF_INDICES_IN_SHARD);

            let mut chunks = chunks.into_iter().peekable();

            while let Some(chunk) = chunks.next() {
                let shard = BlockNumberList::new_pre_sorted(chunk);
                let highest_block_number = if chunks.peek().is_some() {
                    shard.iter().next_back().expect("`chunks` does not return empty list")
                } else {
                    // Insert last list with `u64::MAX`.
                    u64::MAX
                };

                self.write_account_history(&acct, highest_block_number, &shard)?;
            }
        }
        Ok(())
    }

    /// Get all changed storages with the list of block numbers in the given
    /// range.
    ///
    /// Note: This iterates using `next_k2()` which stays within the same k1
    /// (block number + address). It effectively only collects changes from
    /// the first key1 value in the range.
    #[allow(clippy::type_complexity)]
    fn changed_storages_with_range(
        &self,
        range: RangeInclusive<BlockNumber>,
    ) -> Result<BTreeMap<(Address, U256), Vec<u64>>, Self::Error> {
        let mut changeset_cursor = self.traverse_dual::<tables::StorageChangeSets>()?;
        let mut result: BTreeMap<(Address, U256), Vec<u64>> = BTreeMap::new();

        changeset_cursor.for_each_while_k2(
            &BlockNumberAddress((*range.start(), Address::ZERO)),
            &U256::ZERO,
            |num_addr, _, _| range.contains(&num_addr.block_number()),
            |num_addr, slot, _| {
                result.entry((num_addr.address(), slot)).or_default().push(num_addr.block_number());
                Ok(())
            },
        )?;

        Ok(result)
    }

    /// Append storage history indices for multiple (address, slot) pairs.
    fn append_storage_history_index(
        &self,
        index_updates: impl IntoIterator<Item = ((Address, U256), impl IntoIterator<Item = u64>)>,
    ) -> Result<(), HistoryError<Self::Error>> {
        for ((addr, slot), indices) in index_updates {
            // Get the existing last shard (if any) and remember its key so we can
            // delete it before writing new shards
            let existing = self.last_storage_history(&addr, &slot)?;
            let mut last_shard =
                existing.as_ref().map(|(_, list)| list.clone()).unwrap_or_default();

            last_shard.append(indices).map_err(HistoryError::IntList)?;

            // Delete the existing shard before writing new ones to avoid duplicates
            if let Some((old_key, _)) = existing {
                self.queue_delete_dual::<tables::StorageHistory>(&addr, &old_key)?;
            }

            // fast path: all indices fit in one shard
            if last_shard.len() <= sharded_key::NUM_OF_INDICES_IN_SHARD as u64 {
                self.write_storage_history(&addr, slot, u64::MAX, &last_shard)?;
                continue;
            }

            // slow path: rechunk into multiple shards
            let chunks = last_shard.iter().chunks(sharded_key::NUM_OF_INDICES_IN_SHARD);

            let mut chunks = chunks.into_iter().peekable();

            while let Some(chunk) = chunks.next() {
                let shard = BlockNumberList::new_pre_sorted(chunk);
                let highest_block_number = if chunks.peek().is_some() {
                    shard.iter().next_back().expect("`chunks` does not return empty list")
                } else {
                    // Insert last list with `u64::MAX`.
                    u64::MAX
                };

                self.write_storage_history(&addr, slot, highest_block_number, &shard)?;
            }
        }
        Ok(())
    }

    /// Update the history indices for accounts and storage in the given block
    /// range.
    fn update_history_indices_inconsistent(
        &self,
        range: RangeInclusive<BlockNumber>,
    ) -> Result<(), HistoryError<Self::Error>> {
        // account history stage
        {
            let indices = self.changed_accounts_with_range(range.clone())?;
            self.append_account_history_index(indices)?;
        }

        // storage history stage
        {
            let indices = self.changed_storages_with_range(range)?;
            self.append_storage_history_index(indices)?;
        }

        Ok(())
    }

    /// Append a block's header and state changes in an inconsistent manner.
    ///
    /// This may leave the database in an inconsistent state. Users should
    /// prefer higher-level abstractions when possible.
    ///
    /// 1. It MUST be checked that the header is the child of the current chain
    ///    tip before calling this method.
    /// 2. After calling this method, the caller MUST call
    ///    `update_history_indices`.
    fn append_block_inconsistent(
        &self,
        header: &SealedHeader,
        state_changes: &BundleState,
    ) -> Result<(), Self::Error> {
        self.put_header_inconsistent(header.header())?;
        self.put_header_number_inconsistent(&header.hash(), header.number)?;

        let (state_changes, reverts) =
            state_changes.to_plain_state_and_reverts(OriginalValuesKnown::No);

        self.write_state_changes(&state_changes)?;
        self.write_plain_reverts(header.number, &reverts)
    }

    /// Append multiple blocks' headers and state changes in an inconsistent
    /// manner.
    ///
    /// This may leave the database in an inconsistent state. Users should
    /// prefer higher-level abstractions when possible.
    /// 1. It MUST be checked that the first header is the child of the current
    ///    chain tip before calling this method.
    /// 2. After calling this method, the caller MUST call
    ///    `update_history_indices`.
    fn append_blocks_inconsistent(
        &self,
        blocks: &[(SealedHeader, BundleState)],
    ) -> Result<(), Self::Error> {
        blocks.iter().try_for_each(|(header, state)| self.append_block_inconsistent(header, state))
    }

    /// Populate a [`BundleInit`] using cursors over the
    /// [`tables::PlainAccountState`] and [`tables::PlainStorageState`] tables,
    /// based on the given storage and account changesets.
    ///
    /// Returns a map of address -> (old_account, new_account, storage_changes)
    /// where storage_changes maps slot -> (old_value, new_value).
    fn populate_bundle_state<C, D>(
        &self,
        account_changeset: Vec<(u64, Address, Account)>,
        storage_changeset: Vec<(BlockNumberAddress, U256, U256)>,
        plain_accounts_cursor: &mut TableCursor<C, tables::PlainAccountState, Self::Error>,
        plain_storage_cursor: &mut DualTableCursor<D, tables::PlainStorageState, Self::Error>,
    ) -> Result<BundleInit, Self::Error>
    where
        C: KvTraverse<Self::Error>,
        D: DualKeyTraverse<Self::Error>,
    {
        // iterate previous value and get plain state value to create changeset
        // Double option around Account represent if Account state is known (first option) and
        // account is removed (second option)
        let mut state: BundleInit = Default::default();

        // add account changeset changes in reverse order
        for (_block_number, address, old_account) in account_changeset.into_iter().rev() {
            match state.entry(address) {
                hash_map::Entry::Vacant(entry) => {
                    let new_account = plain_accounts_cursor.exact(&address)?;
                    entry.insert((Some(old_account), new_account, HashMap::default()));
                }
                hash_map::Entry::Occupied(mut entry) => {
                    // overwrite old account state.
                    entry.get_mut().0 = Some(old_account);
                }
            }
        }

        // add storage changeset changes
        for (block_and_address, storage_key, old_value) in storage_changeset.into_iter().rev() {
            let BlockNumberAddress((_block_number, address)) = block_and_address;
            // get account state or insert from plain state.
            let account_state = match state.entry(address) {
                hash_map::Entry::Vacant(entry) => {
                    let present_account = plain_accounts_cursor.exact(&address)?;
                    entry.insert((present_account, present_account, HashMap::default()))
                }
                hash_map::Entry::Occupied(entry) => entry.into_mut(),
            };

            // Convert U256 storage key to B256 for the BundleInit map
            let storage_key_b256 = B256::from(storage_key);

            // match storage.
            match account_state.2.entry(storage_key_b256) {
                hash_map::Entry::Vacant(entry) => {
                    let new_value = plain_storage_cursor
                        .next_dual_above(&address, &storage_key)?
                        .filter(|(k, k2, _)| *k == address && *k2 == storage_key)
                        .map(|(_, _, v)| v)
                        .unwrap_or_default();
                    entry.insert((old_value, new_value));
                }
                hash_map::Entry::Occupied(mut entry) => {
                    entry.get_mut().0 = old_value;
                }
            };
        }

        Ok(state)
    }
}

impl<T> UnsafeHistoryWrite for T where T: UnsafeDbWrite + HotKvWrite {}
