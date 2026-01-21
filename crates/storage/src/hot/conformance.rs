#![allow(dead_code)]

use crate::hot::{
    db::{HotDbRead, HotHistoryRead, UnsafeDbWrite, UnsafeHistoryWrite},
    model::HotKv,
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
