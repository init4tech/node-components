use crate::hot::{
    db::{HistoryError, UnsafeDbWrite, UnsafeHistoryWrite},
    tables,
};
use alloy::primitives::{Address, BlockNumber, U256, address};
use reth::primitives::SealedHeader;
use reth_db::{BlockNumberList, models::BlockNumberAddress};
use std::collections::HashSet;
use trevm::revm::database::BundleState;

/// Maximum address value (all bits set to 1).
const ADDRESS_MAX: Address = address!("0xffffffffffffffffffffffffffffffffffffffff");

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

    /// Unwind all data above the given block number.
    ///
    /// This completely reverts the database state to what it was at block `block`,
    /// including:
    /// - Plain account state
    /// - Plain storage state
    /// - Headers and header number mappings
    /// - Account and storage change sets
    /// - Account and storage history indices
    fn unwind_above(&self, block: BlockNumber) -> Result<(), HistoryError<Self::Error>> {
        let first_block_number = block + 1;
        let Some(last_block_number) = self.last_block_number()? else {
            return Ok(());
        };

        if first_block_number > last_block_number {
            return Ok(());
        }

        // 1. Take and process changesets (reverts plain state)
        let storage_range = (BlockNumberAddress((first_block_number, Address::ZERO)), U256::ZERO)
            ..=(BlockNumberAddress((last_block_number, ADDRESS_MAX)), U256::MAX);
        let acct_range = (first_block_number, Address::ZERO)..=(last_block_number, ADDRESS_MAX);

        let storage_changeset = self.take_range_dual::<tables::StorageChangeSets>(storage_range)?;
        let account_changeset = self.take_range_dual::<tables::AccountChangeSets>(acct_range)?;

        // Collect affected addresses and slots for history cleanup
        let mut affected_addresses: HashSet<Address> = HashSet::new();
        let mut affected_storage: HashSet<(Address, U256)> = HashSet::new();

        for (_, address, _) in &account_changeset {
            affected_addresses.insert(*address);
        }
        for (block_addr, slot, _) in &storage_changeset {
            affected_storage.insert((block_addr.address(), *slot));
        }

        // Revert plain state using existing logic
        let mut plain_accounts_cursor = self.traverse_mut::<tables::PlainAccountState>()?;
        let mut plain_storage_cursor = self.traverse_dual_mut::<tables::PlainStorageState>()?;

        let state = self.populate_bundle_state(
            account_changeset,
            storage_changeset,
            &mut plain_accounts_cursor,
            &mut plain_storage_cursor,
        )?;

        for (address, (old_account, new_account, storage)) in &state {
            if old_account != new_account {
                let existing_entry = plain_accounts_cursor.lower_bound(address)?;
                if let Some(account) = old_account {
                    // Check if the old account is effectively empty (account didn't exist before)
                    // An empty account has nonce=0, balance=0, no bytecode
                    let is_empty = account.nonce == 0
                        && account.balance.is_zero()
                        && account.bytecode_hash.is_none();

                    if is_empty {
                        // Account was created - delete it
                        if existing_entry.is_some_and(|(k, _)| k == *address) {
                            plain_accounts_cursor.delete_current()?;
                        }
                    } else {
                        // Account existed before - restore it
                        self.put_account(address, account)?;
                    }
                } else if existing_entry.is_some_and(|(k, _)| k == *address) {
                    plain_accounts_cursor.delete_current()?;
                }
            }

            for (storage_key_b256, (old_storage_value, _)) in storage {
                let storage_key = U256::from_be_bytes(storage_key_b256.0);

                if plain_storage_cursor
                    .next_dual_above(address, &storage_key)?
                    .is_some_and(|(k, k2, _)| k == *address && k2 == storage_key)
                {
                    plain_storage_cursor.delete_current()?;
                }

                if !old_storage_value.is_zero() {
                    self.put_storage(address, &storage_key, old_storage_value)?;
                }
            }
        }

        // 2. Remove headers and header number mappings
        let removed_headers =
            self.take_range::<tables::Headers>(first_block_number..=last_block_number)?;
        for (_, header) in removed_headers {
            let hash = header.hash_slow();
            self.delete_header_number(&hash)?;
        }

        // 3. Clean up account history indices
        for address in affected_addresses {
            if let Some((shard_key, list)) = self.last_account_history(address)? {
                let filtered: Vec<u64> = list.iter().filter(|&bn| bn <= block).collect();
                self.queue_delete_dual::<tables::AccountsHistory>(&address, &shard_key)?;
                if !filtered.is_empty() {
                    let new_list = BlockNumberList::new_pre_sorted(filtered);
                    self.write_account_history(&address, u64::MAX, &new_list)?;
                }
            }
        }

        // 4. Clean up storage history indices
        for (address, slot) in affected_storage {
            if let Some((shard_key, list)) = self.last_storage_history(&address, &slot)? {
                let filtered: Vec<u64> = list.iter().filter(|&bn| bn <= block).collect();
                self.queue_delete_dual::<tables::StorageHistory>(&address, &shard_key)?;
                if !filtered.is_empty() {
                    let new_list = BlockNumberList::new_pre_sorted(filtered);
                    self.write_storage_history(&address, slot, u64::MAX, &new_list)?;
                }
            }
        }

        Ok(())
    }
}

impl<T> HistoryWrite for T where T: UnsafeDbWrite + UnsafeHistoryWrite {}
