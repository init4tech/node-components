/// An in-memory key-value store implementation.
pub mod mem;

/// MDBX-backed key-value store implementation.
pub mod mdbx;

#[cfg(test)]
mod test {
    use crate::{
        hot::{
            mem,
            model::{HotDbRead, HotDbWrite, HotHistoryRead, HotHistoryWrite, HotKv, HotKvWrite},
        },
        tables::hot,
    };
    use alloy::primitives::{B256, Bytes, U256, address, b256};
    use reth::primitives::{Account, Bytecode, Header, SealedHeader};
    use reth_db::{
        BlockNumberList, ClientVersion, mdbx::DatabaseArguments, test_utils::tempdir_path,
    };
    use reth_libmdbx::MaxReadTransactionDuration;

    #[test]
    fn mem_conformance() {
        let hot_kv = mem::MemKv::new();
        conformance(&hot_kv);
    }

    #[test]
    fn mdbx_conformance() {
        let path = tempdir_path();
        let db = reth_db::create_db(
            &path,
            DatabaseArguments::new(ClientVersion::default())
                .with_max_read_transaction_duration(Some(MaxReadTransactionDuration::Unbounded)),
        )
        .unwrap();

        // Create tables from the `crate::tables::hot` module
        let mut writer = db.writer().unwrap();

        writer.queue_create::<hot::Headers>().unwrap();
        writer.queue_create::<hot::HeaderNumbers>().unwrap();
        writer.queue_create::<hot::Bytecodes>().unwrap();
        writer.queue_create::<hot::PlainAccountState>().unwrap();
        writer.queue_create::<hot::AccountsHistory>().unwrap();
        writer.queue_create::<hot::StorageHistory>().unwrap();
        writer.queue_create::<hot::PlainStorageState>().unwrap();
        writer.queue_create::<hot::StorageChangeSets>().unwrap();
        writer.queue_create::<hot::AccountChangeSets>().unwrap();

        writer.commit().expect("Failed to commit table creation");

        conformance(&db);
    }

    fn conformance<T: HotKv>(hot_kv: &T) {
        test_header_roundtrip(hot_kv);
        test_account_roundtrip(hot_kv);
        test_storage_roundtrip(hot_kv);
        test_bytecode_roundtrip(hot_kv);
        test_account_history(hot_kv);
        test_storage_history(hot_kv);
        test_account_changes(hot_kv);
        test_storage_changes(hot_kv);
        test_missing_reads(hot_kv);
    }

    /// Test writing and reading headers via HotDbWrite/HotDbRead
    fn test_header_roundtrip<T: HotKv>(hot_kv: &T) {
        let header = Header { number: 42, gas_limit: 1_000_000, ..Default::default() };
        let sealed = SealedHeader::seal_slow(header.clone());
        let hash = sealed.hash();

        // Write header
        {
            let mut writer = hot_kv.writer().unwrap();
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
        let account =
            Account { nonce: 5, balance: U256::from(1000), bytecode_hash: Some(B256::ZERO) };

        // Write account
        {
            let mut writer = hot_kv.writer().unwrap();
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
        let slot = b256!("0x0000000000000000000000000000000000000000000000000000000000000001");
        let value = U256::from(999);

        // Write storage
        {
            let mut writer = hot_kv.writer().unwrap();
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
            assert_eq!(entry.key, slot);
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
            let mut writer = hot_kv.writer().unwrap();
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
            let mut writer = hot_kv.writer().unwrap();
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
        let slot = b256!("0x0000000000000000000000000000000000000000000000000000000000000042");
        let touched_blocks = BlockNumberList::new([5, 15, 25]).unwrap();
        let highest_block = 50u64;

        // Write storage history
        {
            let mut writer = hot_kv.writer().unwrap();
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
            let mut writer = hot_kv.writer().unwrap();
            writer.write_account_change(block_number, addr, &pre_state).unwrap();
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
        let slot = b256!("0x0000000000000000000000000000000000000000000000000000000000000099");
        let pre_value = U256::from(12345);
        let block_number = 200u64;

        // Write storage change
        {
            let mut writer = hot_kv.writer().unwrap();
            writer.write_storage_change(block_number, addr, &slot, &pre_value).unwrap();
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
        let missing_hash =
            b256!("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
        let missing_slot =
            b256!("0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");

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
}
