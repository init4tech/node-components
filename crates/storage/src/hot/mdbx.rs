use crate::{
    hot::{HotKv, HotKvError, HotKvRead, HotKvReadError, HotKvWrite},
    ser::{DeserError, KeySer, MAX_KEY_SIZE, ValSer},
    tables::{DualKeyed, Table},
};
use bytes::{BufMut, BytesMut};
use reth_db::{
    Database, DatabaseEnv,
    mdbx::{RW, TransactionKind, WriteFlags, tx::Tx},
};
use reth_db_api::DatabaseError;
use reth_libmdbx::RO;
use std::borrow::Cow;

/// Error type for reth-libmdbx based hot storage.
#[derive(Debug, thiserror::Error)]
pub enum MdbxError {
    /// Inner error
    #[error(transparent)]
    Mdbx(#[from] reth_libmdbx::Error),

    /// Reth error.
    #[error(transparent)]
    Reth(#[from] DatabaseError),

    /// Deser.
    #[error(transparent)]
    Deser(#[from] DeserError),
}

impl HotKvReadError for MdbxError {
    fn into_hot_kv_error(self) -> HotKvError {
        match self {
            MdbxError::Mdbx(e) => HotKvError::from_err(e),
            MdbxError::Deser(e) => HotKvError::Deser(e),
            MdbxError::Reth(e) => HotKvError::from_err(e),
        }
    }
}

impl From<DeserError> for DatabaseError {
    fn from(value: DeserError) -> Self {
        DatabaseError::Other(value.to_string())
    }
}

impl HotKv for DatabaseEnv {
    type RoTx = Tx<RO>;
    type RwTx = Tx<RW>;

    fn reader(&self) -> Result<Self::RoTx, HotKvError> {
        self.tx().map_err(HotKvError::from_err)
    }

    fn writer(&self) -> Result<Self::RwTx, HotKvError> {
        self.tx_mut().map_err(HotKvError::from_err)
    }
}

impl<K> HotKvRead for Tx<K>
where
    K: TransactionKind,
{
    type Error = MdbxError;

    fn raw_get<'a>(
        &'a self,
        table: &str,
        key: &[u8],
    ) -> Result<Option<Cow<'a, [u8]>>, Self::Error> {
        let dbi = self.inner.open_db(Some(table)).map(|db| db.dbi())?;

        self.inner.get(dbi, key.as_ref()).map_err(MdbxError::Mdbx)
    }

    fn raw_get_dual<'a>(
        &'a self,
        _table: &str,
        _key1: &[u8],
        _key2: &[u8],
    ) -> Result<Option<Cow<'a, [u8]>>, Self::Error> {
        unimplemented!("Not implemented: raw_get_dual. Use get_dual instead.");
    }

    fn get_dual<T: crate::tables::DualKeyed>(
        &self,
        key1: &T::Key,
        key2: &T::Key2,
    ) -> Result<Option<T::Value>, Self::Error> {
        let mut key1_buf = [0u8; MAX_KEY_SIZE];
        let key1_bytes = key1.encode_key(&mut key1_buf);

        // K2 slice must be EXACTLY the size of the fixed value size, if the
        // table has one. This is a bit ugly, and results in an extra
        // allocation for fixed-size values. This could be avoided using
        // max value size.
        let value_bytes = if let Some(size) = <T as Table>::FIXED_VAL_SIZE {
            let buf = vec![0u8; size];
            let _ = key2.encode_key(&mut buf[..MAX_KEY_SIZE].try_into().unwrap());

            let db = self.inner.open_db(Some(T::NAME))?;
            let mut cursor = self.inner.cursor(&db).map_err(MdbxError::Mdbx)?;
            cursor.get_both_range(key1_bytes, &buf).map_err(MdbxError::Mdbx)
        } else {
            let mut buf = [0u8; MAX_KEY_SIZE];
            let encoded = key2.encode_key(&mut buf);

            let db = self.inner.open_db(Some(T::NAME))?;
            let mut cursor = self.inner.cursor(&db).map_err(MdbxError::Mdbx)?;
            cursor.get_both_range::<Cow<'_, [u8]>>(key1_bytes, encoded).map_err(MdbxError::Mdbx)
        };

        let Some(value_bytes) = value_bytes? else {
            return Ok(None);
        };
        // we need to strip the key2 prefix from the value bytes before decoding
        let value_bytes = &value_bytes[<<T as DualKeyed>::Key2 as KeySer>::SIZE..];

        T::Value::decode_value(value_bytes).map(Some).map_err(Into::into)
    }
}

impl HotKvWrite for Tx<RW> {
    fn queue_raw_put(&mut self, table: &str, key: &[u8], value: &[u8]) -> Result<(), Self::Error> {
        let dbi = self.inner.open_db(Some(table)).map(|db| db.dbi())?;

        self.inner.put(dbi, key, value, WriteFlags::UPSERT).map(|_| ()).map_err(MdbxError::Mdbx)
    }

    fn queue_raw_put_dual(
        &mut self,
        _table: &str,
        _key1: &[u8],
        _key2: &[u8],
        _value: &[u8],
    ) -> Result<(), Self::Error> {
        unimplemented!("Not implemented: queue_raw_put_dual. Use queue_put_dual instead.");
    }

    // Specialized put for dual-keyed tables.
    fn queue_put_dual<T: crate::tables::DualKeyed>(
        &mut self,
        key1: &T::Key,
        key2: &T::Key2,
        value: &T::Value,
    ) -> Result<(), Self::Error> {
        let k2_size = <T::Key2 as KeySer>::SIZE;
        let mut scratch = [0u8; MAX_KEY_SIZE];

        // This will be the total length of key2 + value, reserved in mdbx
        let encoded_len = k2_size + value.encoded_size();

        // Prepend the value with k2.
        let mut buf = BytesMut::with_capacity(encoded_len);
        let encoded_k2 = key2.encode_key(&mut scratch);
        buf.put_slice(encoded_k2);
        value.encode_value_to(&mut buf);

        let encoded_k1 = key1.encode_key(&mut scratch);
        // NB: DUPSORT and RESERVE are incompatible :(
        let db = self.inner.open_db(Some(T::NAME))?;
        self.inner.put(db.dbi(), encoded_k1, &buf, Default::default())?;

        Ok(())
    }

    fn queue_raw_delete(&mut self, table: &str, key: &[u8]) -> Result<(), Self::Error> {
        let dbi = self.inner.open_db(Some(table)).map(|db| db.dbi())?;

        self.inner.del(dbi, key, None).map(|_| ()).map_err(MdbxError::Mdbx)
    }

    fn queue_raw_clear(&mut self, table: &str) -> Result<(), Self::Error> {
        // Future: port more of reth's db env with dbi caching to avoid
        // repeated open_db calls
        let dbi = self.inner.open_db(Some(table)).map(|db| db.dbi())?;
        self.inner.clear_db(dbi).map(|_| ()).map_err(MdbxError::Mdbx)
    }

    fn queue_raw_create(
        &mut self,
        table: &str,
        dual_key: bool,
        fixed_val: bool,
    ) -> Result<(), Self::Error> {
        let mut flags = Default::default();

        if dual_key {
            flags |= reth_libmdbx::DatabaseFlags::DUP_SORT;
            if fixed_val {
                flags |= reth_libmdbx::DatabaseFlags::DUP_FIXED;
            }
        }

        self.inner.create_db(Some(table), flags).map(|_| ()).map_err(MdbxError::Mdbx)
    }

    fn raw_commit(self) -> Result<(), Self::Error> {
        // when committing, mdbx returns true on failure
        self.inner.commit().map(drop).map_err(MdbxError::Mdbx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hot::{HotDbWriter, HotKv, HotKvRead, HotKvWrite},
        tables::hot,
    };
    use alloy::primitives::{Address, B256, BlockNumber, U256};
    use reth::primitives::{Account, Bytecode, Header};
    use reth_db::DatabaseEnv;

    /// A test database wrapper that automatically cleans up on drop
    struct TestDb {
        db: DatabaseEnv,
        #[allow(dead_code)]
        temp_dir: tempfile::TempDir,
    }

    impl std::ops::Deref for TestDb {
        type Target = DatabaseEnv;

        fn deref(&self) -> &Self::Target {
            &self.db
        }
    }

    /// Create a temporary MDBX database for testing that will be automatically cleaned up
    fn create_test_db() -> TestDb {
        let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");

        // Create the database
        let db = reth_db::create_db(&temp_dir, Default::default()).unwrap();

        // Create tables from the `crate::tables::hot` module
        let mut writer = db.writer().unwrap();

        writer.queue_create::<hot::Headers>().unwrap();
        writer.queue_create::<hot::HeaderNumbers>().unwrap();
        writer.queue_create::<hot::CanonicalHeaders>().unwrap();
        writer.queue_create::<hot::Bytecodes>().unwrap();
        writer.queue_create::<hot::PlainAccountState>().unwrap();
        writer.queue_create::<hot::AccountsHistory>().unwrap();
        writer.queue_create::<hot::StorageHistory>().unwrap();
        writer.queue_create::<hot::PlainStorageState>().unwrap();
        writer.queue_create::<hot::StorageChangeSets>().unwrap();
        writer.queue_create::<hot::AccountChangeSets>().unwrap();

        writer.commit().expect("Failed to commit table creation");

        TestDb { db, temp_dir }
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
    fn test_hotkv_basic_operations() {
        let db = create_test_db();
        let (address, account) = create_test_account();
        let (hash, bytecode) = create_test_bytecode();

        // Test HotKv::writer() and basic write operations
        {
            let mut writer: Tx<RW> = db.writer().unwrap();

            // Create tables first
            writer.queue_create::<hot::Bytecodes>().unwrap();

            // Write account data
            writer.queue_put::<hot::PlainAccountState>(&address, &account).unwrap();
            writer.queue_put::<hot::Bytecodes>(&hash, &bytecode).unwrap();

            // Commit the transaction
            writer.raw_commit().unwrap();
        }

        // Test HotKv::reader() and basic read operations
        {
            let reader: Tx<RO> = db.reader().unwrap();

            // Read account data
            let read_account: Option<Account> =
                reader.get::<hot::PlainAccountState>(&address).unwrap();
            assert_eq!(read_account, Some(account));

            // Read bytecode
            let read_bytecode: Option<Bytecode> = reader.get::<hot::Bytecodes>(&hash).unwrap();
            assert_eq!(read_bytecode, Some(bytecode));

            // Test non-existent data
            let nonexistent_addr = Address::from_slice(&[0xff; 20]);
            let nonexistent_account: Option<Account> =
                reader.get::<hot::PlainAccountState>(&nonexistent_addr).unwrap();
            assert_eq!(nonexistent_account, None);
        }
    }

    #[test]
    fn test_raw_operations() {
        let db = create_test_db();

        let table_name = "test_table";
        let key = b"test_key";
        let value = b"test_value";

        // Test raw write operations
        {
            let mut writer: Tx<RW> = db.writer().unwrap();

            // Create table
            writer.queue_raw_create(table_name, false, false).unwrap();

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
    fn test_dual_keyed_operations() {
        let db = create_test_db();

        let address = Address::from_slice(&[0x1; 20]);
        let storage_key = B256::from_slice(&[0x5; 32]);
        let storage_value = U256::from(999u64);

        // Test dual-keyed table operations
        {
            let mut writer: Tx<RW> = db.writer().unwrap();

            // Put storage data using dual keys
            writer
                .queue_put_dual::<hot::PlainStorageState>(&address, &storage_key, &storage_value)
                .unwrap();

            writer.raw_commit().unwrap();
        }

        // Test reading dual-keyed data
        {
            let reader: Tx<RO> = db.reader().unwrap();

            // Read storage using dual key lookup
            let read_value =
                reader.get_dual::<hot::PlainStorageState>(&address, &storage_key).unwrap().unwrap();

            assert_eq!(read_value, storage_value);
        }
    }

    #[test]
    fn test_table_management() {
        let db = create_test_db();

        // Add some data
        let (block_number, header) = create_test_header();
        {
            let mut writer: Tx<RW> = db.writer().unwrap();
            writer.queue_put::<hot::Headers>(&block_number, &header).unwrap();
            writer.raw_commit().unwrap();
        }

        // Verify data exists
        {
            let reader: Tx<RO> = db.reader().unwrap();
            let read_header: Option<Header> = reader.get::<hot::Headers>(&block_number).unwrap();
            assert_eq!(read_header, Some(header.clone()));
        }

        // Clear the table
        {
            let mut writer: Tx<RW> = db.writer().unwrap();
            writer.queue_clear::<hot::Headers>().unwrap();
            writer.raw_commit().unwrap();
        }

        // Verify table is empty
        {
            let reader: Tx<RO> = db.reader().unwrap();
            let read_header: Option<Header> = reader.get::<hot::Headers>(&block_number).unwrap();
            assert_eq!(read_header, None);
        }
    }

    #[test]
    fn test_batch_operations() {
        let db = create_test_db();

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
                writer.queue_put::<hot::PlainAccountState>(address, account).unwrap();
            }

            writer.raw_commit().unwrap();
        }

        // Test batch reads
        {
            let reader: Tx<RO> = db.reader().unwrap();

            for (address, expected_account) in &accounts {
                let read_account: Option<Account> =
                    reader.get::<hot::PlainAccountState>(address).unwrap();
                assert_eq!(read_account.as_ref(), Some(expected_account));
            }
        }

        // Test batch get_many
        {
            let reader: Tx<RO> = db.reader().unwrap();
            let addresses: Vec<Address> = accounts.iter().map(|(addr, _)| *addr).collect();
            let read_accounts: Vec<Option<Account>> =
                reader.get_many::<hot::PlainAccountState, _>(addresses.iter()).unwrap();

            for (i, (_, expected_account)) in accounts.iter().enumerate() {
                assert_eq!(read_accounts[i].as_ref(), Some(expected_account));
            }
        }
    }

    #[test]
    fn test_transaction_isolation() {
        let db = create_test_db();
        let (address, account) = create_test_account();

        // Setup initial data
        {
            let mut writer: Tx<RW> = db.writer().unwrap();
            writer.queue_put::<hot::PlainAccountState>(&address, &account).unwrap();
            writer.raw_commit().unwrap();
        }

        // Start a reader transaction
        let reader: Tx<RO> = db.reader().unwrap();

        // Modify data in a writer transaction
        {
            let mut writer: Tx<RW> = db.writer().unwrap();
            let modified_account =
                Account { nonce: 999, balance: U256::from(9999u64), bytecode_hash: None };
            writer.queue_put::<hot::PlainAccountState>(&address, &modified_account).unwrap();
            writer.raw_commit().unwrap();
        }

        // Reader should still see original data (snapshot isolation)
        {
            let read_account: Option<Account> =
                reader.get::<hot::PlainAccountState>(&address).unwrap();
            assert_eq!(read_account, Some(account));
        }

        // New reader should see modified data
        {
            let new_reader: Tx<RO> = db.reader().unwrap();
            let read_account: Option<Account> =
                new_reader.get::<hot::PlainAccountState>(&address).unwrap();
            assert_eq!(read_account.unwrap().nonce, 999);
        }
    }

    #[test]
    fn test_multiple_readers() {
        let db = create_test_db();
        let (address, account) = create_test_account();

        // Setup data
        {
            let mut writer: Tx<RW> = db.writer().unwrap();
            writer.queue_put::<hot::PlainAccountState>(&address, &account).unwrap();
            writer.raw_commit().unwrap();
        }

        // Create multiple readers
        let reader1: Tx<RO> = db.reader().unwrap();
        let reader2: Tx<RO> = db.reader().unwrap();
        let reader3: Tx<RO> = db.reader().unwrap();

        // All readers should see the same data
        let account1: Option<Account> = reader1.get::<hot::PlainAccountState>(&address).unwrap();
        let account2: Option<Account> = reader2.get::<hot::PlainAccountState>(&address).unwrap();
        let account3: Option<Account> = reader3.get::<hot::PlainAccountState>(&address).unwrap();

        assert_eq!(account1, Some(account));
        assert_eq!(account2, Some(account));
        assert_eq!(account3, Some(account));
    }

    #[test]
    fn test_error_handling() {
        let db = create_test_db();

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
            let result = writer.queue_put::<hot::PlainAccountState>(&address, &account);
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
        let db = create_test_db();

        // Test various data types
        let (block_number, header) = create_test_header();
        let canonical_hash = B256::from_slice(&[0x7; 32]);

        {
            let mut writer: Tx<RW> = db.writer().unwrap();

            // Create tables

            // Write different types
            writer.queue_put::<hot::Headers>(&block_number, &header).unwrap();
            writer.queue_put::<hot::CanonicalHeaders>(&block_number, &canonical_hash).unwrap();

            writer.raw_commit().unwrap();
        }

        {
            let reader: Tx<RO> = db.reader().unwrap();

            // Read and verify
            let read_header: Option<Header> = reader.get::<hot::Headers>(&block_number).unwrap();
            assert_eq!(read_header, Some(header));

            let read_hash: Option<B256> =
                reader.get::<hot::CanonicalHeaders>(&block_number).unwrap();
            assert_eq!(read_hash, Some(canonical_hash));
        }
    }

    #[test]
    fn test_large_data() {
        let db = create_test_db();

        // Create a large bytecode
        let hash = B256::from_slice(&[0x8; 32]);
        let large_code_vec: Vec<u8> = (0..10000).map(|i| (i % 256) as u8).collect();
        let large_bytecode = Bytecode::new_raw(large_code_vec.clone().into());

        {
            let mut writer: Tx<RW> = db.writer().unwrap();
            writer.queue_create::<hot::Bytecodes>().unwrap();
            writer.queue_put::<hot::Bytecodes>(&hash, &large_bytecode).unwrap();
            writer.raw_commit().unwrap();
        }

        {
            let reader: Tx<RO> = db.reader().unwrap();
            let read_bytecode: Option<Bytecode> = reader.get::<hot::Bytecodes>(&hash).unwrap();
            assert_eq!(read_bytecode, Some(large_bytecode));
        }
    }
}
