#![allow(dead_code)]

use crate::hot::{
    db::{HotDbRead, HotHistoryRead, UnsafeDbWrite, UnsafeHistoryWrite},
    model::{HotKv, HotKvWrite},
};
use alloy::primitives::{B256, Bytes, U256, address, b256};
use reth::primitives::{Account, Bytecode, Header, SealedHeader};
use reth_db::BlockNumberList;

/// Run all conformance tests against a [`HotKv`] implementation.
pub fn conformance<T: HotKv>(hot_kv: &T) {
    dbg!("Running HotKv conformance tests...");
    test_header_roundtrip(hot_kv);
    dbg!("Header roundtrip test passed.");
    test_account_roundtrip(hot_kv);
    dbg!("Account roundtrip test passed.");
    test_storage_roundtrip(hot_kv);
    dbg!("Storage roundtrip test passed.");
    test_bytecode_roundtrip(hot_kv);
    dbg!("Bytecode roundtrip test passed.");
    // test_account_history(hot_kv);
    // test_storage_history(hot_kv);
    // test_account_changes(hot_kv);
    // test_storage_changes(hot_kv);
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
    use crate::hot::tables;

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
    use crate::hot::tables;
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
    use crate::hot::tables;

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

// /// Test appending blocks with BundleState, unwinding, and re-appending.
// ///
// /// This test:
// /// 1. Appends 5 blocks with account and storage changes
// /// 2. Verifies state after append
// /// 3. Unwinds 2 blocks back to block 3
// /// 4. Verifies state after unwind
// /// 5. Appends 2 more blocks (different content)
// /// 6. Verifies final state
// fn test_append_and_unwind_blocks<T: HotKv>(hot_kv: &T) {
//     let addr1 = address!("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
//     let slot1 = b256!("0x0000000000000000000000000000000000000000000000000000000000000001");

//     // Helper to create a simple BundleState with account changes
//     // Since BundleState is complex to construct, we'll use the lower-level methods directly
//     // for this test rather than going through append_executed_block

//     // ========== Phase 1: Append 5 blocks using low-level methods ==========
//     let mut headers = Vec::new();
//     let mut prev_hash = B256::ZERO;

//     // Create 5 headers
//     for i in 1..=5 {
//         let header = make_header(i, prev_hash);
//         prev_hash = header.hash();
//         headers.push(header);
//     }

//     // Write blocks with state changes
//     // Use u64::MAX as the shard key for history to simplify lookups
//     let shard_key = u64::MAX;

//     {
//         let writer = hot_kv.writer().unwrap();

//         // Block 1: Create addr1 with nonce=1, balance=100
//         writer.put_header(&headers[0]).unwrap();
//         let acc1 = Account { nonce: 1, balance: U256::from(100), bytecode_hash: None };
//         writer.put_account(&addr1, &acc1).unwrap();
//         // Write change set (pre-state was empty)
//         let pre_acc1 = Account::default();
//         writer.write_account_prestate(1, addr1, &pre_acc1).unwrap();
//         // Write history
//         let history1 = BlockNumberList::new([1]).unwrap();
//         writer.write_account_history(&addr1, shard_key, &history1).unwrap();

//         // Block 2: Update addr1 nonce=2, balance=200
//         writer.put_header(&headers[1]).unwrap();
//         let acc2 = Account { nonce: 2, balance: U256::from(200), bytecode_hash: None };
//         // Write pre-state (was acc1)
//         writer.write_account_prestate(2, addr1, &acc1).unwrap();
//         writer.put_account(&addr1, &acc2).unwrap();
//         let history2 = BlockNumberList::new([1, 2]).unwrap();
//         writer.write_account_history(&addr1, shard_key, &history2).unwrap();

//         // Block 3: Update storage
//         writer.put_header(&headers[2]).unwrap();
//         let acc3 = Account { nonce: 2, balance: U256::from(200), bytecode_hash: None };
//         writer.put_account(&addr1, &acc3).unwrap();
//         writer.write_account_prestate(3, addr1, &acc2).unwrap();
//         // Add storage slot
//         writer.put_storage(&addr1, &slot1, &U256::from(999)).unwrap();
//         writer.write_storage_prestate(3, addr1, &slot1, &U256::ZERO).unwrap();
//         let acc_history3 = BlockNumberList::new([1, 2, 3]).unwrap();
//         writer.write_account_history(&addr1, shard_key, &acc_history3).unwrap();
//         let storage_history3 = BlockNumberList::new([3]).unwrap();
//         writer.write_storage_history(&addr1, slot1, shard_key, &storage_history3).unwrap();

//         // Block 4: Update both
//         writer.put_header(&headers[3]).unwrap();
//         let acc4 = Account { nonce: 3, balance: U256::from(300), bytecode_hash: None };
//         writer.write_account_prestate(4, addr1, &acc3).unwrap();
//         writer.put_account(&addr1, &acc4).unwrap();
//         writer.write_storage_prestate(4, addr1, &slot1, &U256::from(999)).unwrap();
//         writer.put_storage(&addr1, &slot1, &U256::from(1000)).unwrap();
//         let acc_history4 = BlockNumberList::new([1, 2, 3, 4]).unwrap();
//         writer.write_account_history(&addr1, shard_key, &acc_history4).unwrap();
//         let storage_history4 = BlockNumberList::new([3, 4]).unwrap();
//         writer.write_storage_history(&addr1, slot1, shard_key, &storage_history4).unwrap();

//         // Block 5: Final changes
//         writer.put_header(&headers[4]).unwrap();
//         let acc5 = Account { nonce: 4, balance: U256::from(400), bytecode_hash: None };
//         writer.write_account_prestate(5, addr1, &acc4).unwrap();
//         writer.put_account(&addr1, &acc5).unwrap();
//         let acc_history5 = BlockNumberList::new([1, 2, 3, 4, 5]).unwrap();
//         writer.write_account_history(&addr1, shard_key, &acc_history5).unwrap();

//         writer.commit().unwrap();
//     }

//     // Verify state after append
//     {
//         let reader = hot_kv.reader().unwrap();

//         // Check chain tip
//         let (tip_num, tip_hash) = reader.get_chain_tip().unwrap().unwrap();
//         assert_eq!(tip_num, 5);
//         assert_eq!(tip_hash, headers[4].hash());

//         // Check plain state
//         let acc = reader.get_account(&addr1).unwrap().unwrap();
//         assert_eq!(acc.nonce, 4);
//         assert_eq!(acc.balance, U256::from(400));

//         // Check storage
//         let val = reader.get_storage(&addr1, &slot1).unwrap().unwrap();
//         assert_eq!(val, U256::from(1000));

//         // Check account history contains block 5
//         let history = reader.get_account_history(&addr1, u64::MAX).unwrap().unwrap();
//         let history_blocks: Vec<u64> = history.iter().collect();
//         assert!(history_blocks.contains(&5));
//     }

//     // ========== Phase 2: Unwind 2 blocks (to block 3) ==========
//     {
//         let writer = hot_kv.writer().unwrap();
//         let unwound = writer.unwind_to(3).unwrap();
//         assert_eq!(unwound, 2);
//         writer.commit().unwrap();
//     }

//     // Verify state after unwind
//     {
//         let reader = hot_kv.reader().unwrap();

//         // Check chain tip
//         let (tip_num, _) = reader.get_chain_tip().unwrap().unwrap();
//         assert_eq!(tip_num, 3);

//         // Check plain state restored to block 3 values
//         let acc = reader.get_account(&addr1).unwrap().unwrap();
//         assert_eq!(acc.nonce, 2); // Restored to block 3 state
//         assert_eq!(acc.balance, U256::from(200));

//         // Check storage restored
//         let val = reader.get_storage(&addr1, &slot1).unwrap().unwrap();
//         assert_eq!(val, U256::from(999)); // Restored to block 3 value

//         // Check change sets for blocks 4,5 are gone
//         assert!(reader.get_account_change(4, &addr1).unwrap().is_none());
//         assert!(reader.get_account_change(5, &addr1).unwrap().is_none());
//     }

//     // ========== Phase 3: Append 2 more blocks ==========
//     let header4_new = make_header(4, headers[2].hash());
//     let header5_new = make_header(5, header4_new.hash());

//     {
//         let writer = hot_kv.writer().unwrap();

//         // Block 4 (new): Different state changes
//         writer.put_header(&header4_new).unwrap();
//         let acc4_new = Account { nonce: 3, balance: U256::from(350), bytecode_hash: None };
//         let acc3 = Account { nonce: 2, balance: U256::from(200), bytecode_hash: None };
//         writer.write_account_prestate(4, addr1, &acc3).unwrap();
//         writer.put_account(&addr1, &acc4_new).unwrap();
//         writer.write_storage_prestate(4, addr1, &slot1, &U256::from(999)).unwrap();
//         writer.put_storage(&addr1, &slot1, &U256::from(888)).unwrap();
//         let acc_history4_new = BlockNumberList::new([1, 2, 3, 4]).unwrap();
//         writer.write_account_history(&addr1, shard_key, &acc_history4_new).unwrap();
//         let storage_history4_new = BlockNumberList::new([3, 4]).unwrap();
//         writer.write_storage_history(&addr1, slot1, shard_key, &storage_history4_new).unwrap();

//         // Block 5 (new): More changes
//         writer.put_header(&header5_new).unwrap();
//         let acc5_new = Account { nonce: 4, balance: U256::from(450), bytecode_hash: None };
//         writer.write_account_prestate(5, addr1, &acc4_new).unwrap();
//         writer.put_account(&addr1, &acc5_new).unwrap();
//         let acc_history5_new = BlockNumberList::new([1, 2, 3, 4, 5]).unwrap();
//         writer.write_account_history(&addr1, shard_key, &acc_history5_new).unwrap();

//         writer.commit().unwrap();
//     }

//     // Verify final state
//     {
//         let reader = hot_kv.reader().unwrap();

//         // Check chain tip
//         let (tip_num, tip_hash) = reader.get_chain_tip().unwrap().unwrap();
//         assert_eq!(tip_num, 5);
//         assert_eq!(tip_hash, header5_new.hash());
//         assert_ne!(tip_hash, headers[4].hash()); // Different from original block 5

//         // Check plain state
//         let acc = reader.get_account(&addr1).unwrap().unwrap();
//         assert_eq!(acc.nonce, 4);
//         assert_eq!(acc.balance, U256::from(450)); // Different from original

//         // Check storage
//         let val = reader.get_storage(&addr1, &slot1).unwrap().unwrap();
//         assert_eq!(val, U256::from(888)); // Different from original
//     }
// }

/// Test clear_range on a single-keyed table.
///
/// This test verifies that:
/// 1. Keys within the range are deleted
/// 2. Keys outside the range remain intact
/// 3. Edge cases like adjacent keys and boundary conditions work correctly
pub fn test_clear_range<T: HotKv>(hot_kv: &T) {
    use crate::hot::tables;

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
    use crate::hot::tables;

    let headers = (0..10u64)
        .map(|i| Header { number: i, gas_limit: 1_000_000, ..Default::default() })
        .collect::<Vec<_>>();

    // Phase 1: Write 10 headers with block numbers 0-9
    {
        let writer = hot_kv.writer().unwrap();
        for header in headers.iter() {
            writer.put_header_inconsistent(&header).unwrap();
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
        use crate::hot::tables;
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
    use crate::hot::tables;

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
