use crate::{
    hot::{HotKvError, HotKvRead, HotKvWrite},
    tables::hot::{AccountStorageKey, Bytecodes, PlainAccountState},
};
use alloy::primitives::{Address, B256, KECCAK256_EMPTY};
use core::fmt;
use reth::primitives::Account;
use trevm::revm::{
    database::{DBErrorMarker, Database, DatabaseRef, TryDatabaseCommit},
    primitives::{HashMap, StorageKey, StorageValue},
    state::{self, AccountInfo, Bytecode as RevmBytecode},
};

// Error marker implementation
impl DBErrorMarker for HotKvError {}

/// Read-only [`Database`] and [`DatabaseRef`] adapter.
pub struct RevmRead<T: HotKvRead> {
    reader: T,
}

impl<T: HotKvRead> RevmRead<T> {
    /// Create a new read adapter
    pub const fn new(reader: T) -> Self {
        Self { reader }
    }
}

impl<T: HotKvRead> fmt::Debug for RevmRead<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RevmRead").finish()
    }
}

// HotKvRead implementation for RevmRead
impl<T: HotKvRead> HotKvRead for RevmRead<T> {
    type Error = T::Error;

    fn get_raw<'a>(
        &'a self,
        table: &str,
        key: &[u8],
    ) -> Result<Option<std::borrow::Cow<'a, [u8]>>, Self::Error> {
        self.reader.get_raw(table, key)
    }
}

/// Read-write REVM database adapter. This adapter allows committing changes.
/// Despite the naming of [`TryDatabaseCommit::try_commit`], the changes are
/// only persisted when [`Self::persist`] is called. This is because of a
/// mismatch in semantics between the two systems.
pub struct RevmWrite<T: HotKvWrite> {
    writer: T,
}
impl<T: HotKvWrite> RevmWrite<T> {
    /// Create a new write adapter
    pub const fn new(writer: T) -> Self {
        Self { writer }
    }

    /// Persist the changes made in this write transaction.
    pub fn persist(self) -> Result<(), T::Error> {
        self.writer.raw_commit()
    }
}

impl<T: HotKvWrite> fmt::Debug for RevmWrite<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RevmWrite").finish()
    }
}

// HotKvWrite implementation for RevmWrite
impl<T: HotKvWrite> HotKvRead for RevmWrite<T> {
    type Error = T::Error;

    fn get_raw<'a>(
        &'a self,
        table: &str,
        key: &[u8],
    ) -> Result<Option<std::borrow::Cow<'a, [u8]>>, Self::Error> {
        self.writer.get_raw(table, key)
    }
}

impl<T: HotKvWrite> HotKvWrite for RevmWrite<T> {
    fn queue_raw_put(&mut self, table: &str, key: &[u8], value: &[u8]) -> Result<(), Self::Error> {
        self.writer.queue_raw_put(table, key, value)
    }

    fn queue_raw_delete(&mut self, table: &str, key: &[u8]) -> Result<(), Self::Error> {
        self.writer.queue_raw_delete(table, key)
    }

    fn queue_raw_clear(&mut self, table: &str) -> Result<(), Self::Error> {
        self.writer.queue_raw_clear(table)
    }

    fn queue_raw_create(&mut self, table: &str) -> Result<(), Self::Error> {
        self.writer.queue_raw_create(table)
    }

    fn raw_commit(self) -> Result<(), Self::Error> {
        self.writer.raw_commit()
    }
}

// DatabaseRef implementation for RevmRead
impl<T: HotKvRead> DatabaseRef for RevmRead<T>
where
    T::Error: DBErrorMarker,
{
    type Error = T::Error;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let account_opt = self.reader.get::<PlainAccountState>(&address)?;

        let Some(account) = account_opt else {
            return Ok(None);
        };

        let code_hash = account.bytecode_hash.unwrap_or(KECCAK256_EMPTY);
        let code = if code_hash != KECCAK256_EMPTY {
            self.reader.get::<Bytecodes>(&code_hash)?.map(|b| b.0)
        } else {
            None
        };

        Ok(Some(AccountInfo { balance: account.balance, nonce: account.nonce, code_hash, code }))
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<RevmBytecode, Self::Error> {
        Ok(self.reader.get::<Bytecodes>(&code_hash)?.map(|bytecode| bytecode.0).unwrap_or_default())
    }

    fn storage_ref(
        &self,
        address: Address,
        index: StorageKey,
    ) -> Result<StorageValue, Self::Error> {
        let storage_key = AccountStorageKey {
            address: std::borrow::Cow::Borrowed(&address),
            key: std::borrow::Cow::Owned(B256::from_slice(&index.to_be_bytes::<32>())),
        }
        .encode_key();

        Ok(self
            .reader
            .get::<crate::tables::hot::PlainStorageState>(&storage_key)?
            .unwrap_or_default())
    }

    fn block_hash_ref(&self, _number: u64) -> Result<B256, Self::Error> {
        // This would need to be implemented based on your block hash storage
        // For now, return zero hash
        Ok(B256::ZERO)
    }
}

// Database implementation for RevmRead
impl<T: HotKvRead> Database for RevmRead<T>
where
    T::Error: DBErrorMarker,
{
    type Error = T::Error;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.basic_ref(address)
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<RevmBytecode, Self::Error> {
        self.code_by_hash_ref(code_hash)
    }

    fn storage(
        &mut self,
        address: Address,
        index: StorageKey,
    ) -> Result<StorageValue, Self::Error> {
        self.storage_ref(address, index)
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        self.block_hash_ref(number)
    }
}

// DatabaseRef implementation for RevmWrite (delegates to read operations)
impl<T: HotKvWrite> DatabaseRef for RevmWrite<T>
where
    T::Error: DBErrorMarker,
{
    type Error = T::Error;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let account_opt = self.writer.get::<PlainAccountState>(&address)?;

        let Some(account) = account_opt else {
            return Ok(None);
        };

        let code_hash = account.bytecode_hash.unwrap_or(KECCAK256_EMPTY);
        let code = if code_hash != KECCAK256_EMPTY {
            self.writer.get::<Bytecodes>(&code_hash)?.map(|b| b.0)
        } else {
            None
        };

        Ok(Some(AccountInfo { balance: account.balance, nonce: account.nonce, code_hash, code }))
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<RevmBytecode, Self::Error> {
        Ok(self.writer.get::<Bytecodes>(&code_hash)?.map(|bytecode| bytecode.0).unwrap_or_default())
    }

    fn storage_ref(
        &self,
        address: Address,
        index: StorageKey,
    ) -> Result<StorageValue, Self::Error> {
        let storage_key = AccountStorageKey {
            address: std::borrow::Cow::Borrowed(&address),
            key: std::borrow::Cow::Owned(B256::from_slice(&index.to_be_bytes::<32>())),
        }
        .encode_key();

        Ok(self
            .writer
            .get::<crate::tables::hot::PlainStorageState>(&storage_key)?
            .unwrap_or_default())
    }

    fn block_hash_ref(&self, _number: u64) -> Result<B256, Self::Error> {
        // This would need to be implemented based on your block hash storage
        // For now, return zero hash
        Ok(B256::ZERO)
    }
}

// Database implementation for RevmWrite
impl<T: HotKvWrite> Database for RevmWrite<T>
where
    T::Error: DBErrorMarker,
{
    type Error = T::Error;

    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.basic_ref(address)
    }

    fn code_by_hash(&mut self, code_hash: B256) -> Result<RevmBytecode, Self::Error> {
        self.code_by_hash_ref(code_hash)
    }

    fn storage(
        &mut self,
        address: Address,
        index: StorageKey,
    ) -> Result<StorageValue, Self::Error> {
        self.storage_ref(address, index)
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        self.block_hash_ref(number)
    }
}

// TryDatabaseCommit implementation for RevmWrite
impl<T: HotKvWrite> TryDatabaseCommit for RevmWrite<T>
where
    T::Error: DBErrorMarker,
{
    type Error = T::Error;

    fn try_commit(&mut self, changes: HashMap<Address, state::Account>) -> Result<(), Self::Error> {
        for (address, account) in changes {
            // Handle account info changes
            let account_data = Account {
                nonce: account.info.nonce,
                balance: account.info.balance,
                bytecode_hash: (account.info.code_hash != KECCAK256_EMPTY)
                    .then_some(account.info.code_hash),
            };
            self.writer.queue_put::<PlainAccountState>(&address, &account_data)?;

            // Handle storage changes
            for (key, value) in account.storage {
                let storage_key = AccountStorageKey {
                    address: std::borrow::Cow::Borrowed(&address),
                    key: std::borrow::Cow::Owned(B256::from_slice(&key.to_be_bytes::<32>())),
                }
                .encode_key();
                self.writer.queue_put::<crate::tables::hot::PlainStorageState>(
                    &storage_key,
                    &value.present_value(),
                )?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        hot::{HotKv, HotKvRead, HotKvWrite, MemKv},
        tables::hot::{Bytecodes, PlainAccountState, PlainStorageState},
    };
    use alloy::primitives::{Address, B256, U256};
    use reth::primitives::{Account, Bytecode};
    use trevm::revm::{
        database::{Database, DatabaseRef, TryDatabaseCommit},
        primitives::{HashMap, StorageKey, StorageValue},
        state::{Account as RevmAccount, AccountInfo, Bytecode as RevmBytecode},
    };

    /// Create a test account with some data
    fn create_test_account() -> (Address, Account) {
        let address = Address::from_slice(&[0x1; 20]);
        let account = Account {
            nonce: 42,
            balance: U256::from(1000u64),
            bytecode_hash: Some(B256::from_slice(&[0x2; 32])),
        };
        (address, account)
    }

    /// Create test bytecode
    fn create_test_bytecode() -> (B256, Bytecode) {
        let hash = B256::from_slice(&[0x2; 32]);
        let code = RevmBytecode::new_raw(vec![0x60, 0x80, 0x60, 0x40].into());
        let bytecode = Bytecode(code);
        (hash, bytecode)
    }

    #[test]
    fn test_database_ref_traits() -> Result<(), Box<dyn std::error::Error>> {
        let mem_kv = MemKv::default();

        let (address, account) = create_test_account();
        let (hash, bytecode) = create_test_bytecode();

        {
            // Setup data using HotKv
            let mut writer = mem_kv.revm_writer()?;
            writer.queue_put::<PlainAccountState>(&address, &account)?;
            writer.queue_put::<Bytecodes>(&hash, &bytecode)?;
            writer.persist()?;
        }

        {
            // Read using REVM DatabaseRef traits
            let reader = mem_kv.revm_reader()?;

            // Test basic_ref
            let account_info = reader.basic_ref(address)?;
            assert!(account_info.is_some());
            let info = account_info.unwrap();
            assert_eq!(info.nonce, 42);
            assert_eq!(info.balance, U256::from(1000u64));
            assert_eq!(info.code_hash, hash);

            // Test code_by_hash_ref
            let retrieved_code = reader.code_by_hash_ref(hash)?;
            assert_eq!(retrieved_code, bytecode.0);

            // Test storage_ref (should be zero for non-existent storage)
            let storage_val = reader.storage_ref(address, StorageKey::from(U256::from(123u64)))?;
            assert_eq!(storage_val, U256::ZERO);

            // Test block_hash_ref
            let block_hash = reader.block_hash_ref(123)?;
            assert_eq!(block_hash, B256::ZERO);
        }

        Ok(())
    }

    #[test]
    fn test_database_mutable_traits() -> Result<(), Box<dyn std::error::Error>> {
        let mem_kv = MemKv::default();

        let (address, account) = create_test_account();
        let (hash, bytecode) = create_test_bytecode();

        {
            // Setup data using HotKv
            let mut writer = mem_kv.revm_writer()?;
            writer.queue_put::<PlainAccountState>(&address, &account)?;
            writer.queue_put::<Bytecodes>(&hash, &bytecode)?;
            writer.persist()?;
        }

        {
            // Read using mutable REVM Database traits
            let mut reader = mem_kv.revm_reader()?;

            // Test basic
            let account_info = reader.basic(address)?;
            assert!(account_info.is_some());
            let info = account_info.unwrap();
            assert_eq!(info.nonce, 42);
            assert_eq!(info.balance, U256::from(1000u64));

            // Test code_by_hash
            let retrieved_code = reader.code_by_hash(hash)?;
            assert_eq!(retrieved_code, bytecode.0);

            // Test storage
            let storage_val = reader.storage(address, StorageKey::from(U256::from(123u64)))?;
            assert_eq!(storage_val, U256::ZERO);

            // Test block_hash
            let block_hash = reader.block_hash(123)?;
            assert_eq!(block_hash, B256::ZERO);
        }

        Ok(())
    }

    #[test]
    fn test_write_database_traits() -> Result<(), Box<dyn std::error::Error>> {
        let mem_kv = MemKv::default();

        let (address, account) = create_test_account();
        let (hash, bytecode) = create_test_bytecode();

        {
            // Setup initial data
            let mut writer = mem_kv.revm_writer()?;
            writer.queue_put::<PlainAccountState>(&address, &account)?;
            writer.queue_put::<Bytecodes>(&hash, &bytecode)?;
            writer.persist()?;
        }

        {
            // Test write operations using DatabaseRef and Database traits
            let mut writer = mem_kv.revm_writer()?;

            // Test read operations work on writer
            let account_info = writer.basic_ref(address)?;
            assert!(account_info.is_some());

            let account_info_mut = writer.basic(address)?;
            assert!(account_info_mut.is_some());

            let code = writer.code_by_hash_ref(hash)?;
            assert_eq!(code, bytecode.0);

            let code_mut = writer.code_by_hash(hash)?;
            assert_eq!(code_mut, bytecode.0);

            // Don't persist this writer to test that reads work
        }

        Ok(())
    }

    #[test]
    fn test_try_database_commit() -> Result<(), Box<dyn std::error::Error>> {
        let mem_kv = MemKv::default();

        let address = Address::from_slice(&[0x1; 20]);

        {
            let mut writer = mem_kv.revm_writer()?;

            // Create REVM state changes
            let mut changes = HashMap::default();
            let account_info = AccountInfo {
                nonce: 55,
                balance: U256::from(2000u64),
                code_hash: KECCAK256_EMPTY,
                code: None,
            };

            let mut storage = HashMap::default();
            storage.insert(
                StorageKey::from(U256::from(100u64)),
                trevm::revm::state::EvmStorageSlot::new(U256::from(200u64), 0),
            );

            let revm_account = RevmAccount {
                info: account_info,
                storage,
                status: trevm::revm::state::AccountStatus::Touched,
                transaction_id: 0,
            };

            changes.insert(address, revm_account);

            // Commit changes using REVM trait
            writer.try_commit(changes)?;
            writer.persist()?;
        }

        {
            // Verify changes were persisted using HotKv traits
            let reader = mem_kv.revm_reader()?;

            let account: Option<Account> = reader.get::<PlainAccountState>(&address)?;
            assert!(account.is_some());
            let acc = account.unwrap();
            assert_eq!(acc.nonce, 55);
            assert_eq!(acc.balance, U256::from(2000u64));
            assert_eq!(acc.bytecode_hash, None);

            // Check storage was written
            let storage_key = AccountStorageKey {
                address: std::borrow::Cow::Borrowed(&address),
                key: std::borrow::Cow::Owned(B256::from_slice(
                    &U256::from(100u64).to_be_bytes::<32>(),
                )),
            }
            .encode_key();

            let storage_val: Option<StorageValue> =
                reader.get::<PlainStorageState>(&storage_key)?;
            assert_eq!(storage_val, Some(U256::from(200u64)));
        }

        Ok(())
    }

    #[test]
    fn test_mixed_usage_patterns() -> Result<(), Box<dyn std::error::Error>> {
        let mem_kv = MemKv::default();

        let address1 = Address::from_slice(&[0x1; 20]);
        let address2 = Address::from_slice(&[0x2; 20]);

        // Write some data using HotKv
        {
            let mut writer = mem_kv.revm_writer()?;
            let account = Account { nonce: 10, balance: U256::from(500u64), bytecode_hash: None };
            writer.queue_put::<PlainAccountState>(&address1, &account)?;
            writer.persist()?;
        }

        // Write more data using REVM traits
        {
            let mut writer = mem_kv.revm_writer()?;
            let mut changes = HashMap::default();
            let revm_account = RevmAccount {
                info: AccountInfo {
                    nonce: 20,
                    balance: U256::from(1500u64),
                    code_hash: KECCAK256_EMPTY,
                    code: None,
                },
                storage: HashMap::default(),
                status: trevm::revm::state::AccountStatus::Touched,
                transaction_id: 0,
            };
            changes.insert(address2, revm_account);
            writer.try_commit(changes)?;
            writer.persist()?;
        }

        // Read using mixed approaches
        {
            let reader = mem_kv.revm_reader()?;

            // Read address1 using HotKv
            let account1: Option<Account> = reader.get::<PlainAccountState>(&address1)?;
            assert!(account1.is_some());
            assert_eq!(account1.unwrap().nonce, 10);

            // Read address2 using REVM DatabaseRef
            let account2_info = reader.basic_ref(address2)?;
            assert!(account2_info.is_some());
            assert_eq!(account2_info.unwrap().nonce, 20);
        }

        Ok(())
    }

    #[test]
    fn test_error_handling() -> Result<(), Box<dyn std::error::Error>> {
        let mem_kv = MemKv::default();

        let address = Address::from_slice(&[0x1; 20]);
        let hash = B256::from_slice(&[0x99; 32]);

        let reader = mem_kv.revm_reader()?;

        // Test reading non-existent account
        let account_info = reader.basic_ref(address)?;
        assert!(account_info.is_none());

        // Test reading non-existent code
        let code = reader.code_by_hash_ref(hash)?;
        assert!(code.is_empty());

        // Test reading non-existent storage
        let storage = reader.storage_ref(address, StorageKey::from(U256::from(123u64)))?;
        assert_eq!(storage, U256::ZERO);

        Ok(())
    }

    #[test]
    fn test_concurrent_readers() -> Result<(), Box<dyn std::error::Error>> {
        let mem_kv = MemKv::default();

        let (address, account) = create_test_account();

        // Setup data
        {
            let mut writer = mem_kv.revm_writer()?;
            writer.queue_put::<PlainAccountState>(&address, &account)?;
            writer.persist()?;
        }

        // Create multiple readers
        let reader1 = mem_kv.revm_reader()?;
        let reader2 = mem_kv.revm_reader()?;

        // Both should read the same data
        let account1 = reader1.basic_ref(address)?;
        let account2 = reader2.basic_ref(address)?;

        assert_eq!(account1, account2);
        assert!(account1.is_some());

        Ok(())
    }
}
