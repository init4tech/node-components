//! Utilities for testing MDBX storage implementation.

use crate::hot::{
    db::UnsafeDbWrite,
    impls::mdbx::{DatabaseArguments, DatabaseEnv, DatabaseEnvKind},
    model::{HotKv, HotKvWrite},
    tables::{self, SingleKey, Table},
};
use alloy::primitives::Bytes;
use reth_libmdbx::MaxReadTransactionDuration;
use tempfile::{TempDir, tempdir};

// Test table definitions for traversal tests
#[derive(Debug)]
struct TestTable;

impl Table for TestTable {
    const NAME: &'static str = "mdbx_test_table";
    type Key = u64;
    type Value = Bytes;
}

impl SingleKey for TestTable {}

/// Creates a temporary MDBX database for testing that will be automatically
/// cleaned up when the TempDir is dropped.
pub fn create_test_rw_db() -> (TempDir, DatabaseEnv) {
    let dir = tempdir().unwrap();

    let args = DatabaseArguments::new()
        .with_max_read_transaction_duration(Some(MaxReadTransactionDuration::Unbounded));

    let db = DatabaseEnv::open(dir.path(), DatabaseEnvKind::RW, args).unwrap();

    // Create tables from the `crate::tables::hot` module
    let mut writer = db.writer().unwrap();

    writer.queue_create::<tables::Headers>().unwrap();
    writer.queue_create::<tables::HeaderNumbers>().unwrap();
    writer.queue_create::<tables::Bytecodes>().unwrap();
    writer.queue_create::<tables::PlainAccountState>().unwrap();
    writer.queue_create::<tables::AccountsHistory>().unwrap();
    writer.queue_create::<tables::StorageHistory>().unwrap();
    writer.queue_create::<tables::PlainStorageState>().unwrap();
    writer.queue_create::<tables::StorageChangeSets>().unwrap();
    writer.queue_create::<tables::AccountChangeSets>().unwrap();

    writer.queue_create::<TestTable>().unwrap();

    writer.commit().expect("Failed to commit table creation");

    (dir, db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hot::{
        conformance::conformance,
        db::UnsafeDbWrite,
        impls::mdbx::Tx,
        model::{DualTableTraverse, HotKv, HotKvRead, HotKvWrite, TableTraverse, TableTraverseMut},
        tables,
    };
    use alloy::primitives::{Address, B256, BlockNumber, Bytes, U256};
    use reth::primitives::{Account, Bytecode, Header, SealedHeader};
    use reth_libmdbx::{RO, RW};
    use serial_test::serial;

    /// Create a temporary MDBX database for testing that will be automatically cleaned up
    fn run_test<F: FnOnce(&DatabaseEnv)>(f: F) {
        let (dir, db) = create_test_rw_db();

        f(&db);

        drop(dir);
    }

    /// Create test data
    fn create_test_account() -> (Address, Account) {
        let address = Address::from_slice(&[0x1; 20]);
        let account = Account {
            nonce: 42,
            balance: U256::from(1000u64),
            bytecode_hash: Some(B256::from_slice(&[0x2; 32])),
        };
        (address, account)
    }

    fn create_test_bytecode() -> (B256, Bytecode) {
        let hash = B256::from_slice(&[0x2; 32]);
        let code = reth::primitives::Bytecode::new_raw(vec![0x60, 0x80, 0x60, 0x40].into());
        (hash, code)
    }

    fn create_test_header() -> (BlockNumber, Header) {
        let block_number = 12345;
        let header = Header {
            number: block_number,
            gas_limit: 8000000,
            gas_used: 100000,
            timestamp: 1640995200,
            parent_hash: B256::from_slice(&[0x3; 32]),
            state_root: B256::from_slice(&[0x4; 32]),
            ..Default::default()
        };
        (block_number, header)
    }

    #[test]
    #[serial]
    fn test_hotkv_basic_operations() {
        run_test(test_hotkv_basic_operations_inner);
    }

    fn test_hotkv_basic_operations_inner(db: &DatabaseEnv) {
        let (address, account) = create_test_account();
        let (hash, bytecode) = create_test_bytecode();

        // Test HotKv::writer() and basic write operations
        {
            let mut writer: Tx<RW> = db.writer().unwrap();

            // Create tables first
            writer.queue_create::<tables::Bytecodes>().unwrap();

            // Write account data
            writer.queue_put::<tables::PlainAccountState>(&address, &account).unwrap();
            writer.queue_put::<tables::Bytecodes>(&hash, &bytecode).unwrap();

            // Commit the transaction
            writer.raw_commit().unwrap();
        }

        // Test HotKv::reader() and basic read operations
        {
            let reader: Tx<RO> = db.reader().unwrap();

            // Read account data
            let read_account: Option<Account> =
                reader.get::<tables::PlainAccountState>(&address).unwrap();
            assert_eq!(read_account, Some(account));

            // Read bytecode
            let read_bytecode: Option<Bytecode> = reader.get::<tables::Bytecodes>(&hash).unwrap();
            assert_eq!(read_bytecode, Some(bytecode));

            // Test non-existent data
            let nonexistent_addr = Address::from_slice(&[0xff; 20]);
            let nonexistent_account: Option<Account> =
                reader.get::<tables::PlainAccountState>(&nonexistent_addr).unwrap();
            assert_eq!(nonexistent_account, None);
        }
    }

    #[test]
    #[serial]
    fn test_raw_operations() {
        run_test(test_raw_operations_inner)
    }

    fn test_raw_operations_inner(db: &DatabaseEnv) {
        let table_name = "test_table";
        let key = b"test_key";
        let value = b"test_value";

        // Test raw write operations
        {
            let mut writer: Tx<RW> = db.writer().unwrap();

            // Create table
            writer.queue_raw_create(table_name, None, None).unwrap();

            // Put raw data
            writer.queue_raw_put(table_name, key, value).unwrap();

            writer.raw_commit().unwrap();
        }

        // Test raw read operations
        {
            let reader: Tx<RO> = db.reader().unwrap();

            let read_value = reader.raw_get(table_name, key).unwrap();
            assert_eq!(read_value.as_deref(), Some(value.as_slice()));

            // Test non-existent key
            let nonexistent = reader.raw_get(table_name, b"nonexistent").unwrap();
            assert_eq!(nonexistent, None);
        }

        // Test raw delete
        {
            let mut writer: Tx<RW> = db.writer().unwrap();

            writer.queue_raw_delete(table_name, key).unwrap();
            writer.raw_commit().unwrap();
        }

        // Verify deletion
        {
            let reader: Tx<RO> = db.reader().unwrap();
            let deleted_value = reader.raw_get(table_name, key).unwrap();
            assert_eq!(deleted_value, None);
        }
    }

    #[test]
    #[serial]
    fn test_dual_keyed_operations() {
        run_test(test_dual_keyed_operations_inner)
    }

    fn test_dual_keyed_operations_inner(db: &DatabaseEnv) {
        let address = Address::from_slice(&[0x1; 20]);
        let storage_key = B256::from_slice(&[0x5; 32]);
        let storage_value = U256::from(999u64);

        // Test dual-keyed table operations
        {
            let mut writer: Tx<RW> = db.writer().unwrap();

            // Put storage data using dual keys
            writer
                .queue_put_dual::<tables::PlainStorageState>(&address, &storage_key, &storage_value)
                .unwrap();

            writer.raw_commit().unwrap();
        }

        // Test reading dual-keyed data
        {
            let reader: Tx<RO> = db.reader().unwrap();

            // Read storage using dual key lookup
            let read_value = reader
                .get_dual::<tables::PlainStorageState>(&address, &storage_key)
                .unwrap()
                .unwrap();

            assert_eq!(read_value, storage_value);
        }
    }

    #[test]
    #[serial]
    fn test_table_management() {
        run_test(test_table_management_inner)
    }

    fn test_table_management_inner(db: &DatabaseEnv) {
        // Add some data
        let (block_number, header) = create_test_header();
        {
            let mut writer: Tx<RW> = db.writer().unwrap();
            writer.queue_put::<tables::Headers>(&block_number, &header).unwrap();
            writer.raw_commit().unwrap();
        }

        // Verify data exists
        {
            let reader: Tx<RO> = db.reader().unwrap();
            let read_header: Option<Header> = reader.get::<tables::Headers>(&block_number).unwrap();
            assert_eq!(read_header, Some(header.clone()));
        }

        // Clear the table
        {
            let mut writer: Tx<RW> = db.writer().unwrap();
            writer.queue_clear::<tables::Headers>().unwrap();
            writer.raw_commit().unwrap();
        }

        // Verify table is empty
        {
            let reader: Tx<RO> = db.reader().unwrap();
            let read_header: Option<Header> = reader.get::<tables::Headers>(&block_number).unwrap();
            assert_eq!(read_header, None);
        }
    }

    #[test]
    fn test_batch_operations() {
        run_test(test_batch_operations_inner)
    }

    fn test_batch_operations_inner(db: &DatabaseEnv) {
        // Create test data
        let accounts: Vec<(Address, Account)> = (0..10)
            .map(|i| {
                let mut addr_bytes = [0u8; 20];
                addr_bytes[19] = i;
                let address = Address::from_slice(&addr_bytes);
                let account = Account {
                    nonce: i.into(),
                    balance: U256::from((i as u64) * 100),
                    bytecode_hash: None,
                };
                (address, account)
            })
            .collect();

        // Test batch writes
        {
            let mut writer: Tx<RW> = db.writer().unwrap();

            // Write multiple accounts
            for (address, account) in &accounts {
                writer.queue_put::<tables::PlainAccountState>(address, account).unwrap();
            }

            writer.raw_commit().unwrap();
        }

        // Test batch reads
        {
            let reader: Tx<RO> = db.reader().unwrap();

            for (address, expected_account) in &accounts {
                let read_account: Option<Account> =
                    reader.get::<tables::PlainAccountState>(address).unwrap();
                assert_eq!(read_account.as_ref(), Some(expected_account));
            }
        }

        // Test batch get_many
        {
            let reader: Tx<RO> = db.reader().unwrap();
            let addresses: Vec<Address> = accounts.iter().map(|(addr, _)| *addr).collect();
            let read_accounts: Vec<(_, Option<Account>)> =
                reader.get_many::<tables::PlainAccountState, _>(addresses.iter()).unwrap();

            for (i, (_, expected_account)) in accounts.iter().enumerate() {
                assert_eq!(read_accounts[i].1.as_ref(), Some(expected_account));
            }
        }
    }

    #[test]
    fn test_transaction_isolation() {
        run_test(test_transaction_isolation_inner)
    }

    fn test_transaction_isolation_inner(db: &DatabaseEnv) {
        let (address, account) = create_test_account();

        // Setup initial data
        {
            let mut writer: Tx<RW> = db.writer().unwrap();
            writer.queue_put::<tables::PlainAccountState>(&address, &account).unwrap();
            writer.raw_commit().unwrap();
        }

        // Start a reader transaction
        let reader: Tx<RO> = db.reader().unwrap();

        // Modify data in a writer transaction
        {
            let mut writer: Tx<RW> = db.writer().unwrap();
            let modified_account =
                Account { nonce: 999, balance: U256::from(9999u64), bytecode_hash: None };
            writer.queue_put::<tables::PlainAccountState>(&address, &modified_account).unwrap();
            writer.raw_commit().unwrap();
        }

        // Reader should still see original data (snapshot isolation)
        {
            let read_account: Option<Account> =
                reader.get::<tables::PlainAccountState>(&address).unwrap();
            assert_eq!(read_account, Some(account));
        }

        // New reader should see modified data
        {
            let new_reader: Tx<RO> = db.reader().unwrap();
            let read_account: Option<Account> =
                new_reader.get::<tables::PlainAccountState>(&address).unwrap();
            assert_eq!(read_account.unwrap().nonce, 999);
        }
    }

    #[test]
    fn test_multiple_readers() {
        run_test(test_multiple_readers_inner)
    }

    fn test_multiple_readers_inner(db: &DatabaseEnv) {
        let (address, account) = create_test_account();

        // Setup data
        {
            let mut writer: Tx<RW> = db.writer().unwrap();
            writer.queue_put::<tables::PlainAccountState>(&address, &account).unwrap();
            writer.raw_commit().unwrap();
        }

        // Create multiple readers
        let reader1: Tx<RO> = db.reader().unwrap();
        let reader2: Tx<RO> = db.reader().unwrap();
        let reader3: Tx<RO> = db.reader().unwrap();

        // All readers should see the same data
        let account1: Option<Account> = reader1.get::<tables::PlainAccountState>(&address).unwrap();
        let account2: Option<Account> = reader2.get::<tables::PlainAccountState>(&address).unwrap();
        let account3: Option<Account> = reader3.get::<tables::PlainAccountState>(&address).unwrap();

        assert_eq!(account1, Some(account));
        assert_eq!(account2, Some(account));
        assert_eq!(account3, Some(account));
    }

    #[test]
    fn test_error_handling() {
        run_test(test_error_handling_inner)
    }

    fn test_error_handling_inner(db: &DatabaseEnv) {
        // Test reading from non-existent table
        {
            let reader: Tx<RO> = db.reader().unwrap();
            let result = reader.raw_get("nonexistent_table", b"key");

            // Should handle gracefully (may return None or error depending on MDBX behavior)
            match result {
                Ok(None) => {} // This is fine
                Err(_) => {}   // This is also acceptable for non-existent table
                Ok(Some(_)) => panic!("Should not return data for non-existent table"),
            }
        }

        // Test writing to a table without creating it first
        {
            let mut writer: Tx<RW> = db.writer().unwrap();
            let (address, account) = create_test_account();

            // This should handle the case where table doesn't exist
            let result = writer.queue_put::<tables::PlainAccountState>(&address, &account);
            match result {
                Ok(_) => {
                    // If it succeeds, commit should work
                    writer.raw_commit().unwrap();
                }
                Err(_) => {
                    // If it fails, that's expected behavior
                }
            }
        }
    }

    #[test]
    fn test_serialization_roundtrip() {
        run_test(test_serialization_roundtrip_inner)
    }

    fn test_serialization_roundtrip_inner(db: &DatabaseEnv) {
        // Test various data types
        let (block_number, header) = create_test_header();
        let header = SealedHeader::new_unhashed(header);

        {
            let mut writer: Tx<RW> = db.writer().unwrap();

            // Write different types
            writer.put_header(&header).unwrap();

            writer.raw_commit().unwrap();
        }

        {
            let reader: Tx<RO> = db.reader().unwrap();

            // Read and verify
            let read_header: Option<Header> = reader.get::<tables::Headers>(&block_number).unwrap();
            assert_eq!(read_header.as_ref(), Some(header.header()));

            let read_hash: Option<u64> =
                reader.get::<tables::HeaderNumbers>(&header.hash()).unwrap();
            assert_eq!(read_hash, Some(header.number));
        }
    }

    #[test]
    fn test_large_data() {
        run_test(test_large_data_inner)
    }

    fn test_large_data_inner(db: &DatabaseEnv) {
        // Create a large bytecode
        let hash = B256::from_slice(&[0x8; 32]);
        let large_code_vec: Vec<u8> = (0..10000).map(|i| (i % 256) as u8).collect();
        let large_bytecode = Bytecode::new_raw(large_code_vec.clone().into());

        {
            let mut writer: Tx<RW> = db.writer().unwrap();
            writer.queue_create::<tables::Bytecodes>().unwrap();
            writer.queue_put::<tables::Bytecodes>(&hash, &large_bytecode).unwrap();
            writer.raw_commit().unwrap();
        }

        {
            let reader: Tx<RO> = db.reader().unwrap();
            let read_bytecode: Option<Bytecode> = reader.get::<tables::Bytecodes>(&hash).unwrap();
            assert_eq!(read_bytecode, Some(large_bytecode));
        }
    }

    // ========================================================================
    // Cursor Traversal Tests
    // ========================================================================

    #[test]
    fn test_table_traverse_basic_navigation() {
        run_test(test_table_traverse_basic_navigation_inner)
    }

    fn test_table_traverse_basic_navigation_inner(db: &DatabaseEnv) {
        // Setup test data with multiple entries
        let test_data: Vec<(u64, Bytes)> = vec![
            (1, Bytes::from_static(b"value_001")),
            (2, Bytes::from_static(b"value_002")),
            (3, Bytes::from_static(b"value_003")),
            (10, Bytes::from_static(b"value_010")),
            (20, Bytes::from_static(b"value_020")),
        ];

        // Insert test data
        {
            let mut writer: Tx<RW> = db.writer().unwrap();
            for (key, value) in &test_data {
                writer.queue_put::<TestTable>(key, value).unwrap();
            }
            writer.raw_commit().unwrap();
        }

        // Test cursor traversal
        {
            let tx: Tx<RO> = db.reader().unwrap();
            let mut cursor = tx.new_cursor::<TestTable>().unwrap();

            // Test first()
            let first_result = TableTraverse::<TestTable, _>::first(&mut cursor).unwrap();
            assert!(first_result.is_some());
            let (key, value) = first_result.unwrap();
            assert_eq!(key, test_data[0].0);
            assert_eq!(value, test_data[0].1);

            // Test last()
            let last_result = TableTraverse::<TestTable, _>::last(&mut cursor).unwrap();
            assert!(last_result.is_some());
            let (key, value) = last_result.unwrap();
            assert_eq!(key, test_data.last().unwrap().0);
            assert_eq!(value, test_data.last().unwrap().1);

            // Test exact lookup
            let exact_result = TableTraverse::<TestTable, _>::exact(&mut cursor, &2u64).unwrap();
            assert!(exact_result.is_some());
            assert_eq!(exact_result.unwrap(), test_data[1].1);

            // Test exact lookup for non-existent key
            let missing_result =
                TableTraverse::<TestTable, _>::exact(&mut cursor, &999u64).unwrap();
            assert!(missing_result.is_none());

            // Test next_above (range lookup)
            let range_result =
                TableTraverse::<TestTable, _>::lower_bound(&mut cursor, &5u64).unwrap();
            assert!(range_result.is_some());
            let (key, value) = range_result.unwrap();
            assert_eq!(key, test_data[3].0); // key 10
            assert_eq!(value, test_data[3].1);
        }
    }

    #[test]
    fn test_table_traverse_sequential_navigation() {
        run_test(test_table_traverse_sequential_navigation_inner)
    }

    fn test_table_traverse_sequential_navigation_inner(db: &DatabaseEnv) {
        // Setup sequential test data
        let test_data: Vec<(u64, Bytes)> = (1..=10)
            .map(|i| {
                let s = format!("value_{:03}", i);
                let s = s.as_bytes();
                let value = Bytes::copy_from_slice(s);
                (i, value)
            })
            .collect();

        // Insert test data
        {
            let mut writer: Tx<RW> = db.writer().unwrap();
            for (key, value) in &test_data {
                writer.queue_put::<TestTable>(key, value).unwrap();
            }
            writer.raw_commit().unwrap();
        }

        // Test sequential navigation
        {
            let tx: Tx<RO> = db.reader().unwrap();
            let mut cursor = tx.new_cursor::<TestTable>().unwrap();

            // Start from first and traverse forward
            let mut current_idx = 0;
            let first_result = TableTraverse::<TestTable, _>::first(&mut cursor).unwrap();
            assert!(first_result.is_some());

            let (key, value) = first_result.unwrap();
            assert_eq!(key, test_data[current_idx].0);
            assert_eq!(value, test_data[current_idx].1);

            // Navigate forward through all entries
            while current_idx < test_data.len() - 1 {
                let next_result = TableTraverse::<TestTable, _>::read_next(&mut cursor).unwrap();
                assert!(next_result.is_some());

                current_idx += 1;
                let (key, value) = next_result.unwrap();
                assert_eq!(key, test_data[current_idx].0);
                assert_eq!(value, test_data[current_idx].1);
            }

            // Next should return None at the end
            let beyond_end = TableTraverse::<TestTable, _>::read_next(&mut cursor).unwrap();
            assert!(beyond_end.is_none());

            // Navigate backward
            while current_idx > 0 {
                let prev_result = TableTraverse::<TestTable, _>::read_prev(&mut cursor).unwrap();
                assert!(prev_result.is_some());

                current_idx -= 1;
                let (key, value) = prev_result.unwrap();
                assert_eq!(key, test_data[current_idx].0);
                assert_eq!(value, test_data[current_idx].1);
            }

            // Previous should return None at the beginning
            let before_start = TableTraverse::<TestTable, _>::read_prev(&mut cursor).unwrap();
            assert!(before_start.is_none());
        }
    }

    #[test]
    fn test_table_traverse_mut_delete() {
        run_test(test_table_traverse_mut_delete_inner)
    }

    fn test_table_traverse_mut_delete_inner(db: &DatabaseEnv) {
        let test_data: Vec<(u64, Bytes)> = vec![
            (1, Bytes::from_static(b"delete_value_1")),
            (2, Bytes::from_static(b"delete_value_2")),
            (3, Bytes::from_static(b"delete_value_3")),
        ];

        // Insert test data
        {
            let mut writer: Tx<RW> = db.writer().unwrap();
            for (key, value) in &test_data {
                writer.queue_put::<TestTable>(key, value).unwrap();
            }
            writer.raw_commit().unwrap();
        }
        // Test cursor deletion
        {
            let tx: Tx<RW> = db.writer().unwrap();

            let mut cursor = tx.new_cursor::<TestTable>().unwrap();

            // Navigate to middle entry
            let first = TableTraverse::<TestTable, _>::first(&mut cursor).unwrap().unwrap();
            assert_eq!(first.0, test_data[0].0);

            let next = TableTraverse::<TestTable, _>::read_next(&mut cursor).unwrap().unwrap();
            assert_eq!(next.0, test_data[1].0);

            // Delete current entry (key 2)
            TableTraverseMut::<TestTable, _>::delete_current(&mut cursor).unwrap();

            drop(cursor);
            tx.raw_commit().unwrap();
        }

        // Verify deletion
        {
            let tx: Tx<RO> = db.reader().unwrap();
            let mut cursor = tx.new_cursor::<TestTable>().unwrap();

            // Should only have first and third entries
            let first = TableTraverse::<TestTable, _>::first(&mut cursor).unwrap().unwrap();
            assert_eq!(first.0, test_data[0].0);

            let second = TableTraverse::<TestTable, _>::read_next(&mut cursor).unwrap().unwrap();
            assert_eq!(second.0, test_data[2].0);

            // Should be no more entries
            let none = TableTraverse::<TestTable, _>::read_next(&mut cursor).unwrap();
            assert!(none.is_none());

            // Verify deleted key is gone
            let missing =
                TableTraverse::<TestTable, _>::exact(&mut cursor, &test_data[1].0).unwrap();
            assert!(missing.is_none());
        }
    }

    #[test]
    fn test_table_traverse_accounts() {
        run_test(test_table_traverse_accounts_inner)
    }

    fn test_table_traverse_accounts_inner(db: &DatabaseEnv) {
        // Setup test accounts
        let test_accounts: Vec<(Address, Account)> = (0..5)
            .map(|i| {
                let mut addr_bytes = [0u8; 20];
                addr_bytes[19] = i;
                let address = Address::from_slice(&addr_bytes);
                let account = Account {
                    nonce: (i as u64) * 10,
                    balance: U256::from((i as u64) * 1000),
                    bytecode_hash: if i % 2 == 0 { Some(B256::from_slice(&[i; 32])) } else { None },
                };
                (address, account)
            })
            .collect();

        // Insert test data
        {
            let mut writer: Tx<RW> = db.writer().unwrap();

            for (address, account) in &test_accounts {
                writer.queue_put::<tables::PlainAccountState>(address, account).unwrap();
            }

            writer.raw_commit().unwrap();
        }

        // Test typed table traversal
        {
            let tx: Tx<RO> = db.reader().unwrap();
            let mut cursor = tx.new_cursor::<tables::PlainAccountState>().unwrap();

            // Test first with type-safe operations
            let first_raw =
                TableTraverse::<tables::PlainAccountState, _>::first(&mut cursor).unwrap();
            assert!(first_raw.is_some());
            let (first_key, first_account) = first_raw.unwrap();
            assert_eq!(first_key, test_accounts[0].0);
            assert_eq!(first_account, test_accounts[0].1);

            // Test last
            let last_raw =
                TableTraverse::<tables::PlainAccountState, _>::last(&mut cursor).unwrap();
            assert!(last_raw.is_some());
            let (last_key, last_account) = last_raw.unwrap();
            assert_eq!(last_key, test_accounts.last().unwrap().0);
            assert_eq!(last_account, test_accounts.last().unwrap().1);

            // Test exact lookup
            let target_address = &test_accounts[2].0;
            let exact_account =
                TableTraverse::<tables::PlainAccountState, _>::exact(&mut cursor, target_address)
                    .unwrap();
            assert!(exact_account.is_some());
            assert_eq!(exact_account.unwrap(), test_accounts[2].1);

            // Test range lookup
            let mut partial_addr = [0u8; 20];
            partial_addr[19] = 3; // Between entries 2 and 3
            let range_addr = Address::from_slice(&partial_addr);

            let range_result = TableTraverse::<tables::PlainAccountState, _>::lower_bound(
                &mut cursor,
                &range_addr,
            )
            .unwrap();
            assert!(range_result.is_some());
            let (found_addr, found_account) = range_result.unwrap();
            assert_eq!(found_addr, test_accounts[3].0);
            assert_eq!(found_account, test_accounts[3].1);
        }
    }

    #[test]
    fn test_dual_table_traverse() {
        run_test(test_dual_table_traverse_inner)
    }

    fn test_dual_table_traverse_inner(db: &DatabaseEnv) {
        let one_addr = Address::repeat_byte(0x01);
        let two_addr = Address::repeat_byte(0x02);

        let one_slot = B256::with_last_byte(0x01);
        let two_slot = B256::with_last_byte(0x06);
        let three_slot = B256::with_last_byte(0x09);

        let one_value = U256::from(0x100);
        let two_value = U256::from(0x200);
        let three_value = U256::from(0x300);
        let four_value = U256::from(0x400);
        let five_value = U256::from(0x500);

        // Setup test storage data
        let test_storage: Vec<(Address, B256, U256)> = vec![
            (one_addr, one_slot, one_value),
            (one_addr, two_slot, two_value),
            (one_addr, three_slot, three_value),
            (two_addr, one_slot, four_value),
            (two_addr, two_slot, five_value),
        ];

        // Insert test data
        {
            let mut writer: Tx<RW> = db.writer().unwrap();

            for (address, storage_key, value) in &test_storage {
                writer
                    .queue_put_dual::<tables::PlainStorageState>(address, storage_key, value)
                    .unwrap();
            }

            writer.raw_commit().unwrap();
        }

        // Test dual-keyed traversal
        {
            let tx: Tx<RO> = db.reader().unwrap();
            let mut cursor = tx.new_cursor::<tables::PlainStorageState>().unwrap();

            // Test exact dual lookup
            let address = &test_storage[1].0;
            let storage_key = &test_storage[1].1;
            let expected_value = &test_storage[1].2;

            let exact_result = DualTableTraverse::<tables::PlainStorageState, _>::exact_dual(
                &mut cursor,
                address,
                storage_key,
            )
            .unwrap()
            .unwrap();
            assert_eq!(exact_result, *expected_value);

            // Test range lookup for dual keys
            let search_key = B256::with_last_byte(0x02);
            let range_result = DualTableTraverse::<tables::PlainStorageState, _>::next_dual_above(
                &mut cursor,
                &test_storage[0].0, // Address 0x01
                &search_key,
            )
            .unwrap()
            .unwrap();

            let (found_addr, found_key, found_value) = range_result;
            assert_eq!(found_addr, test_storage[1].0); // Same address
            assert_eq!(found_key, test_storage[1].1); // Next storage key (0x02)
            assert_eq!(found_value, test_storage[1].2); // Corresponding value

            // Test next_k1 (move to next primary key)
            // First position cursor at first entry of first address
            DualTableTraverse::<tables::PlainStorageState, _>::exact_dual(
                &mut cursor,
                &test_storage[0].0,
                &test_storage[0].1,
            )
            .unwrap();

            // Move to next primary key (different address)
            let next_k1_result =
                DualTableTraverse::<tables::PlainStorageState, _>::next_k1(&mut cursor).unwrap();
            assert!(next_k1_result.is_some());
            let (next_addr, next_storage_key, next_value) = next_k1_result.unwrap();
            assert_eq!(next_addr, test_storage[3].0); // Address 0x02
            assert_eq!(next_storage_key, test_storage[3].1); // First storage key for new address
            assert_eq!(next_value, test_storage[3].2);
        }
    }

    #[test]
    fn test_dual_table_traverse_empty_results() {
        run_test(test_dual_table_traverse_empty_results_inner)
    }

    fn test_dual_table_traverse_empty_results_inner(db: &DatabaseEnv) {
        // Setup minimal test data
        let address = Address::from_slice(&[0x01; 20]);
        let storage_key = B256::from_slice(&[0x01; 32]);
        let value = U256::from(100);

        {
            let mut writer: Tx<RW> = db.writer().unwrap();
            writer
                .queue_put_dual::<tables::PlainStorageState>(&address, &storage_key, &value)
                .unwrap();
            writer.raw_commit().unwrap();
        }

        {
            let tx: Tx<RO> = db.reader().unwrap();
            let mut cursor = tx.new_cursor::<tables::PlainStorageState>().unwrap();

            // Test exact lookup for non-existent dual key
            let missing_addr = Address::from_slice(&[0xFF; 20]);
            let missing_key = B256::from_slice(&[0xFF; 32]);

            let exact_missing = DualTableTraverse::<tables::PlainStorageState, _>::exact_dual(
                &mut cursor,
                &missing_addr,
                &missing_key,
            )
            .unwrap();
            assert!(exact_missing.is_none());

            // Test range lookup beyond all data
            let beyond_key = B256::from_slice(&[0xFF; 32]);
            let range_missing = DualTableTraverse::<tables::PlainStorageState, _>::next_dual_above(
                &mut cursor,
                &address,
                &beyond_key,
            )
            .unwrap();
            assert!(range_missing.is_none());

            // Position at the only entry, then try next_k1
            DualTableTraverse::<tables::PlainStorageState, _>::exact_dual(
                &mut cursor,
                &address,
                &storage_key,
            )
            .unwrap();

            let next_k1_missing =
                DualTableTraverse::<tables::PlainStorageState, _>::next_k1(&mut cursor).unwrap();
            assert!(next_k1_missing.is_none());
        }
    }

    #[test]
    fn test_table_traverse_empty_table() {
        run_test(test_table_traverse_empty_table_inner)
    }

    fn test_table_traverse_empty_table_inner(db: &DatabaseEnv) {
        // TestTable is already created but empty
        {
            let tx: Tx<RO> = db.reader().unwrap();
            let mut cursor = tx.new_cursor::<TestTable>().unwrap();

            // All operations should return None on empty table
            assert!(TableTraverse::<TestTable, _>::first(&mut cursor).unwrap().is_none());
            assert!(TableTraverse::<TestTable, _>::last(&mut cursor).unwrap().is_none());
            assert!(TableTraverse::<TestTable, _>::exact(&mut cursor, &42u64).unwrap().is_none());
            assert!(
                TableTraverse::<TestTable, _>::lower_bound(&mut cursor, &42u64).unwrap().is_none()
            );
            assert!(TableTraverse::<TestTable, _>::read_next(&mut cursor).unwrap().is_none());
            assert!(TableTraverse::<TestTable, _>::read_prev(&mut cursor).unwrap().is_none());
        }
    }

    #[test]
    fn test_table_traverse_state_management() {
        run_test(test_table_traverse_state_management_inner)
    }

    fn test_table_traverse_state_management_inner(db: &DatabaseEnv) {
        let test_data: Vec<(u64, Bytes)> = vec![
            (1, Bytes::from_static(b"state_value_1")),
            (2, Bytes::from_static(b"state_value_2")),
            (3, Bytes::from_static(b"state_value_3")),
        ];

        {
            let mut writer: Tx<RW> = db.writer().unwrap();
            for (key, value) in &test_data {
                writer.queue_put::<TestTable>(key, value).unwrap();
            }
            writer.raw_commit().unwrap();
        }

        {
            let tx: Tx<RO> = db.reader().unwrap();
            let mut cursor = tx.new_cursor::<TestTable>().unwrap();

            // Test that cursor operations maintain state correctly

            // Start at first
            let first = TableTraverse::<TestTable, _>::first(&mut cursor).unwrap().unwrap();
            assert_eq!(first.0, test_data[0].0);

            // Move to second via next
            let second = TableTraverse::<TestTable, _>::read_next(&mut cursor).unwrap().unwrap();
            assert_eq!(second.0, test_data[1].0);

            // Jump to last
            let last = TableTraverse::<TestTable, _>::last(&mut cursor).unwrap().unwrap();
            assert_eq!(last.0, test_data[2].0);

            // Move back via prev
            let back_to_second =
                TableTraverse::<TestTable, _>::read_prev(&mut cursor).unwrap().unwrap();
            assert_eq!(back_to_second.0, test_data[1].0);

            // Use exact to jump to specific position
            let exact_first =
                TableTraverse::<TestTable, _>::exact(&mut cursor, &test_data[0].0).unwrap();
            assert!(exact_first.is_some());
            assert_eq!(exact_first.unwrap(), test_data[0].1);

            // Verify cursor is now positioned at first entry
            let next_from_first =
                TableTraverse::<TestTable, _>::read_next(&mut cursor).unwrap().unwrap();
            assert_eq!(next_from_first.0, test_data[1].0);

            // Use range lookup - look for key >= 1, should find key 1
            let range_lookup =
                TableTraverse::<TestTable, _>::lower_bound(&mut cursor, &1u64).unwrap().unwrap();
            assert_eq!(range_lookup.0, test_data[0].0); // Should find key 1

            // Verify we can continue navigation from range position
            let next_after_range =
                TableTraverse::<TestTable, _>::read_next(&mut cursor).unwrap().unwrap();
            assert_eq!(next_after_range.0, test_data[1].0);
        }
    }

    #[test]
    fn mdbx_conformance() {
        run_test(conformance)
    }

    #[test]
    fn test_cache_db_info() {
        run_test(test_cache_db_info_inner)
    }

    fn test_cache_db_info_inner(db: &DatabaseEnv) {
        // Tables are already created in create_test_rw_db()
        // Try to get cache_db_info for an existing table
        let reader: Tx<RO> = db.reader().unwrap();

        // This should work - Headers table was created in setup
        reader.cache_db_info::<tables::Headers>().unwrap();

        // Try with TestTable which was also created
        reader.cache_db_info::<TestTable>().unwrap();

        // Use a DUP_FIXED table and assert the result contains the expected
        // flags
        let result3 = reader.cache_db_info::<tables::PlainStorageState>().unwrap();
        assert!(result3.is_dupfixed());
    }

    #[test]
    fn test_storage_roundtrip_debug() {
        run_test(test_storage_roundtrip_debug_inner)
    }

    fn test_storage_roundtrip_debug_inner(db: &DatabaseEnv) {
        use alloy::primitives::{address, b256};

        let addr = address!("0xabcdef0123456789abcdef0123456789abcdef01");
        let slot = b256!("0x0000000000000000000000000000000000000000000000000000000000000001");
        let value = U256::from(999);

        // Write storage
        {
            let mut writer: Tx<RW> = db.writer().unwrap();

            // Check db_info before write
            {
                let db_info = writer.cache_db_info::<tables::PlainStorageState>().unwrap();
                assert!(db_info.is_dupfixed());
            }

            writer.queue_put_dual::<tables::PlainStorageState>(&addr, &slot, &value).unwrap();
            writer.raw_commit().unwrap();
        }

        // Read storage
        {
            let reader: Tx<RO> = db.reader().unwrap();

            // Check db_info after write
            {
                let db_info = reader.cache_db_info::<tables::PlainStorageState>().unwrap();
                assert!(db_info.is_dupfixed());
            }

            let read_value = reader.get_dual::<tables::PlainStorageState>(&addr, &slot).unwrap();
            assert!(read_value.is_some());
            assert_eq!(read_value.unwrap(), U256::from(999));
        }
    }

    // #[test]
    // fn mdbx_append_unwind_conformance() {
    //     let (dir, db) = create_test_rw_db();
    //     conformance_append_unwind(&db);
    // }
}
