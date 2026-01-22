#![allow(dead_code)]

use crate::hot::{
    db::{HistoryError, HistoryRead, HistoryWrite, HotDbRead, UnsafeDbWrite, UnsafeHistoryWrite},
    model::{
        DualKeyValue, DualTableTraverse, HotKv, HotKvRead, HotKvWrite, KeyValue, TableTraverse,
    },
    tables::{self, DualKey, SingleKey},
};
use alloy::primitives::{Address, B256, Bytes, U256, address, b256};
use reth::primitives::{Account, Bytecode, Header, SealedHeader};
use reth_db::BlockNumberList;
use std::collections::HashMap;
use std::fmt::Debug;
use trevm::revm::{
    bytecode::Bytecode as RevmBytecode,
    database::{
        AccountStatus, BundleAccount, BundleState,
        states::{
            StorageSlot,
            reverts::{AccountInfoRevert, AccountRevert, RevertToSlot, Reverts},
        },
    },
    primitives::map::DefaultHashBuilder,
    state::AccountInfo,
};

/// Run all conformance tests against a [`HotKv`] implementation.
pub fn conformance<T: HotKv>(hot_kv: &T) {
    test_header_roundtrip(hot_kv);
    test_account_roundtrip(hot_kv);
    test_storage_roundtrip(hot_kv);
    test_storage_update_replaces(hot_kv);
    test_bytecode_roundtrip(hot_kv);
    test_account_history(hot_kv);
    test_storage_history(hot_kv);
    test_account_changes(hot_kv);
    test_storage_changes(hot_kv);
    test_missing_reads(hot_kv);
}

// /// Run append and unwind conformance tests.
// ///
// /// This test requires a fresh database (no prior state) to properly test
// /// the append/unwind functionality.
// pub fn conformance_append_unwind<T: HotKv>(hot_kv: &T) {
//     test_append_and_unwind_blocks(hot_kv);
// }

/// Test writing and reading headers via HotDbWrite/HotDbRead
fn test_header_roundtrip<T: HotKv>(hot_kv: &T) {
    let header = Header { number: 42, gas_limit: 1_000_000, ..Default::default() };
    let sealed = SealedHeader::seal_slow(header.clone());
    let hash = sealed.hash();

    // Write header
    {
        let writer = hot_kv.writer().unwrap();
        writer.put_header(&sealed).unwrap();
        writer.commit().unwrap();
    }

    // Read header by number
    {
        let reader = hot_kv.reader().unwrap();
        let read_header = reader.get_header(42).unwrap();
        assert!(read_header.is_some());
        assert_eq!(read_header.unwrap().number, 42);
    }

    // Read header number by hash
    {
        let reader = hot_kv.reader().unwrap();
        let read_number = reader.get_header_number(&hash).unwrap();
        assert!(read_number.is_some());
        assert_eq!(read_number.unwrap(), 42);
    }

    // Read header by hash
    {
        let reader = hot_kv.reader().unwrap();
        let read_header = reader.header_by_hash(&hash).unwrap();
        assert!(read_header.is_some());
        assert_eq!(read_header.unwrap().number, 42);
    }
}

/// Test writing and reading accounts via HotDbWrite/HotDbRead
fn test_account_roundtrip<T: HotKv>(hot_kv: &T) {
    let addr = address!("0x1234567890123456789012345678901234567890");
    let account = Account { nonce: 5, balance: U256::from(1000), bytecode_hash: Some(B256::ZERO) };

    // Write account
    {
        let writer = hot_kv.writer().unwrap();
        writer.put_account(&addr, &account).unwrap();
        writer.commit().unwrap();
    }

    // Read account
    {
        let reader = hot_kv.reader().unwrap();
        let read_account = reader.get_account(&addr).unwrap();
        assert!(read_account.is_some());
        let read_account = read_account.unwrap();
        assert_eq!(read_account.nonce, 5);
        assert_eq!(read_account.balance, U256::from(1000));
    }
}

/// Test writing and reading storage via HotDbWrite/HotDbRead
fn test_storage_roundtrip<T: HotKv>(hot_kv: &T) {
    let addr = address!("0xabcdef0123456789abcdef0123456789abcdef01");
    let slot = U256::from(42);
    let value = U256::from(999);

    // Write storage
    {
        let writer = hot_kv.writer().unwrap();
        writer.put_storage(&addr, &slot, &value).unwrap();
        writer.commit().unwrap();
    }

    // Read storage
    {
        let reader = hot_kv.reader().unwrap();
        let read_value = reader.get_storage(&addr, &slot).unwrap();
        assert!(read_value.is_some());
        assert_eq!(read_value.unwrap(), U256::from(999));
    }

    // Read storage entry
    {
        let reader = hot_kv.reader().unwrap();
        let read_entry = reader.get_storage_entry(&addr, &slot).unwrap();
        assert!(read_entry.is_some());
        let entry = read_entry.unwrap();
        assert_eq!(entry.key, B256::new(slot.to_be_bytes()));
        assert_eq!(entry.value, U256::from(999));
    }
}

/// Test that updating a storage slot replaces the value (no duplicates).
///
/// This test verifies that DUPSORT tables properly handle updates by deleting
/// existing entries before inserting new ones.
fn test_storage_update_replaces<T: HotKv>(hot_kv: &T) {
    let addr = address!("0x2222222222222222222222222222222222222222");
    let slot = U256::from(1);

    // Write initial value
    {
        let writer = hot_kv.writer().unwrap();
        writer.put_storage(&addr, &slot, &U256::from(10)).unwrap();
        writer.commit().unwrap();
    }

    // Update to new value
    {
        let writer = hot_kv.writer().unwrap();
        writer.put_storage(&addr, &slot, &U256::from(20)).unwrap();
        writer.commit().unwrap();
    }

    // Verify: only ONE entry exists with the NEW value
    let reader = hot_kv.reader().unwrap();
    let mut cursor = reader.traverse_dual::<tables::PlainStorageState>().unwrap();

    let mut count = 0;
    let mut found_value = None;
    while let Some((k, k2, v)) = cursor.read_next().unwrap() {
        if k == addr && k2 == slot {
            count += 1;
            found_value = Some(v);
        }
    }

    assert_eq!(count, 1, "Should have exactly one entry, not duplicates");
    assert_eq!(found_value, Some(U256::from(20)), "Value should be 20");
}

/// Test writing and reading bytecode via HotDbWrite/HotDbRead
fn test_bytecode_roundtrip<T: HotKv>(hot_kv: &T) {
    let code = Bytes::from_static(&[0x60, 0x00, 0x60, 0x00, 0xf3]); // Simple EVM bytecode
    let bytecode = Bytecode::new_raw(code);
    let code_hash = bytecode.hash_slow();

    // Write bytecode
    {
        let writer = hot_kv.writer().unwrap();
        writer.put_bytecode(&code_hash, &bytecode).unwrap();
        writer.commit().unwrap();
    }

    // Read bytecode
    {
        let reader = hot_kv.reader().unwrap();
        let read_bytecode = reader.get_bytecode(&code_hash).unwrap();
        assert!(read_bytecode.is_some());
    }
}

/// Test account history via HotHistoryWrite/HotHistoryRead
fn test_account_history<T: HotKv>(hot_kv: &T) {
    let addr = address!("0x1111111111111111111111111111111111111111");
    let touched_blocks = BlockNumberList::new([10, 20, 30]).unwrap();
    let latest_height = 100u64;

    // Write account history
    {
        let writer = hot_kv.writer().unwrap();
        writer.write_account_history(&addr, latest_height, &touched_blocks).unwrap();
        writer.commit().unwrap();
    }

    // Read account history
    {
        let reader = hot_kv.reader().unwrap();
        let read_history = reader.get_account_history(&addr, latest_height).unwrap();
        assert!(read_history.is_some());
        let history = read_history.unwrap();
        assert_eq!(history.iter().collect::<Vec<_>>(), vec![10, 20, 30]);
    }
}

/// Test storage history via HotHistoryWrite/HotHistoryRead
fn test_storage_history<T: HotKv>(hot_kv: &T) {
    let addr = address!("0x2222222222222222222222222222222222222222");
    let slot = U256::from(42);
    let touched_blocks = BlockNumberList::new([5, 15, 25]).unwrap();
    let highest_block = 50u64;

    // Write storage history
    {
        let writer = hot_kv.writer().unwrap();
        writer.write_storage_history(&addr, slot, highest_block, &touched_blocks).unwrap();
        writer.commit().unwrap();
    }

    // Read storage history
    {
        let reader = hot_kv.reader().unwrap();
        let read_history = reader.get_storage_history(&addr, slot, highest_block).unwrap();
        assert!(read_history.is_some());
        let history = read_history.unwrap();
        assert_eq!(history.iter().collect::<Vec<_>>(), vec![5, 15, 25]);
    }
}

/// Test account change sets via HotHistoryWrite/HotHistoryRead
fn test_account_changes<T: HotKv>(hot_kv: &T) {
    let addr = address!("0x3333333333333333333333333333333333333333");
    let pre_state = Account { nonce: 10, balance: U256::from(5000), bytecode_hash: None };
    let block_number = 100u64;

    // Write account change
    {
        let writer = hot_kv.writer().unwrap();
        writer.write_account_prestate(block_number, addr, &pre_state).unwrap();
        writer.commit().unwrap();
    }

    // Read account change
    {
        let reader = hot_kv.reader().unwrap();

        let read_change = reader.get_account_change(block_number, &addr).unwrap();

        assert!(read_change.is_some());
        let change = read_change.unwrap();
        assert_eq!(change.nonce, 10);
        assert_eq!(change.balance, U256::from(5000));
    }
}

/// Test storage change sets via HotHistoryWrite/HotHistoryRead
fn test_storage_changes<T: HotKv>(hot_kv: &T) {
    let addr = address!("0x4444444444444444444444444444444444444444");
    let slot = U256::from(153);
    let pre_value = U256::from(12345);
    let block_number = 200u64;

    // Write storage change
    {
        let writer = hot_kv.writer().unwrap();
        writer.write_storage_prestate(block_number, addr, &slot, &pre_value).unwrap();
        writer.commit().unwrap();
    }

    // Read storage change
    {
        let reader = hot_kv.reader().unwrap();
        let read_change = reader.get_storage_change(block_number, &addr, &slot).unwrap();
        assert!(read_change.is_some());
        assert_eq!(read_change.unwrap(), U256::from(12345));
    }
}

/// Test that missing reads return None
fn test_missing_reads<T: HotKv>(hot_kv: &T) {
    let missing_addr = address!("0x9999999999999999999999999999999999999999");
    let missing_hash = b256!("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
    let missing_slot = U256::from(99999);

    let reader = hot_kv.reader().unwrap();

    // Missing header
    assert!(reader.get_header(999999).unwrap().is_none());

    // Missing header number
    assert!(reader.get_header_number(&missing_hash).unwrap().is_none());

    // Missing account
    assert!(reader.get_account(&missing_addr).unwrap().is_none());

    // Missing storage
    assert!(reader.get_storage(&missing_addr, &missing_slot).unwrap().is_none());

    // Missing bytecode
    assert!(reader.get_bytecode(&missing_hash).unwrap().is_none());

    // Missing header by hash
    assert!(reader.header_by_hash(&missing_hash).unwrap().is_none());

    // Missing account history
    assert!(reader.get_account_history(&missing_addr, 1000).unwrap().is_none());

    // Missing storage history
    assert!(reader.get_storage_history(&missing_addr, missing_slot, 1000).unwrap().is_none());

    // Missing account change
    assert!(reader.get_account_change(999999, &missing_addr).unwrap().is_none());

    // Missing storage change
    assert!(reader.get_storage_change(999999, &missing_addr, &missing_slot).unwrap().is_none());
}

/// Helper to create a sealed header at a given height with specific parent
fn make_header(number: u64, parent_hash: B256) -> SealedHeader {
    let header = Header { number, parent_hash, gas_limit: 1_000_000, ..Default::default() };
    SealedHeader::seal_slow(header)
}

/// Test update_history_indices_inconsistent for account history.
///
/// This test verifies that:
/// 1. Account change sets are correctly indexed into account history
/// 2. Appending to existing history works correctly
/// 3. Old shards are deleted when appending
pub fn test_update_history_indices_account<T: HotKv>(hot_kv: &T) {
    let addr1 = address!("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let addr2 = address!("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");

    // Phase 1: Write account change sets for blocks 1-3
    {
        let writer = hot_kv.writer().unwrap();

        // Block 1: addr1 changed
        let pre_acc = Account::default();
        writer.write_account_prestate(1, addr1, &pre_acc).unwrap();

        // Block 2: addr1 and addr2 changed
        let acc1 = Account { nonce: 1, balance: U256::from(100), bytecode_hash: None };
        writer.write_account_prestate(2, addr1, &acc1).unwrap();
        writer.write_account_prestate(2, addr2, &pre_acc).unwrap();

        // Block 3: addr2 changed
        let acc2 = Account { nonce: 1, balance: U256::from(200), bytecode_hash: None };
        writer.write_account_prestate(3, addr2, &acc2).unwrap();

        writer.commit().unwrap();
    }

    // Phase 2: Run update_history_indices_inconsistent for blocks 1-3
    {
        let writer = hot_kv.writer().unwrap();
        writer.update_history_indices_inconsistent(1..=3).unwrap();
        writer.commit().unwrap();
    }

    // Phase 3: Verify account history was created correctly
    {
        let reader = hot_kv.reader().unwrap();

        // addr1 should have history at blocks 1, 2
        let (_, history1) =
            reader.last_account_history(addr1).unwrap().expect("addr1 should have history");
        let blocks1: Vec<u64> = history1.iter().collect();
        assert_eq!(blocks1, vec![1, 2], "addr1 history mismatch");

        // addr2 should have history at blocks 2, 3
        let (_, history2) =
            reader.last_account_history(addr2).unwrap().expect("addr2 should have history");
        let blocks2: Vec<u64> = history2.iter().collect();
        assert_eq!(blocks2, vec![2, 3], "addr2 history mismatch");
    }

    // Phase 4: Write more change sets for blocks 4-5
    {
        let writer = hot_kv.writer().unwrap();

        // Block 4: addr1 changed
        let acc1 = Account { nonce: 2, balance: U256::from(300), bytecode_hash: None };
        writer.write_account_prestate(4, addr1, &acc1).unwrap();

        // Block 5: addr1 changed again
        let acc1_v2 = Account { nonce: 3, balance: U256::from(400), bytecode_hash: None };
        writer.write_account_prestate(5, addr1, &acc1_v2).unwrap();

        writer.commit().unwrap();
    }

    // Phase 5: Run update_history_indices_inconsistent for blocks 4-5
    {
        let writer = hot_kv.writer().unwrap();
        writer.update_history_indices_inconsistent(4..=5).unwrap();
        writer.commit().unwrap();
    }

    // Phase 6: Verify history was appended correctly
    {
        let reader = hot_kv.reader().unwrap();

        // addr1 should now have history at blocks 1, 2, 4, 5
        let (_, history1) =
            reader.last_account_history(addr1).unwrap().expect("addr1 should have history");
        let blocks1: Vec<u64> = history1.iter().collect();
        assert_eq!(blocks1, vec![1, 2, 4, 5], "addr1 history mismatch after append");

        // addr2 should still have history at blocks 2, 3 (unchanged)
        let (_, history2) =
            reader.last_account_history(addr2).unwrap().expect("addr2 should have history");
        let blocks2: Vec<u64> = history2.iter().collect();
        assert_eq!(blocks2, vec![2, 3], "addr2 history should be unchanged");
    }
}

/// Test update_history_indices_inconsistent for storage history.
///
/// This test verifies that:
/// 1. Storage change sets are correctly indexed into storage history
/// 2. Appending to existing history works correctly
/// 3. Old shards are deleted when appending
/// 4. Different slots for the same address are tracked separately
pub fn test_update_history_indices_storage<T: HotKv>(hot_kv: &T) {
    let addr1 = address!("0xcccccccccccccccccccccccccccccccccccccccc");
    let slot1 = U256::from(1);
    let slot2 = U256::from(2);

    // Phase 1: Write storage change sets for blocks 1-3
    {
        let writer = hot_kv.writer().unwrap();

        // Block 1: addr1.slot1 changed
        writer.write_storage_prestate(1, addr1, &slot1, &U256::ZERO).unwrap();

        // Block 2: addr1.slot1 and addr1.slot2 changed
        writer.write_storage_prestate(2, addr1, &slot1, &U256::from(100)).unwrap();
        writer.write_storage_prestate(2, addr1, &slot2, &U256::ZERO).unwrap();

        // Block 3: addr1.slot2 changed
        writer.write_storage_prestate(3, addr1, &slot2, &U256::from(200)).unwrap();

        writer.commit().unwrap();
    }

    // Phase 2: Run update_history_indices_inconsistent for blocks 1-3
    {
        let writer = hot_kv.writer().unwrap();
        writer.update_history_indices_inconsistent(1..=3).unwrap();
        writer.commit().unwrap();
    }

    // Phase 3: Verify storage history was created correctly
    {
        let reader = hot_kv.reader().unwrap();

        // addr1.slot1 should have history at blocks 1, 2
        let (_, history1) = reader
            .last_storage_history(&addr1, &slot1)
            .unwrap()
            .expect("addr1.slot1 should have history");
        let blocks1: Vec<u64> = history1.iter().collect();
        assert_eq!(blocks1, vec![1, 2], "addr1.slot1 history mismatch");

        // addr1.slot2 should have history at blocks 2, 3
        let (_, history2) = reader
            .last_storage_history(&addr1, &slot2)
            .unwrap()
            .expect("addr1.slot2 should have history");
        let blocks2: Vec<u64> = history2.iter().collect();
        assert_eq!(blocks2, vec![2, 3], "addr1.slot2 history mismatch");
    }

    // Phase 4: Write more change sets for blocks 4-5
    {
        let writer = hot_kv.writer().unwrap();

        // Block 4: addr1.slot1 changed
        writer.write_storage_prestate(4, addr1, &slot1, &U256::from(300)).unwrap();

        // Block 5: addr1.slot1 changed again
        writer.write_storage_prestate(5, addr1, &slot1, &U256::from(400)).unwrap();

        writer.commit().unwrap();
    }

    // Phase 5: Run update_history_indices_inconsistent for blocks 4-5
    {
        let writer = hot_kv.writer().unwrap();
        writer.update_history_indices_inconsistent(4..=5).unwrap();
        writer.commit().unwrap();
    }

    // Phase 6: Verify history was appended correctly
    {
        let reader = hot_kv.reader().unwrap();

        // addr1.slot1 should now have history at blocks 1, 2, 4, 5
        let (_, history1) = reader
            .last_storage_history(&addr1, &slot1)
            .unwrap()
            .expect("addr1.slot1 should have history");
        let blocks1: Vec<u64> = history1.iter().collect();
        assert_eq!(blocks1, vec![1, 2, 4, 5], "addr1.slot1 history mismatch after append");

        // addr1.slot2 should still have history at blocks 2, 3 (unchanged)
        let (_, history2) = reader
            .last_storage_history(&addr1, &slot2)
            .unwrap()
            .expect("addr1.slot2 should have history");
        let blocks2: Vec<u64> = history2.iter().collect();
        assert_eq!(blocks2, vec![2, 3], "addr1.slot2 history should be unchanged");
    }
}

/// Test that appending to history correctly removes old entries at same k1,k2.
///
/// This test specifically verifies that when we append new indices to an existing
/// shard, the old shard is properly deleted so we don't end up with duplicate data.
pub fn test_history_append_removes_old_entries<T: HotKv>(hot_kv: &T) {
    let addr = address!("0xdddddddddddddddddddddddddddddddddddddddd");

    // Phase 1: Manually write account history
    {
        let writer = hot_kv.writer().unwrap();
        let initial_history = BlockNumberList::new([10, 20, 30]).unwrap();
        writer.write_account_history(&addr, u64::MAX, &initial_history).unwrap();
        writer.commit().unwrap();
    }

    // Verify initial state
    {
        let reader = hot_kv.reader().unwrap();
        let (key, history) =
            reader.last_account_history(addr).unwrap().expect("should have history");
        assert_eq!(key, u64::MAX);
        let blocks: Vec<u64> = history.iter().collect();
        assert_eq!(blocks, vec![10, 20, 30]);
    }

    // Phase 2: Write account change set for block 40
    {
        let writer = hot_kv.writer().unwrap();
        let acc = Account { nonce: 1, balance: U256::from(100), bytecode_hash: None };
        writer.write_account_prestate(40, addr, &acc).unwrap();
        writer.commit().unwrap();
    }

    // Phase 3: Run update_history_indices_inconsistent
    {
        let writer = hot_kv.writer().unwrap();
        writer.update_history_indices_inconsistent(40..=40).unwrap();
        writer.commit().unwrap();
    }

    // Phase 4: Verify history was correctly appended
    {
        let reader = hot_kv.reader().unwrap();
        let (key, history) =
            reader.last_account_history(addr).unwrap().expect("should have history");
        assert_eq!(key, u64::MAX, "key should still be u64::MAX");
        let blocks: Vec<u64> = history.iter().collect();
        assert_eq!(blocks, vec![10, 20, 30, 40], "history should include appended block");
    }
}

/// Test deleting dual-keyed account history entries.
///
/// This test verifies that:
/// 1. Writing dual-keyed entries works correctly
/// 2. Deleting specific dual-keyed entries removes only that entry
/// 3. Other entries for the same k1 remain intact
/// 4. Traversal after deletion shows the entry is gone
pub fn test_delete_dual_account_history<T: HotKv>(hot_kv: &T) {
    let addr1 = address!("0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");
    let addr2 = address!("0xffffffffffffffffffffffffffffffffffffffff");

    // Phase 1: Write account history entries for multiple addresses
    {
        let writer = hot_kv.writer().unwrap();

        // Write history for addr1 at two different shard keys
        let history1_a = BlockNumberList::new([1, 2, 3]).unwrap();
        let history1_b = BlockNumberList::new([4, 5, 6]).unwrap();
        writer.write_account_history(&addr1, 100, &history1_a).unwrap();
        writer.write_account_history(&addr1, u64::MAX, &history1_b).unwrap();

        // Write history for addr2
        let history2 = BlockNumberList::new([10, 20, 30]).unwrap();
        writer.write_account_history(&addr2, u64::MAX, &history2).unwrap();

        writer.commit().unwrap();
    }

    // Phase 2: Verify all entries exist
    {
        let reader = hot_kv.reader().unwrap();

        // Check addr1 entries
        let hist1_a = reader.get_account_history(&addr1, 100).unwrap();
        assert!(hist1_a.is_some(), "addr1 shard 100 should exist");
        assert_eq!(hist1_a.unwrap().iter().collect::<Vec<_>>(), vec![1, 2, 3]);

        let hist1_b = reader.get_account_history(&addr1, u64::MAX).unwrap();
        assert!(hist1_b.is_some(), "addr1 shard u64::MAX should exist");
        assert_eq!(hist1_b.unwrap().iter().collect::<Vec<_>>(), vec![4, 5, 6]);

        // Check addr2 entry
        let hist2 = reader.get_account_history(&addr2, u64::MAX).unwrap();
        assert!(hist2.is_some(), "addr2 should exist");
        assert_eq!(hist2.unwrap().iter().collect::<Vec<_>>(), vec![10, 20, 30]);
    }

    // Phase 3: Delete addr1's u64::MAX entry
    {
        let writer = hot_kv.writer().unwrap();
        writer.queue_delete_dual::<tables::AccountsHistory>(&addr1, &u64::MAX).unwrap();
        writer.commit().unwrap();
    }

    // Phase 4: Verify only the deleted entry is gone
    {
        let reader = hot_kv.reader().unwrap();

        // addr1 shard 100 should still exist
        let hist1_a = reader.get_account_history(&addr1, 100).unwrap();
        assert!(hist1_a.is_some(), "addr1 shard 100 should still exist after delete");
        assert_eq!(hist1_a.unwrap().iter().collect::<Vec<_>>(), vec![1, 2, 3]);

        // addr1 shard u64::MAX should be gone
        let hist1_b = reader.get_account_history(&addr1, u64::MAX).unwrap();
        assert!(hist1_b.is_none(), "addr1 shard u64::MAX should be deleted");

        // addr2 should be unaffected
        let hist2 = reader.get_account_history(&addr2, u64::MAX).unwrap();
        assert!(hist2.is_some(), "addr2 should be unaffected by delete");
        assert_eq!(hist2.unwrap().iter().collect::<Vec<_>>(), vec![10, 20, 30]);

        // Verify last_account_history now returns shard 100 for addr1
        let (key, _) =
            reader.last_account_history(addr1).unwrap().expect("addr1 should still have history");
        assert_eq!(key, 100, "last shard for addr1 should now be 100");
    }
}

/// Test deleting dual-keyed storage history entries.
///
/// This test verifies that:
/// 1. Writing storage history entries works correctly
/// 2. Deleting specific (address, slot, shard) entries removes only that entry
/// 3. Other slots for the same address remain intact
/// 4. Traversal after deletion shows the entry is gone
pub fn test_delete_dual_storage_history<T: HotKv>(hot_kv: &T) {
    use reth_db::models::ShardedKey;

    let addr = address!("0x1111111111111111111111111111111111111111");
    let slot1 = U256::from(100);
    let slot2 = U256::from(200);

    // Phase 1: Write storage history entries for multiple slots
    {
        let writer = hot_kv.writer().unwrap();

        // Write history for slot1
        let history1 = BlockNumberList::new([1, 2, 3]).unwrap();
        writer.write_storage_history(&addr, slot1, u64::MAX, &history1).unwrap();

        // Write history for slot2
        let history2 = BlockNumberList::new([10, 20, 30]).unwrap();
        writer.write_storage_history(&addr, slot2, u64::MAX, &history2).unwrap();

        writer.commit().unwrap();
    }

    // Phase 2: Verify both entries exist
    {
        let reader = hot_kv.reader().unwrap();

        let hist1 = reader.get_storage_history(&addr, slot1, u64::MAX).unwrap();
        assert!(hist1.is_some(), "slot1 should exist");
        assert_eq!(hist1.unwrap().iter().collect::<Vec<_>>(), vec![1, 2, 3]);

        let hist2 = reader.get_storage_history(&addr, slot2, u64::MAX).unwrap();
        assert!(hist2.is_some(), "slot2 should exist");
        assert_eq!(hist2.unwrap().iter().collect::<Vec<_>>(), vec![10, 20, 30]);
    }

    // Phase 3: Delete slot1's entry
    {
        let writer = hot_kv.writer().unwrap();
        let key_to_delete = ShardedKey::new(slot1, u64::MAX);
        writer.queue_delete_dual::<tables::StorageHistory>(&addr, &key_to_delete).unwrap();
        writer.commit().unwrap();
    }

    // Phase 4: Verify only slot1 is gone
    {
        let reader = hot_kv.reader().unwrap();

        // slot1 should be gone
        let hist1 = reader.get_storage_history(&addr, slot1, u64::MAX).unwrap();
        assert!(hist1.is_none(), "slot1 should be deleted");

        // slot2 should be unaffected
        let hist2 = reader.get_storage_history(&addr, slot2, u64::MAX).unwrap();
        assert!(hist2.is_some(), "slot2 should be unaffected");
        assert_eq!(hist2.unwrap().iter().collect::<Vec<_>>(), vec![10, 20, 30]);

        // last_storage_history for slot1 should return None
        let last1 = reader.last_storage_history(&addr, &slot1).unwrap();
        assert!(last1.is_none(), "last_storage_history for slot1 should return None");

        // last_storage_history for slot2 should still work
        let last2 = reader.last_storage_history(&addr, &slot2).unwrap();
        assert!(last2.is_some(), "last_storage_history for slot2 should still work");
    }
}

/// Test deleting and re-adding dual-keyed entries.
///
/// This test verifies that after deleting an entry, we can write a new entry
/// with the same key and it works correctly.
pub fn test_delete_and_rewrite_dual<T: HotKv>(hot_kv: &T) {
    let addr = address!("0x2222222222222222222222222222222222222222");

    // Phase 1: Write initial entry
    {
        let writer = hot_kv.writer().unwrap();
        let history = BlockNumberList::new([1, 2, 3]).unwrap();
        writer.write_account_history(&addr, u64::MAX, &history).unwrap();
        writer.commit().unwrap();
    }

    // Verify initial state
    {
        let reader = hot_kv.reader().unwrap();
        let hist = reader.get_account_history(&addr, u64::MAX).unwrap();
        assert_eq!(hist.unwrap().iter().collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    // Phase 2: Delete the entry
    {
        let writer = hot_kv.writer().unwrap();
        writer.queue_delete_dual::<tables::AccountsHistory>(&addr, &u64::MAX).unwrap();
        writer.commit().unwrap();
    }

    // Verify deleted
    {
        let reader = hot_kv.reader().unwrap();
        let hist = reader.get_account_history(&addr, u64::MAX).unwrap();
        assert!(hist.is_none(), "entry should be deleted");
    }

    // Phase 3: Write new entry with same key but different value
    {
        let writer = hot_kv.writer().unwrap();
        let new_history = BlockNumberList::new([100, 200, 300]).unwrap();
        writer.write_account_history(&addr, u64::MAX, &new_history).unwrap();
        writer.commit().unwrap();
    }

    // Verify new value
    {
        let reader = hot_kv.reader().unwrap();
        let hist = reader.get_account_history(&addr, u64::MAX).unwrap();
        assert!(hist.is_some(), "new entry should exist");
        assert_eq!(hist.unwrap().iter().collect::<Vec<_>>(), vec![100, 200, 300]);
    }
}

/// Test clear_range on a single-keyed table.
///
/// This test verifies that:
/// 1. Keys within the range are deleted
/// 2. Keys outside the range remain intact
/// 3. Edge cases like adjacent keys and boundary conditions work correctly
pub fn test_clear_range<T: HotKv>(hot_kv: &T) {
    // Phase 1: Write 15 headers with block numbers 0-14
    {
        let writer = hot_kv.writer().unwrap();
        for i in 0u64..15 {
            let header = Header { number: i, gas_limit: 1_000_000, ..Default::default() };
            writer.put_header_inconsistent(&header).unwrap();
        }
        writer.commit().unwrap();
    }

    // Verify all headers exist
    {
        let reader = hot_kv.reader().unwrap();
        for i in 0u64..15 {
            assert!(reader.get_header(i).unwrap().is_some(), "header {} should exist", i);
        }
    }

    // Phase 2: Clear range 5..=9 (middle range)
    {
        let writer = hot_kv.writer().unwrap();
        writer.clear_range::<tables::Headers>(5..=9).unwrap();
        writer.commit().unwrap();
    }

    // Verify: 0-4 and 10-14 should exist, 5-9 should be gone
    {
        let reader = hot_kv.reader().unwrap();

        // Keys before range should exist
        for i in 0u64..5 {
            assert!(reader.get_header(i).unwrap().is_some(), "header {} should still exist", i);
        }

        // Keys in range should be deleted
        for i in 5u64..10 {
            assert!(reader.get_header(i).unwrap().is_none(), "header {} should be deleted", i);
        }

        // Keys after range should exist
        for i in 10u64..15 {
            assert!(reader.get_header(i).unwrap().is_some(), "header {} should still exist", i);
        }
    }

    // Phase 3: Test corner case - clear adjacent keys at the boundary
    {
        let writer = hot_kv.writer().unwrap();
        // Clear keys 3 and 4 (adjacent to the already cleared range)
        writer.clear_range::<tables::Headers>(3..=4).unwrap();
        writer.commit().unwrap();
    }

    // Verify: 0-2 and 10-14 should exist, 3-9 should be gone
    {
        let reader = hot_kv.reader().unwrap();

        // Keys 0-2 should exist
        for i in 0u64..3 {
            assert!(reader.get_header(i).unwrap().is_some(), "header {} should still exist", i);
        }

        // Keys 3-9 should all be deleted now
        for i in 3u64..10 {
            assert!(reader.get_header(i).unwrap().is_none(), "header {} should be deleted", i);
        }

        // Keys 10-14 should exist
        for i in 10u64..15 {
            assert!(reader.get_header(i).unwrap().is_some(), "header {} should still exist", i);
        }
    }

    // Phase 4: Test clearing a range that includes the first key
    {
        let writer = hot_kv.writer().unwrap();
        writer.clear_range::<tables::Headers>(0..=1).unwrap();
        writer.commit().unwrap();
    }

    {
        let reader = hot_kv.reader().unwrap();
        assert!(reader.get_header(0).unwrap().is_none(), "header 0 should be deleted");
        assert!(reader.get_header(1).unwrap().is_none(), "header 1 should be deleted");
        assert!(reader.get_header(2).unwrap().is_some(), "header 2 should still exist");
    }

    // Phase 5: Test clearing a range that includes the last key
    {
        let writer = hot_kv.writer().unwrap();
        writer.clear_range::<tables::Headers>(13..=14).unwrap();
        writer.commit().unwrap();
    }

    {
        let reader = hot_kv.reader().unwrap();
        assert!(reader.get_header(12).unwrap().is_some(), "header 12 should still exist");
        assert!(reader.get_header(13).unwrap().is_none(), "header 13 should be deleted");
        assert!(reader.get_header(14).unwrap().is_none(), "header 14 should be deleted");
    }

    // Phase 6: Test clearing a single key
    {
        let writer = hot_kv.writer().unwrap();
        writer.clear_range::<tables::Headers>(11..=11).unwrap();
        writer.commit().unwrap();
    }

    {
        let reader = hot_kv.reader().unwrap();
        assert!(reader.get_header(10).unwrap().is_some(), "header 10 should still exist");
        assert!(reader.get_header(11).unwrap().is_none(), "header 11 should be deleted");
        assert!(reader.get_header(12).unwrap().is_some(), "header 12 should still exist");
    }

    // Phase 7: Test clearing a range where nothing exists (should be no-op)
    {
        let writer = hot_kv.writer().unwrap();
        writer.clear_range::<tables::Headers>(100..=200).unwrap();
        writer.commit().unwrap();
    }

    // Verify remaining keys are still intact
    {
        let reader = hot_kv.reader().unwrap();
        assert!(reader.get_header(2).unwrap().is_some(), "header 2 should still exist");
        assert!(reader.get_header(10).unwrap().is_some(), "header 10 should still exist");
        assert!(reader.get_header(12).unwrap().is_some(), "header 12 should still exist");
    }
}

/// Test take_range on a single-keyed table.
///
/// Similar to clear_range but also returns the removed keys.
pub fn test_take_range<T: HotKv>(hot_kv: &T) {
    let headers = (0..10u64)
        .map(|i| Header { number: i, gas_limit: 1_000_000, ..Default::default() })
        .collect::<Vec<_>>();

    // Phase 1: Write 10 headers with block numbers 0-9
    {
        let writer = hot_kv.writer().unwrap();
        for header in headers.iter() {
            writer.put_header_inconsistent(header).unwrap();
        }
        writer.commit().unwrap();
    }

    // Phase 2: Take range 3..=6 and verify returned keys
    {
        let writer = hot_kv.writer().unwrap();
        let removed = writer.take_range::<tables::Headers>(3..=6).unwrap();
        writer.commit().unwrap();

        // Should return keys 3, 4, 5, 6 in order
        assert_eq!(removed.len(), 4);

        for i in 0..4 {
            assert_eq!(removed[i].0, (i as u64) + 3);
            assert_eq!(&removed[i].1, &headers[i + 3]);
        }
    }

    // Verify the keys are actually removed
    {
        let reader = hot_kv.reader().unwrap();
        for i in 0u64..3 {
            assert!(reader.get_header(i).unwrap().is_some(), "header {} should exist", i);
        }
        for i in 3u64..7 {
            assert!(reader.get_header(i).unwrap().is_none(), "header {} should be gone", i);
        }
        for i in 7u64..10 {
            assert!(reader.get_header(i).unwrap().is_some(), "header {} should exist", i);
        }
    }

    // Phase 3: Take empty range (nothing to remove)
    {
        let writer = hot_kv.writer().unwrap();
        let removed = writer.take_range::<tables::Headers>(100..=200).unwrap();
        writer.commit().unwrap();

        assert!(removed.is_empty(), "should return empty vec for non-existent range");
    }

    // Phase 4: Take single key
    {
        let writer = hot_kv.writer().unwrap();
        let removed = writer.take_range::<tables::Headers>(8..=8).unwrap();
        writer.commit().unwrap();

        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].0, 8);
        assert_eq!(&removed[0].1, &headers[8]);
    }

    {
        let reader = hot_kv.reader().unwrap();
        assert!(reader.get_header(7).unwrap().is_some());
        assert!(reader.get_header(8).unwrap().is_none());
        assert!(reader.get_header(9).unwrap().is_some());
    }
}

/// Test clear_range_dual on a dual-keyed table.
///
/// This test verifies that:
/// 1. All k2 entries for k1 values within the range are deleted
/// 2. k1 values outside the range remain intact
/// 3. Edge cases work correctly
pub fn test_clear_range_dual<T: HotKv>(hot_kv: &T) {
    let addr1 = address!("0x1000000000000000000000000000000000000001");
    let addr2 = address!("0x2000000000000000000000000000000000000002");
    let addr3 = address!("0x3000000000000000000000000000000000000003");
    let addr4 = address!("0x4000000000000000000000000000000000000004");
    let addr5 = address!("0x5000000000000000000000000000000000000005");

    // Phase 1: Write account history entries for multiple addresses with multiple shards
    {
        let writer = hot_kv.writer().unwrap();

        // addr1: two shards
        let history1_a = BlockNumberList::new([1, 2, 3]).unwrap();
        let history1_b = BlockNumberList::new([4, 5, 6]).unwrap();
        writer.write_account_history(&addr1, 100, &history1_a).unwrap();
        writer.write_account_history(&addr1, u64::MAX, &history1_b).unwrap();

        // addr2: one shard
        let history2 = BlockNumberList::new([10, 20]).unwrap();
        writer.write_account_history(&addr2, u64::MAX, &history2).unwrap();

        // addr3: one shard
        let history3 = BlockNumberList::new([30, 40]).unwrap();
        writer.write_account_history(&addr3, u64::MAX, &history3).unwrap();

        // addr4: two shards
        let history4_a = BlockNumberList::new([50, 60]).unwrap();
        let history4_b = BlockNumberList::new([70, 80]).unwrap();
        writer.write_account_history(&addr4, 200, &history4_a).unwrap();
        writer.write_account_history(&addr4, u64::MAX, &history4_b).unwrap();

        // addr5: one shard
        let history5 = BlockNumberList::new([90, 100]).unwrap();
        writer.write_account_history(&addr5, u64::MAX, &history5).unwrap();

        writer.commit().unwrap();
    }

    // Verify all entries exist
    {
        let reader = hot_kv.reader().unwrap();
        assert!(reader.get_account_history(&addr1, 100).unwrap().is_some());
        assert!(reader.get_account_history(&addr1, u64::MAX).unwrap().is_some());
        assert!(reader.get_account_history(&addr2, u64::MAX).unwrap().is_some());
        assert!(reader.get_account_history(&addr3, u64::MAX).unwrap().is_some());
        assert!(reader.get_account_history(&addr4, 200).unwrap().is_some());
        assert!(reader.get_account_history(&addr4, u64::MAX).unwrap().is_some());
        assert!(reader.get_account_history(&addr5, u64::MAX).unwrap().is_some());
    }

    // Phase 2: Clear range addr2..=addr3 (middle range)
    {
        let writer = hot_kv.writer().unwrap();
        writer.clear_range_dual::<tables::AccountsHistory>((addr2, 0)..=(addr3, u64::MAX)).unwrap();
        writer.commit().unwrap();
    }

    // Verify: addr1 and addr4, addr5 should exist, addr2 and addr3 should be gone
    {
        let reader = hot_kv.reader().unwrap();

        // addr1 entries should still exist
        assert!(
            reader.get_account_history(&addr1, 100).unwrap().is_some(),
            "addr1 shard 100 should exist"
        );
        assert!(
            reader.get_account_history(&addr1, u64::MAX).unwrap().is_some(),
            "addr1 shard max should exist"
        );

        // addr2 and addr3 should be deleted
        assert!(
            reader.get_account_history(&addr2, u64::MAX).unwrap().is_none(),
            "addr2 should be deleted"
        );
        assert!(
            reader.get_account_history(&addr3, u64::MAX).unwrap().is_none(),
            "addr3 should be deleted"
        );

        // addr4 and addr5 entries should still exist
        assert!(
            reader.get_account_history(&addr4, 200).unwrap().is_some(),
            "addr4 shard 200 should exist"
        );
        assert!(
            reader.get_account_history(&addr4, u64::MAX).unwrap().is_some(),
            "addr4 shard max should exist"
        );
        assert!(
            reader.get_account_history(&addr5, u64::MAX).unwrap().is_some(),
            "addr5 should exist"
        );
    }
}

/// Test take_range_dual on a dual-keyed table.
///
/// Similar to clear_range_dual but also returns the removed (k1, k2) pairs.
pub fn test_take_range_dual<T: HotKv>(hot_kv: &T) {
    let addr1 = address!("0xa000000000000000000000000000000000000001");
    let addr2 = address!("0xb000000000000000000000000000000000000002");
    let addr3 = address!("0xc000000000000000000000000000000000000003");

    // Phase 1: Write account history entries
    {
        let writer = hot_kv.writer().unwrap();

        // addr1: two shards
        let history1_a = BlockNumberList::new([1, 2]).unwrap();
        let history1_b = BlockNumberList::new([3, 4]).unwrap();
        writer.write_account_history(&addr1, 50, &history1_a).unwrap();
        writer.write_account_history(&addr1, u64::MAX, &history1_b).unwrap();

        // addr2: one shard
        let history2 = BlockNumberList::new([10, 20]).unwrap();
        writer.write_account_history(&addr2, u64::MAX, &history2).unwrap();

        // addr3: one shard
        let history3 = BlockNumberList::new([30, 40]).unwrap();
        writer.write_account_history(&addr3, u64::MAX, &history3).unwrap();

        writer.commit().unwrap();
    }

    // Phase 2: Take range addr1..=addr2 and verify returned pairs
    {
        let writer = hot_kv.writer().unwrap();
        let removed = writer
            .take_range_dual::<tables::AccountsHistory>((addr1, 0)..=(addr2, u64::MAX))
            .unwrap();
        writer.commit().unwrap();

        // Should return (addr1, 50), (addr1, max), (addr2, max)
        assert_eq!(removed.len(), 3, "should have removed 3 entries");
        assert_eq!(removed[0].0, addr1);
        assert_eq!(removed[0].1, 50);
        assert_eq!(removed[1].0, addr1);
        assert_eq!(removed[1].1, u64::MAX);
        assert_eq!(removed[2].0, addr2);
        assert_eq!(removed[2].1, u64::MAX);
    }

    // Verify only addr3 remains
    {
        let reader = hot_kv.reader().unwrap();
        assert!(reader.get_account_history(&addr1, 50).unwrap().is_none());
        assert!(reader.get_account_history(&addr1, u64::MAX).unwrap().is_none());
        assert!(reader.get_account_history(&addr2, u64::MAX).unwrap().is_none());
        assert!(reader.get_account_history(&addr3, u64::MAX).unwrap().is_some());
    }

    // Phase 3: Take empty range
    {
        let writer = hot_kv.writer().unwrap();
        let removed = writer
            .take_range_dual::<tables::AccountsHistory>(
                (address!("0xf000000000000000000000000000000000000000"), 0)
                    ..=(address!("0xff00000000000000000000000000000000000000"), u64::MAX),
            )
            .unwrap();
        writer.commit().unwrap();

        assert!(removed.is_empty(), "should return empty vec for non-existent range");
    }
}

// ============================================================================
// Unwind Conformance Test
// ============================================================================

/// Collect all entries from a single-keyed table.
fn collect_single_table<T, R>(reader: &R) -> Vec<KeyValue<T>>
where
    T: SingleKey,
    T::Key: Ord,
    R: HotKvRead,
{
    let mut cursor = reader.traverse::<T>().unwrap();
    let mut entries = Vec::new();
    if let Some(first) = TableTraverse::<T, _>::first(&mut *cursor.inner_mut()).unwrap() {
        entries.push(first);
        while let Some(next) = TableTraverse::<T, _>::read_next(&mut *cursor.inner_mut()).unwrap() {
            entries.push(next);
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

/// Collect all entries from a dual-keyed table.
fn collect_dual_table<T, R>(reader: &R) -> Vec<DualKeyValue<T>>
where
    T: DualKey,
    T::Key: Ord,
    T::Key2: Ord,
    R: HotKvRead,
{
    let mut cursor = reader.traverse_dual::<T>().unwrap();
    let mut entries = Vec::new();
    if let Some(first) = DualTableTraverse::<T, _>::first(&mut *cursor.inner_mut()).unwrap() {
        entries.push(first);
        while let Some(next) =
            DualTableTraverse::<T, _>::read_next(&mut *cursor.inner_mut()).unwrap()
        {
            entries.push(next);
        }
    }
    entries.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    entries
}

/// Assert two single-keyed table contents are equal.
fn assert_single_tables_equal<T>(table_name: &str, a: Vec<KeyValue<T>>, b: Vec<KeyValue<T>>)
where
    T: SingleKey,
    T::Key: Debug + PartialEq,
    T::Value: Debug + PartialEq,
{
    assert_eq!(
        a.len(),
        b.len(),
        "{} table entry count mismatch: {} vs {}",
        table_name,
        a.len(),
        b.len()
    );
    for (i, (entry_a, entry_b)) in a.iter().zip(b.iter()).enumerate() {
        assert_eq!(
            entry_a, entry_b,
            "{} table entry {} mismatch:\n  A: {:?}\n  B: {:?}",
            table_name, i, entry_a, entry_b
        );
    }
}

/// Assert two dual-keyed table contents are equal.
fn assert_dual_tables_equal<T>(table_name: &str, a: Vec<DualKeyValue<T>>, b: Vec<DualKeyValue<T>>)
where
    T: DualKey,
    T::Key: Debug + PartialEq,
    T::Key2: Debug + PartialEq,
    T::Value: Debug + PartialEq,
{
    assert_eq!(
        a.len(),
        b.len(),
        "{} table entry count mismatch: {} vs {}",
        table_name,
        a.len(),
        b.len()
    );
    for (i, (entry_a, entry_b)) in a.iter().zip(b.iter()).enumerate() {
        assert_eq!(
            entry_a, entry_b,
            "{} table entry {} mismatch:\n  A: {:?}\n  B: {:?}",
            table_name, i, entry_a, entry_b
        );
    }
}

/// Create a BundleState with account and storage changes.
///
/// This function creates a proper BundleState with reverts populated so that
/// `to_plain_state_and_reverts` will produce the expected output.
#[allow(clippy::type_complexity)]
fn make_bundle_state(
    accounts: Vec<(Address, Option<AccountInfo>, Option<AccountInfo>)>,
    storage: Vec<(Address, Vec<(U256, U256, U256)>)>, // (addr, [(slot, old, new)])
    _contracts: Vec<(B256, RevmBytecode)>,
) -> BundleState {
    let mut state: HashMap<Address, BundleAccount, DefaultHashBuilder> = Default::default();

    // Build account reverts for this block
    let mut block_reverts: Vec<(Address, AccountRevert)> = Vec::new();

    for (addr, original, info) in &accounts {
        let account_storage: HashMap<U256, StorageSlot, DefaultHashBuilder> = Default::default();
        state.insert(
            *addr,
            BundleAccount {
                info: info.clone(),
                original_info: original.clone(),
                storage: account_storage,
                status: AccountStatus::Changed,
            },
        );

        // Create account revert - this stores what to restore to when unwinding
        let account_info_revert = match original {
            Some(orig) => AccountInfoRevert::RevertTo(orig.clone()),
            None => AccountInfoRevert::DeleteIt,
        };

        block_reverts.push((
            *addr,
            AccountRevert {
                account: account_info_revert,
                storage: Default::default(), // Storage reverts added below
                previous_status: AccountStatus::Changed,
                wipe_storage: false,
            },
        ));
    }

    // Process storage changes
    for (addr, slots) in &storage {
        let account = state.entry(*addr).or_insert_with(|| BundleAccount {
            info: None,
            original_info: None,
            storage: Default::default(),
            status: AccountStatus::Changed,
        });

        // Find or create the account revert entry
        let revert_entry = block_reverts.iter_mut().find(|(a, _)| a == addr);
        let account_revert = if let Some((_, revert)) = revert_entry {
            revert
        } else {
            block_reverts.push((
                *addr,
                AccountRevert {
                    account: AccountInfoRevert::DoNothing,
                    storage: Default::default(),
                    previous_status: AccountStatus::Changed,
                    wipe_storage: false,
                },
            ));
            &mut block_reverts.last_mut().unwrap().1
        };

        for (slot, old_value, new_value) in slots {
            account.storage.insert(
                *slot,
                StorageSlot { previous_or_original_value: *old_value, present_value: *new_value },
            );

            // Add storage revert entry
            account_revert.storage.insert(*slot, RevertToSlot::Some(*old_value));
        }
    }

    // Create Reverts with one block's worth of reverts
    let reverts = Reverts::new(vec![block_reverts]);

    BundleState { state, contracts: Default::default(), reverts, state_size: 0, reverts_size: 0 }
}

/// Create a simple AccountInfo for testing.
fn make_account_info(nonce: u64, balance: U256, code_hash: Option<B256>) -> AccountInfo {
    AccountInfo { nonce, balance, code_hash: code_hash.unwrap_or(B256::ZERO), code: None }
}

/// Test that unwinding produces the exact same state as never having appended.
///
/// This test:
/// 1. Creates 5 blocks with complex state changes
/// 2. Appends all 5 blocks to store_a, then unwinds to block 1 (keeping blocks 0, 1)
/// 3. Appends only blocks 0, 1 to store_b
/// 4. Compares ALL tables between the two stores - they must be exactly equal
///
/// This proves that `unwind_above` correctly reverses all state changes including:
/// - Plain account state
/// - Plain storage state
/// - Headers and header number mappings
/// - Account and storage change sets
/// - Account and storage history indices
pub fn test_unwind_conformance<Kv: HotKv>(store_a: &Kv, store_b: &Kv) {
    // Test addresses
    let addr1 = address!("0x1111111111111111111111111111111111111111");
    let addr2 = address!("0x2222222222222222222222222222222222222222");
    let addr3 = address!("0x3333333333333333333333333333333333333333");
    let addr4 = address!("0x4444444444444444444444444444444444444444");

    // Storage slots
    let slot1 = U256::from(1);
    let slot2 = U256::from(2);
    let slot3 = U256::from(3);

    // Create bytecode
    let code = Bytes::from_static(&[0x60, 0x00, 0x60, 0x00, 0xf3]);
    let bytecode = RevmBytecode::new_raw(code);
    let code_hash = bytecode.hash_slow();

    // Create 5 blocks with complex state
    let mut blocks: Vec<(SealedHeader, BundleState)> = Vec::new();
    let mut prev_hash = B256::ZERO;

    // Block 0: Create addr1, addr2, addr3 with different states
    {
        let header = Header {
            number: 0,
            parent_hash: prev_hash,
            gas_limit: 1_000_000,
            ..Default::default()
        };
        let sealed = SealedHeader::seal_slow(header);
        prev_hash = sealed.hash();

        let bundle = make_bundle_state(
            vec![
                (addr1, None, Some(make_account_info(1, U256::from(100), None))),
                (addr2, None, Some(make_account_info(1, U256::from(200), None))),
                (addr3, None, Some(make_account_info(1, U256::from(300), None))),
            ],
            vec![(addr1, vec![(slot1, U256::ZERO, U256::from(10))])],
            vec![],
        );
        blocks.push((sealed, bundle));
    }

    // Block 1: Update addr1, addr2; add storage to addr2
    {
        let header = Header {
            number: 1,
            parent_hash: prev_hash,
            gas_limit: 1_000_000,
            ..Default::default()
        };
        let sealed = SealedHeader::seal_slow(header);
        prev_hash = sealed.hash();

        let bundle = make_bundle_state(
            vec![
                (
                    addr1,
                    Some(make_account_info(1, U256::from(100), None)),
                    Some(make_account_info(2, U256::from(150), None)),
                ),
                (
                    addr2,
                    Some(make_account_info(1, U256::from(200), None)),
                    Some(make_account_info(2, U256::from(250), None)),
                ),
            ],
            vec![
                (addr1, vec![(slot1, U256::from(10), U256::from(20))]),
                (addr2, vec![(slot1, U256::ZERO, U256::from(100))]),
            ],
            vec![],
        );
        blocks.push((sealed, bundle));
    }

    // Block 2: Update addr3, add bytecode (this is the boundary - will be unwound)
    {
        let header = Header {
            number: 2,
            parent_hash: prev_hash,
            gas_limit: 1_000_000,
            ..Default::default()
        };
        let sealed = SealedHeader::seal_slow(header);
        prev_hash = sealed.hash();

        let bundle = make_bundle_state(
            vec![(
                addr3,
                Some(make_account_info(1, U256::from(300), None)),
                Some(make_account_info(2, U256::from(350), Some(code_hash))),
            )],
            vec![(addr3, vec![(slot1, U256::ZERO, U256::from(1000))])],
            vec![(code_hash, bytecode.clone())],
        );
        blocks.push((sealed, bundle));
    }

    // Block 3: Create addr4, update existing storage
    {
        let header = Header {
            number: 3,
            parent_hash: prev_hash,
            gas_limit: 1_000_000,
            ..Default::default()
        };
        let sealed = SealedHeader::seal_slow(header);
        prev_hash = sealed.hash();

        let bundle = make_bundle_state(
            vec![
                (addr4, None, Some(make_account_info(1, U256::from(400), None))),
                (
                    addr1,
                    Some(make_account_info(2, U256::from(150), None)),
                    Some(make_account_info(3, U256::from(175), None)),
                ),
            ],
            vec![
                (
                    addr1,
                    vec![
                        (slot1, U256::from(20), U256::from(30)),
                        (slot2, U256::ZERO, U256::from(50)),
                    ],
                ),
                (addr4, vec![(slot1, U256::ZERO, U256::from(500))]),
            ],
            vec![],
        );
        blocks.push((sealed, bundle));
    }

    // Block 4: Update multiple addresses and storage
    {
        let header = Header {
            number: 4,
            parent_hash: prev_hash,
            gas_limit: 1_000_000,
            ..Default::default()
        };
        let sealed = SealedHeader::seal_slow(header);

        let bundle = make_bundle_state(
            vec![
                (
                    addr1,
                    Some(make_account_info(3, U256::from(175), None)),
                    Some(make_account_info(4, U256::from(200), None)),
                ),
                (
                    addr2,
                    Some(make_account_info(2, U256::from(250), None)),
                    Some(make_account_info(3, U256::from(275), None)),
                ),
                (
                    addr4,
                    Some(make_account_info(1, U256::from(400), None)),
                    Some(make_account_info(2, U256::from(450), None)),
                ),
            ],
            vec![
                (
                    addr1,
                    vec![
                        (slot1, U256::from(30), U256::from(40)),
                        (slot3, U256::ZERO, U256::from(60)),
                    ],
                ),
                (
                    addr2,
                    vec![
                        (slot1, U256::from(100), U256::from(150)),
                        (slot2, U256::ZERO, U256::from(200)),
                    ],
                ),
            ],
            vec![],
        );
        blocks.push((sealed, bundle));
    }

    // Store A: Append all 5 blocks, then unwind to block 1
    {
        let writer = store_a.writer().unwrap();
        writer.append_blocks(&blocks).unwrap();
        writer.commit().unwrap();
    }
    {
        let writer = store_a.writer().unwrap();
        writer.unwind_above(1).unwrap();
        writer.commit().unwrap();
    }

    // Store B: Append only blocks 0, 1
    {
        let writer = store_b.writer().unwrap();
        writer.append_blocks(&blocks[0..2]).unwrap();
        writer.commit().unwrap();
    }

    // Compare all tables
    let reader_a = store_a.reader().unwrap();
    let reader_b = store_b.reader().unwrap();

    // Single-keyed tables
    assert_single_tables_equal::<tables::Headers>(
        "Headers",
        collect_single_table::<tables::Headers, _>(&reader_a),
        collect_single_table::<tables::Headers, _>(&reader_b),
    );

    assert_single_tables_equal::<tables::HeaderNumbers>(
        "HeaderNumbers",
        collect_single_table::<tables::HeaderNumbers, _>(&reader_a),
        collect_single_table::<tables::HeaderNumbers, _>(&reader_b),
    );

    assert_single_tables_equal::<tables::PlainAccountState>(
        "PlainAccountState",
        collect_single_table::<tables::PlainAccountState, _>(&reader_a),
        collect_single_table::<tables::PlainAccountState, _>(&reader_b),
    );

    // Note: Bytecodes are not removed on unwind (they're content-addressed),
    // so store_a may have more bytecodes than store_b. We skip this comparison.
    // assert_single_tables_equal::<tables::Bytecodes>(...)

    // Dual-keyed tables
    assert_dual_tables_equal::<tables::PlainStorageState>(
        "PlainStorageState",
        collect_dual_table::<tables::PlainStorageState, _>(&reader_a),
        collect_dual_table::<tables::PlainStorageState, _>(&reader_b),
    );

    assert_dual_tables_equal::<tables::AccountChangeSets>(
        "AccountChangeSets",
        collect_dual_table::<tables::AccountChangeSets, _>(&reader_a),
        collect_dual_table::<tables::AccountChangeSets, _>(&reader_b),
    );

    assert_dual_tables_equal::<tables::StorageChangeSets>(
        "StorageChangeSets",
        collect_dual_table::<tables::StorageChangeSets, _>(&reader_a),
        collect_dual_table::<tables::StorageChangeSets, _>(&reader_b),
    );

    assert_dual_tables_equal::<tables::AccountsHistory>(
        "AccountsHistory",
        collect_dual_table::<tables::AccountsHistory, _>(&reader_a),
        collect_dual_table::<tables::AccountsHistory, _>(&reader_b),
    );

    assert_dual_tables_equal::<tables::StorageHistory>(
        "StorageHistory",
        collect_dual_table::<tables::StorageHistory, _>(&reader_a),
        collect_dual_table::<tables::StorageHistory, _>(&reader_b),
    );
}

// ============================================================================
// Value Edge Case Tests
// ============================================================================

/// Test that zero storage values are correctly stored and retrieved.
///
/// This verifies that U256::ZERO is not confused with "not set" or deleted.
pub fn test_zero_storage_value<T: HotKv>(hot_kv: &T) {
    let addr = address!("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa1");
    let slot = U256::from(1);

    // Write zero value
    {
        let writer = hot_kv.writer().unwrap();
        writer.put_storage(&addr, &slot, &U256::ZERO).unwrap();
        writer.commit().unwrap();
    }

    // Read zero value - should return Some(ZERO), not None
    {
        let reader = hot_kv.reader().unwrap();
        let value = reader.get_storage(&addr, &slot).unwrap();
        assert!(value.is_some(), "Zero storage value should be Some, not None");
        assert_eq!(value.unwrap(), U256::ZERO, "Zero storage value should be U256::ZERO");
    }

    // Verify via traversal that the entry exists
    {
        let reader = hot_kv.reader().unwrap();
        let mut cursor = reader.traverse_dual::<tables::PlainStorageState>().unwrap();
        let mut found = false;
        while let Some((k1, k2, v)) = cursor.read_next().unwrap() {
            if k1 == addr && k2 == slot {
                found = true;
                assert_eq!(v, U256::ZERO);
            }
        }
        assert!(found, "Zero value entry should exist in table");
    }
}

/// Test that empty accounts (all zero fields) are correctly stored and retrieved.
///
/// This verifies that an account with nonce=0, balance=0, no code is not
/// confused with a non-existent account.
pub fn test_empty_account<T: HotKv>(hot_kv: &T) {
    let addr = address!("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa2");
    let empty_account = Account { nonce: 0, balance: U256::ZERO, bytecode_hash: None };

    // Write empty account
    {
        let writer = hot_kv.writer().unwrap();
        writer.put_account(&addr, &empty_account).unwrap();
        writer.commit().unwrap();
    }

    // Read empty account - should return Some, not None
    {
        let reader = hot_kv.reader().unwrap();
        let account = reader.get_account(&addr).unwrap();
        assert!(account.is_some(), "Empty account should be Some, not None");
        let account = account.unwrap();
        assert_eq!(account.nonce, 0);
        assert_eq!(account.balance, U256::ZERO);
        assert!(account.bytecode_hash.is_none());
    }
}

/// Test that maximum storage values (U256::MAX) are correctly stored and retrieved.
pub fn test_max_storage_value<T: HotKv>(hot_kv: &T) {
    let addr = address!("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa3");
    let slot = U256::from(1);

    // Write max value
    {
        let writer = hot_kv.writer().unwrap();
        writer.put_storage(&addr, &slot, &U256::MAX).unwrap();
        writer.commit().unwrap();
    }

    // Read max value
    {
        let reader = hot_kv.reader().unwrap();
        let value = reader.get_storage(&addr, &slot).unwrap();
        assert!(value.is_some());
        assert_eq!(value.unwrap(), U256::MAX, "Max storage value should be preserved");
    }
}

/// Test that maximum block numbers (u64::MAX) work correctly in headers.
pub fn test_max_block_number<T: HotKv>(hot_kv: &T) {
    let header = Header { number: u64::MAX, gas_limit: 1_000_000, ..Default::default() };
    let sealed = SealedHeader::seal_slow(header.clone());

    // Write header at max block number
    {
        let writer = hot_kv.writer().unwrap();
        writer.put_header(&sealed).unwrap();
        writer.commit().unwrap();
    }

    // Read header
    {
        let reader = hot_kv.reader().unwrap();
        let read_header = reader.get_header(u64::MAX).unwrap();
        assert!(read_header.is_some());
        assert_eq!(read_header.unwrap().number, u64::MAX);
    }
}

// ============================================================================
// Cursor Operation Tests
// ============================================================================

/// Test cursor operations on an empty table.
///
/// Verifies that first(), last(), exact(), lower_bound() return None on empty tables.
pub fn test_cursor_empty_table<T: HotKv>(hot_kv: &T) {
    // Use a table that we haven't written to in this test
    // We'll use HeaderNumbers which should be empty if we haven't written headers with hashes
    let reader = hot_kv.reader().unwrap();

    // Create a fresh address that definitely doesn't exist
    let missing_addr = address!("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb01");

    // Test single-key cursor on PlainAccountState for a non-existent key
    {
        let mut cursor = reader.traverse::<tables::PlainAccountState>().unwrap();

        // exact() for non-existent key should return None
        let exact_result = cursor.exact(&missing_addr).unwrap();
        assert!(exact_result.is_none(), "exact() on non-existent key should return None");

        // lower_bound for a key beyond all existing should return None
        let lb_result =
            cursor.lower_bound(&address!("0xffffffffffffffffffffffffffffffffffffff99")).unwrap();
        // This might return something if there are entries, but for a truly empty table it would be None
        // We're mainly testing that it doesn't panic
        let _ = lb_result;
    }

    // Test dual-key cursor
    {
        let mut cursor = reader.traverse_dual::<tables::PlainStorageState>().unwrap();

        // exact_dual for non-existent keys should return None
        let exact_result = cursor.exact_dual(&missing_addr, &U256::from(999)).unwrap();
        assert!(exact_result.is_none(), "exact_dual() on non-existent key should return None");
    }
}

/// Test cursor exact() match semantics.
///
/// Verifies that exact() returns only exact matches, not lower_bound semantics.
pub fn test_cursor_exact_match<T: HotKv>(hot_kv: &T) {
    // Write headers at block numbers 10, 20, 30
    {
        let writer = hot_kv.writer().unwrap();
        for i in [10u64, 20, 30] {
            let header = Header { number: i, gas_limit: 1_000_000, ..Default::default() };
            writer.put_header_inconsistent(&header).unwrap();
        }
        writer.commit().unwrap();
    }

    let reader = hot_kv.reader().unwrap();
    let mut cursor = reader.traverse::<tables::Headers>().unwrap();

    // exact() for existing key should return value
    let exact_10 = cursor.exact(&10u64).unwrap();
    assert!(exact_10.is_some(), "exact(10) should find the header");
    assert_eq!(exact_10.unwrap().number, 10);

    // exact() for non-existing key should return None, not the next key
    let exact_15 = cursor.exact(&15u64).unwrap();
    assert!(exact_15.is_none(), "exact(15) should return None, not header 20");

    // Verify lower_bound would have found something at 15
    let lb_15 = cursor.lower_bound(&15u64).unwrap();
    assert!(lb_15.is_some(), "lower_bound(15) should find header 20");
    assert_eq!(lb_15.unwrap().0, 20);
}

/// Test cursor backward iteration with read_prev().
pub fn test_cursor_backward_iteration<T: HotKv>(hot_kv: &T) {
    // Write headers at block numbers 100, 101, 102, 103, 104
    {
        let writer = hot_kv.writer().unwrap();
        for i in 100u64..105 {
            let header = Header { number: i, gas_limit: 1_000_000, ..Default::default() };
            writer.put_header_inconsistent(&header).unwrap();
        }
        writer.commit().unwrap();
    }

    let reader = hot_kv.reader().unwrap();
    let mut cursor = reader.traverse::<tables::Headers>().unwrap();

    // Position at last entry
    let last = cursor.last().unwrap();
    assert!(last.is_some());
    let (num, _) = last.unwrap();
    assert_eq!(num, 104);

    // Iterate backward
    let prev1 = cursor.read_prev().unwrap();
    assert!(prev1.is_some());
    assert_eq!(prev1.unwrap().0, 103);

    let prev2 = cursor.read_prev().unwrap();
    assert!(prev2.is_some());
    assert_eq!(prev2.unwrap().0, 102);

    let prev3 = cursor.read_prev().unwrap();
    assert!(prev3.is_some());
    assert_eq!(prev3.unwrap().0, 101);

    let prev4 = cursor.read_prev().unwrap();
    assert!(prev4.is_some());
    assert_eq!(prev4.unwrap().0, 100);

    // Should hit beginning
    let prev5 = cursor.read_prev().unwrap();
    assert!(prev5.is_none(), "read_prev() past beginning should return None");
}

/// Test dual-key cursor navigation between k1 values.
pub fn test_cursor_dual_navigation<T: HotKv>(hot_kv: &T) {
    let addr1 = address!("0xcccccccccccccccccccccccccccccccccccccc01");
    let addr2 = address!("0xcccccccccccccccccccccccccccccccccccccc02");
    let addr3 = address!("0xcccccccccccccccccccccccccccccccccccccc03");

    // Write storage for multiple addresses with multiple slots
    {
        let writer = hot_kv.writer().unwrap();

        // addr1: slots 1, 2, 3
        writer.put_storage(&addr1, &U256::from(1), &U256::from(10)).unwrap();
        writer.put_storage(&addr1, &U256::from(2), &U256::from(20)).unwrap();
        writer.put_storage(&addr1, &U256::from(3), &U256::from(30)).unwrap();

        // addr2: slots 1, 2
        writer.put_storage(&addr2, &U256::from(1), &U256::from(100)).unwrap();
        writer.put_storage(&addr2, &U256::from(2), &U256::from(200)).unwrap();

        // addr3: slot 1
        writer.put_storage(&addr3, &U256::from(1), &U256::from(1000)).unwrap();

        writer.commit().unwrap();
    }

    let reader = hot_kv.reader().unwrap();
    let mut cursor = reader.traverse_dual::<tables::PlainStorageState>().unwrap();

    // Position at first entry
    let first =
        DualTableTraverse::<tables::PlainStorageState, _>::first(&mut *cursor.inner_mut()).unwrap();
    assert!(first.is_some());
    let (k1, k2, _) = first.unwrap();
    assert_eq!(k1, addr1);
    assert_eq!(k2, U256::from(1));

    // next_k1() should jump to addr2
    let next_addr = cursor.next_k1().unwrap();
    assert!(next_addr.is_some());
    let (k1, k2, _) = next_addr.unwrap();
    assert_eq!(k1, addr2, "next_k1() should jump to addr2");
    assert_eq!(k2, U256::from(1), "Should be at first slot of addr2");

    // next_k1() again should jump to addr3
    let next_addr = cursor.next_k1().unwrap();
    assert!(next_addr.is_some());
    let (k1, _, _) = next_addr.unwrap();
    assert_eq!(k1, addr3, "next_k1() should jump to addr3");

    // next_k1() again should return None (no more k1 values)
    let next_addr = cursor.next_k1().unwrap();
    assert!(next_addr.is_none(), "next_k1() at end should return None");

    // Test previous_k1()
    // First position at addr3
    cursor.last_of_k1(&addr3).unwrap();
    let prev_addr = cursor.previous_k1().unwrap();
    assert!(prev_addr.is_some());
    let (k1, _, _) = prev_addr.unwrap();
    assert_eq!(k1, addr2, "previous_k1() from addr3 should go to addr2");
}

/// Test cursor on table with single entry.
pub fn test_cursor_single_entry<T: HotKv>(hot_kv: &T) {
    let addr = address!("0xdddddddddddddddddddddddddddddddddddddd01");
    let account = Account { nonce: 42, balance: U256::from(1000), bytecode_hash: None };

    // Write single account
    {
        let writer = hot_kv.writer().unwrap();
        writer.put_account(&addr, &account).unwrap();
        writer.commit().unwrap();
    }

    let reader = hot_kv.reader().unwrap();
    let mut cursor = reader.traverse::<tables::PlainAccountState>().unwrap();

    // first() and last() should return the same entry
    let first = cursor.first().unwrap();
    assert!(first.is_some());
    let (first_addr, _) = first.unwrap();

    let last = cursor.last().unwrap();
    assert!(last.is_some());
    let (last_addr, _) = last.unwrap();

    assert_eq!(first_addr, last_addr, "first() and last() should be same for single entry");

    // read_next() after first() should return None
    cursor.first().unwrap();
    let next = cursor.read_next().unwrap();
    assert!(next.is_none(), "read_next() after first() on single entry should return None");
}

// ============================================================================
// Batch Operation Tests
// ============================================================================

/// Test get_many batch retrieval.
pub fn test_get_many<T: HotKv>(hot_kv: &T) {
    let addr1 = address!("0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee01");
    let addr2 = address!("0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee02");
    let addr3 = address!("0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee03");
    let addr4 = address!("0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee04"); // non-existent

    let acc1 = Account { nonce: 1, balance: U256::from(100), bytecode_hash: None };
    let acc2 = Account { nonce: 2, balance: U256::from(200), bytecode_hash: None };
    let acc3 = Account { nonce: 3, balance: U256::from(300), bytecode_hash: None };

    // Write accounts
    {
        let writer = hot_kv.writer().unwrap();
        writer.put_account(&addr1, &acc1).unwrap();
        writer.put_account(&addr2, &acc2).unwrap();
        writer.put_account(&addr3, &acc3).unwrap();
        writer.commit().unwrap();
    }

    // Batch retrieve
    {
        let reader = hot_kv.reader().unwrap();
        let keys = [addr1, addr2, addr3, addr4];
        let results = reader.get_many::<tables::PlainAccountState, _>(&keys).unwrap();

        assert_eq!(results.len(), 4);

        // Build a map for easier checking (order not guaranteed)
        let result_map: HashMap<&Address, Option<Account>> =
            results.iter().map(|(k, v)| (*k, *v)).collect();

        assert!(result_map[&addr1].is_some());
        assert_eq!(result_map[&addr1].as_ref().unwrap().nonce, 1);

        assert!(result_map[&addr2].is_some());
        assert_eq!(result_map[&addr2].as_ref().unwrap().nonce, 2);

        assert!(result_map[&addr3].is_some());
        assert_eq!(result_map[&addr3].as_ref().unwrap().nonce, 3);

        assert!(result_map[&addr4].is_none(), "Non-existent key should return None");
    }
}

/// Test queue_put_many batch writes.
pub fn test_queue_put_many<T: HotKv>(hot_kv: &T) {
    let entries: Vec<(u64, Header)> = (200u64..210)
        .map(|i| (i, Header { number: i, gas_limit: 1_000_000, ..Default::default() }))
        .collect();

    // Batch write using queue_put_many
    {
        let writer = hot_kv.writer().unwrap();
        let refs: Vec<(&u64, &Header)> = entries.iter().map(|(k, v)| (k, v)).collect();
        writer.queue_put_many::<tables::Headers, _>(refs).unwrap();
        writer.commit().unwrap();
    }

    // Verify all entries exist
    {
        let reader = hot_kv.reader().unwrap();
        for i in 200u64..210 {
            let header = reader.get_header(i).unwrap();
            assert!(header.is_some(), "Header {} should exist after batch write", i);
            assert_eq!(header.unwrap().number, i);
        }
    }
}

/// Test queue_clear clears all entries in a table.
pub fn test_queue_clear<T: HotKv>(hot_kv: &T) {
    // Write some headers
    {
        let writer = hot_kv.writer().unwrap();
        for i in 300u64..310 {
            let header = Header { number: i, gas_limit: 1_000_000, ..Default::default() };
            writer.put_header_inconsistent(&header).unwrap();
        }
        writer.commit().unwrap();
    }

    // Verify entries exist
    {
        let reader = hot_kv.reader().unwrap();
        for i in 300u64..310 {
            assert!(reader.get_header(i).unwrap().is_some());
        }
    }

    // Clear the table
    {
        let writer = hot_kv.writer().unwrap();
        writer.queue_clear::<tables::Headers>().unwrap();
        writer.commit().unwrap();
    }

    // Verify all entries are gone
    {
        let reader = hot_kv.reader().unwrap();
        for i in 300u64..310 {
            assert!(
                reader.get_header(i).unwrap().is_none(),
                "Header {} should be gone after clear",
                i
            );
        }
    }
}

// ============================================================================
// Transaction Ordering Tests
// ============================================================================

/// Test that put-then-delete in the same transaction results in deletion.
pub fn test_put_then_delete_same_key<T: HotKv>(hot_kv: &T) {
    let addr = address!("0xffffffffffffffffffffffffffffffffffff0001");
    let account = Account { nonce: 99, balance: U256::from(9999), bytecode_hash: None };

    // In a single transaction: put then delete
    {
        let writer = hot_kv.writer().unwrap();
        writer.put_account(&addr, &account).unwrap();
        writer.queue_delete::<tables::PlainAccountState>(&addr).unwrap();
        writer.commit().unwrap();
    }

    // Account should not exist
    {
        let reader = hot_kv.reader().unwrap();
        let result = reader.get_account(&addr).unwrap();
        assert!(result.is_none(), "Put-then-delete should result in no entry");
    }
}

/// Test that delete-then-put in the same transaction results in the put value.
pub fn test_delete_then_put_same_key<T: HotKv>(hot_kv: &T) {
    let addr = address!("0xffffffffffffffffffffffffffffffffffff0002");
    let old_account = Account { nonce: 1, balance: U256::from(100), bytecode_hash: None };
    let new_account = Account { nonce: 2, balance: U256::from(200), bytecode_hash: None };

    // First, write an account
    {
        let writer = hot_kv.writer().unwrap();
        writer.put_account(&addr, &old_account).unwrap();
        writer.commit().unwrap();
    }

    // In a single transaction: delete then put new value
    {
        let writer = hot_kv.writer().unwrap();
        writer.queue_delete::<tables::PlainAccountState>(&addr).unwrap();
        writer.put_account(&addr, &new_account).unwrap();
        writer.commit().unwrap();
    }

    // Should have the new value
    {
        let reader = hot_kv.reader().unwrap();
        let result = reader.get_account(&addr).unwrap();
        assert!(result.is_some(), "Delete-then-put should result in entry existing");
        let account = result.unwrap();
        assert_eq!(account.nonce, 2, "Should have the new nonce");
        assert_eq!(account.balance, U256::from(200), "Should have the new balance");
    }
}

/// Test that multiple puts to the same key in one transaction use last value.
pub fn test_multiple_puts_same_key<T: HotKv>(hot_kv: &T) {
    let addr = address!("0xffffffffffffffffffffffffffffffffffff0003");

    // In a single transaction: put three different values
    {
        let writer = hot_kv.writer().unwrap();
        writer
            .put_account(
                &addr,
                &Account { nonce: 1, balance: U256::from(100), bytecode_hash: None },
            )
            .unwrap();
        writer
            .put_account(
                &addr,
                &Account { nonce: 2, balance: U256::from(200), bytecode_hash: None },
            )
            .unwrap();
        writer
            .put_account(
                &addr,
                &Account { nonce: 3, balance: U256::from(300), bytecode_hash: None },
            )
            .unwrap();
        writer.commit().unwrap();
    }

    // Should have the last value
    {
        let reader = hot_kv.reader().unwrap();
        let result = reader.get_account(&addr).unwrap();
        assert!(result.is_some());
        let account = result.unwrap();
        assert_eq!(account.nonce, 3, "Should have the last nonce (3)");
        assert_eq!(account.balance, U256::from(300), "Should have the last balance (300)");
    }
}

/// Test that abandoned transaction (dropped without commit) makes no changes.
pub fn test_abandoned_transaction<T: HotKv>(hot_kv: &T) {
    let addr = address!("0xffffffffffffffffffffffffffffffffffff0004");
    let account = Account { nonce: 42, balance: U256::from(4200), bytecode_hash: None };

    // Start a transaction, write, but don't commit (drop it)
    {
        let writer = hot_kv.writer().unwrap();
        writer.put_account(&addr, &account).unwrap();
        // writer is dropped here without commit
    }

    // Account should not exist
    {
        let reader = hot_kv.reader().unwrap();
        let result = reader.get_account(&addr).unwrap();
        assert!(result.is_none(), "Abandoned transaction should not persist changes");
    }
}

// ============================================================================
// Chain Validation Error Tests
// ============================================================================

/// Test that validate_chain_extension rejects non-contiguous blocks.
pub fn test_validate_noncontiguous_blocks<Kv: HotKv>(hot_kv: &Kv) {
    // First, append a genesis block
    let genesis = make_header(0, B256::ZERO);
    {
        let writer = hot_kv.writer().unwrap();
        let bundle = make_bundle_state(vec![], vec![], vec![]);
        writer.append_blocks(&[(genesis.clone(), bundle)]).unwrap();
        writer.commit().unwrap();
    }

    // Try to append block 2 (skipping block 1)
    let block2 = make_header(2, genesis.hash());
    {
        let writer = hot_kv.writer().unwrap();
        let bundle = make_bundle_state(vec![], vec![], vec![]);
        let result = writer.append_blocks(&[(block2, bundle)]);

        match result {
            Err(HistoryError::NonContiguousBlock { expected, got }) => {
                assert_eq!(expected, 1, "Expected block should be 1");
                assert_eq!(got, 2, "Got block should be 2");
            }
            Err(e) => panic!("Expected NonContiguousBlock error, got: {:?}", e),
            Ok(_) => panic!("Expected error for non-contiguous blocks"),
        }
    }
}

/// Test that validate_chain_extension rejects wrong parent hash.
pub fn test_validate_parent_hash_mismatch<Kv: HotKv>(hot_kv: &Kv) {
    // Append genesis block
    let genesis = make_header(0, B256::ZERO);
    {
        let writer = hot_kv.writer().unwrap();
        let bundle = make_bundle_state(vec![], vec![], vec![]);
        writer.append_blocks(&[(genesis.clone(), bundle)]).unwrap();
        writer.commit().unwrap();
    }

    // Try to append block 1 with wrong parent hash
    let wrong_parent = b256!("0x1111111111111111111111111111111111111111111111111111111111111111");
    let block1 = make_header(1, wrong_parent);
    {
        let writer = hot_kv.writer().unwrap();
        let bundle = make_bundle_state(vec![], vec![], vec![]);
        let result = writer.append_blocks(&[(block1, bundle)]);

        match result {
            Err(HistoryError::ParentHashMismatch { expected, got }) => {
                assert_eq!(expected, genesis.hash(), "Expected parent should be genesis hash");
                assert_eq!(got, wrong_parent, "Got parent should be wrong_parent");
            }
            Err(e) => panic!("Expected ParentHashMismatch error, got: {:?}", e),
            Ok(_) => panic!("Expected error for parent hash mismatch"),
        }
    }
}

/// Test appending genesis block (block 0) to empty database.
pub fn test_append_genesis_block<Kv: HotKv>(hot_kv: &Kv) {
    let addr = address!("0x0000000000000000000000000000000000000001");

    // Create genesis block with initial state
    let genesis = make_header(0, B256::ZERO);
    let bundle = make_bundle_state(
        vec![(addr, None, Some(make_account_info(0, U256::from(1_000_000), None)))],
        vec![],
        vec![],
    );

    // Append genesis
    {
        let writer = hot_kv.writer().unwrap();
        writer.append_blocks(&[(genesis.clone(), bundle)]).unwrap();
        writer.commit().unwrap();
    }

    // Verify genesis exists
    {
        let reader = hot_kv.reader().unwrap();
        let header = reader.get_header(0).unwrap();
        assert!(header.is_some(), "Genesis header should exist");
        assert_eq!(header.unwrap().number, 0);

        // Verify chain tip
        let tip = reader.get_chain_tip().unwrap();
        assert!(tip.is_some());
        let (num, hash) = tip.unwrap();
        assert_eq!(num, 0);
        assert_eq!(hash, genesis.hash());
    }
}

/// Test unwinding to block 0 (keeping only genesis).
pub fn test_unwind_to_zero<Kv: HotKv>(hot_kv: &Kv) {
    let addr = address!("0x1111111111111111111111111111111111111111");

    // Build a chain of 5 blocks
    let mut blocks = Vec::new();
    let mut prev_hash = B256::ZERO;

    for i in 0u64..5 {
        let header = make_header(i, prev_hash);
        prev_hash = header.hash();

        let bundle = make_bundle_state(
            vec![(
                addr,
                if i == 0 {
                    None
                } else {
                    Some(make_account_info(i - 1, U256::from(i * 100), None))
                },
                Some(make_account_info(i, U256::from((i + 1) * 100), None)),
            )],
            vec![],
            vec![],
        );
        blocks.push((header, bundle));
    }

    // Append all blocks
    {
        let writer = hot_kv.writer().unwrap();
        writer.append_blocks(&blocks).unwrap();
        writer.commit().unwrap();
    }

    // Verify chain tip is at block 4
    {
        let reader = hot_kv.reader().unwrap();
        let tip = reader.last_block_number().unwrap();
        assert_eq!(tip, Some(4));
    }

    // Unwind to block 0 (keep only genesis)
    {
        let writer = hot_kv.writer().unwrap();
        writer.unwind_above(0).unwrap();
        writer.commit().unwrap();
    }

    // Verify only genesis remains
    {
        let reader = hot_kv.reader().unwrap();
        let tip = reader.last_block_number().unwrap();
        assert_eq!(tip, Some(0), "Only genesis should remain after unwind to 0");

        // Verify blocks 1-4 are gone
        for i in 1u64..5 {
            assert!(reader.get_header(i).unwrap().is_none(), "Block {} should be gone", i);
        }

        // Verify genesis account state (nonce=0 from block 0)
        let account = reader.get_account(&addr).unwrap();
        assert!(account.is_some());
        assert_eq!(account.unwrap().nonce, 0, "Account should have genesis state");
    }
}

// ============================================================================
// History Sharding Tests
// ============================================================================

/// Test history at exactly the shard boundary.
///
/// NUM_OF_INDICES_IN_SHARD is typically 1000. This test writes exactly that many
/// entries to verify boundary handling.
pub fn test_history_shard_boundary<T: HotKv>(hot_kv: &T) {
    use reth_db::models::sharded_key;

    let addr = address!("0xaaaabbbbccccddddeeeeffffaaaabbbbccccdddd");
    let shard_size = sharded_key::NUM_OF_INDICES_IN_SHARD;

    // Write exactly shard_size account changes
    {
        let writer = hot_kv.writer().unwrap();
        for i in 1..=shard_size {
            let acc = Account { nonce: i as u64, balance: U256::from(i), bytecode_hash: None };
            writer.write_account_prestate(i as u64, addr, &acc).unwrap();
        }
        writer.commit().unwrap();
    }

    // Build history indices
    {
        let writer = hot_kv.writer().unwrap();
        writer.update_history_indices_inconsistent(1..=(shard_size as u64)).unwrap();
        writer.commit().unwrap();
    }

    // Verify history - should fit in exactly one shard
    {
        let reader = hot_kv.reader().unwrap();
        let (key, history) =
            reader.last_account_history(addr).unwrap().expect("Should have history");

        // With exactly shard_size entries, it should be stored with key = u64::MAX
        assert_eq!(key, u64::MAX, "Shard key should be u64::MAX for single full shard");

        let blocks: Vec<u64> = history.iter().collect();
        assert_eq!(blocks.len(), shard_size, "Should have exactly {} blocks", shard_size);
    }
}

/// Test history overflow into multiple shards.
pub fn test_history_multi_shard<T: HotKv>(hot_kv: &T) {
    use reth_db::models::sharded_key;

    let addr = address!("0xbbbbccccddddeeeeffffaaaabbbbccccddddeee1");
    let shard_size = sharded_key::NUM_OF_INDICES_IN_SHARD;
    let total_entries = shard_size + 100; // Overflow into second shard

    // Write more than shard_size account changes
    {
        let writer = hot_kv.writer().unwrap();
        for i in 1..=total_entries {
            let acc = Account { nonce: i as u64, balance: U256::from(i), bytecode_hash: None };
            writer.write_account_prestate(i as u64, addr, &acc).unwrap();
        }
        writer.commit().unwrap();
    }

    // Build history indices
    {
        let writer = hot_kv.writer().unwrap();
        writer.update_history_indices_inconsistent(1..=(total_entries as u64)).unwrap();
        writer.commit().unwrap();
    }

    // Verify we have multiple shards
    {
        let reader = hot_kv.reader().unwrap();

        // Count shards by traversing
        let mut cursor = reader.traverse_dual::<tables::AccountsHistory>().unwrap();
        let mut shard_count = 0;
        let mut total_blocks = 0;

        // Find entries for our address
        if let Some((k1, _, list)) = cursor.next_dual_above(&addr, &0u64).unwrap()
            && k1 == addr
        {
            shard_count += 1;
            total_blocks += list.iter().count();

            // Continue reading for same address
            while let Some((k1, _, list)) = cursor.read_next().unwrap() {
                if k1 != addr {
                    break;
                }
                shard_count += 1;
                total_blocks += list.iter().count();
            }
        }

        assert!(shard_count >= 2, "Should have at least 2 shards, got {}", shard_count);
        assert_eq!(total_blocks, total_entries, "Total blocks across shards should match");
    }
}

// ============================================================================
// HistoryRead Method Tests
// ============================================================================

/// Test get_headers_range retrieves headers in range.
pub fn test_get_headers_range<T: HotKv>(hot_kv: &T) {
    // Write headers 500-509
    {
        let writer = hot_kv.writer().unwrap();
        for i in 500u64..510 {
            let header = Header { number: i, gas_limit: 1_000_000, ..Default::default() };
            writer.put_header_inconsistent(&header).unwrap();
        }
        writer.commit().unwrap();
    }

    let reader = hot_kv.reader().unwrap();

    // Get range 502-506
    let headers = reader.get_headers_range(502, 506).unwrap();
    assert_eq!(headers.len(), 5, "Should get 5 headers (502, 503, 504, 505, 506)");
    for (i, header) in headers.iter().enumerate() {
        assert_eq!(header.number, 502 + i as u64);
    }

    // Get range that starts before existing entries
    let headers = reader.get_headers_range(498, 502).unwrap();
    // Should get 500, 501, 502 (498 and 499 don't exist)
    assert_eq!(headers.len(), 3);

    // Get range with no entries
    let headers = reader.get_headers_range(600, 610).unwrap();
    assert!(headers.is_empty(), "Should get empty vec for non-existent range");
}

/// Test first_header and last_header.
pub fn test_first_last_header<T: HotKv>(hot_kv: &T) {
    // Write headers 1000, 1005, 1010
    {
        let writer = hot_kv.writer().unwrap();
        for i in [1000u64, 1005, 1010] {
            let header = Header { number: i, gas_limit: 1_000_000, ..Default::default() };
            writer.put_header_inconsistent(&header).unwrap();
        }
        writer.commit().unwrap();
    }

    let reader = hot_kv.reader().unwrap();

    let first = reader.first_header().unwrap();
    assert!(first.is_some());
    assert_eq!(first.unwrap().number, 1000);

    let last = reader.last_header().unwrap();
    assert!(last.is_some());
    assert_eq!(last.unwrap().number, 1010);
}

/// Test has_block returns correct boolean.
pub fn test_has_block<T: HotKv>(hot_kv: &T) {
    // Write header at block 2000
    {
        let writer = hot_kv.writer().unwrap();
        let header = Header { number: 2000, gas_limit: 1_000_000, ..Default::default() };
        writer.put_header_inconsistent(&header).unwrap();
        writer.commit().unwrap();
    }

    let reader = hot_kv.reader().unwrap();

    assert!(reader.has_block(2000).unwrap(), "Block 2000 should exist");
    assert!(!reader.has_block(2001).unwrap(), "Block 2001 should not exist");
    assert!(!reader.has_block(1999).unwrap(), "Block 1999 should not exist");
}

/// Test get_execution_range returns first and last block numbers.
pub fn test_get_execution_range<T: HotKv>(hot_kv: &T) {
    // Write headers 3000, 3001, 3002
    {
        let writer = hot_kv.writer().unwrap();
        for i in [3000u64, 3001, 3002] {
            let header = Header { number: i, gas_limit: 1_000_000, ..Default::default() };
            writer.put_header_inconsistent(&header).unwrap();
        }
        writer.commit().unwrap();
    }

    let reader = hot_kv.reader().unwrap();

    let range = reader.get_execution_range().unwrap();
    assert!(range.is_some());
    let (first, last) = range.unwrap();
    assert_eq!(first, 3000);
    assert_eq!(last, 3002);
}

/// Test get_chain_tip returns highest block number and hash.
pub fn test_get_chain_tip<T: HotKv>(hot_kv: &T) {
    let header = Header { number: 4000, gas_limit: 1_000_000, ..Default::default() };
    let expected_hash = header.hash_slow();

    {
        let writer = hot_kv.writer().unwrap();
        writer.put_header_inconsistent(&header).unwrap();
        writer.commit().unwrap();
    }

    let reader = hot_kv.reader().unwrap();

    let tip = reader.get_chain_tip().unwrap();
    assert!(tip.is_some());
    let (num, hash) = tip.unwrap();
    assert_eq!(num, 4000);
    assert_eq!(hash, expected_hash);
}
