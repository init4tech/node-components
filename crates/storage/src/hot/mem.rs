use crate::{
    hot::{HotKv, HotKvError, HotKvRead, HotKvWrite},
    ser::MAX_KEY_SIZE,
};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

type Table = BTreeMap<[u8; MAX_KEY_SIZE], bytes::Bytes>;
type Store = BTreeMap<String, Table>;

type TableOp = BTreeMap<[u8; MAX_KEY_SIZE], QueuedKvOp>;
type OpStore = BTreeMap<String, QueuedTableOp>;

/// A simple in-memory key-value store using a BTreeMap.
///
/// This implementation supports concurrent multiple concurrent read
/// transactions. Write transactions are exclusive, and cannot overlap
/// with other read or write transactions.
#[derive(Clone)]
pub struct MemKv {
    map: Arc<RwLock<Store>>,
}

impl core::fmt::Debug for MemKv {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MemKv").finish()
    }
}

impl MemKv {
    /// Create a new empty in-memory KV store.
    pub fn new() -> Self {
        Self { map: Arc::new(RwLock::new(BTreeMap::new())) }
    }

    #[track_caller]
    fn key(k: &[u8]) -> [u8; MAX_KEY_SIZE] {
        assert!(k.len() <= MAX_KEY_SIZE, "Key length exceeds MAX_KEY_SIZE");
        let mut buf = [0u8; MAX_KEY_SIZE];
        buf[..k.len()].copy_from_slice(k);
        buf
    }
}

impl Default for MemKv {
    fn default() -> Self {
        Self::new()
    }
}

/// Read-only transaction for MemKv.
pub struct MemKvRoTx {
    guard: RwLockReadGuard<'static, Store>,

    // Keep the store alive while the transaction exists
    _store: Arc<RwLock<Store>>,
}

impl core::fmt::Debug for MemKvRoTx {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MemKvRoTx").finish()
    }
}

// SAFETY: MemKvRoTx holds a read guard which ensures the data remains valid
unsafe impl Send for MemKvRoTx {}
unsafe impl Sync for MemKvRoTx {}

/// Read-write transaction for MemKv.
pub struct MemKvRwTx {
    guard: RwLockWriteGuard<'static, Store>,
    queued_ops: OpStore,

    // Keep the store alive while the transaction exists
    _store: Arc<RwLock<Store>>,
}

impl MemKvRwTx {
    fn commit_inner(&mut self) {
        let ops = std::mem::take(&mut self.queued_ops);

        for (table, table_op) in ops.into_iter() {
            table_op.apply(&table, &mut self.guard);
        }
    }

    /// Downgrade the transaction to a read-only transaction without
    /// committing, discarding queued changes.
    pub fn downgrade(self) -> MemKvRoTx {
        let guard = RwLockWriteGuard::downgrade(self.guard);

        MemKvRoTx { guard, _store: self._store }
    }

    /// Commit the transaction and downgrade to a read-only transaction.
    pub fn commit_downgrade(mut self) -> MemKvRoTx {
        self.commit_inner();

        let guard = RwLockWriteGuard::downgrade(self.guard);

        MemKvRoTx { guard, _store: self._store }
    }
}

impl core::fmt::Debug for MemKvRwTx {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MemKvRwTx").finish()
    }
}

/// Queued key-value operation
#[derive(Debug, Clone)]
enum QueuedKvOp {
    Delete,
    Put { value: bytes::Bytes },
}

impl QueuedKvOp {
    /// Apply the op to a table
    fn apply(self, table: &mut Table, key: [u8; MAX_KEY_SIZE]) {
        match self {
            QueuedKvOp::Put { value } => {
                table.insert(key, value);
            }
            QueuedKvOp::Delete => {
                table.remove(&key);
            }
        }
    }
}

/// Queued table operation
#[derive(Debug)]
enum QueuedTableOp {
    Modify { ops: TableOp },
    Clear { new_table: TableOp },
}

impl Default for QueuedTableOp {
    fn default() -> Self {
        QueuedTableOp::Modify { ops: TableOp::new() }
    }
}

impl QueuedTableOp {
    const fn is_clear(&self) -> bool {
        matches!(self, QueuedTableOp::Clear { .. })
    }

    fn get(&self, key: &[u8; MAX_KEY_SIZE]) -> Option<&QueuedKvOp> {
        match self {
            QueuedTableOp::Modify { ops } => ops.get(key),
            QueuedTableOp::Clear { new_table } => new_table.get(key),
        }
    }

    fn put(&mut self, key: [u8; MAX_KEY_SIZE], op: QueuedKvOp) {
        match self {
            QueuedTableOp::Modify { ops } | QueuedTableOp::Clear { new_table: ops } => {
                ops.insert(key, op);
            }
        }
    }

    fn delete(&mut self, key: [u8; MAX_KEY_SIZE]) {
        match self {
            QueuedTableOp::Modify { ops } | QueuedTableOp::Clear { new_table: ops } => {
                ops.insert(key, QueuedKvOp::Delete);
            }
        }
    }

    /// Get mutable reference to the inner ops if applicable
    fn apply(self, key: &str, store: &mut Store) {
        match self {
            QueuedTableOp::Modify { ops } => {
                let table = store.entry(key.to_owned()).or_default();
                for (key, op) in ops {
                    op.apply(table, key);
                }
            }
            QueuedTableOp::Clear { new_table } => {
                let mut table = Table::new();
                for (k, op) in new_table {
                    op.apply(&mut table, k);
                }

                // replace the table entirely
                store.insert(key.to_owned(), table);
            }
        }
    }
}

// SAFETY: MemKvRwTx holds a write guard which ensures exclusive access
unsafe impl Send for MemKvRwTx {}

impl HotKv for MemKv {
    type RoTx = MemKvRoTx;
    type RwTx = MemKvRwTx;

    fn reader(&self) -> Result<Self::RoTx, HotKvError> {
        let guard = self
            .map
            .try_read()
            .map_err(|_| HotKvError::Inner("Failed to acquire read lock".into()))?;

        // SAFETY: This is safe-ish, as we ensure the map is not dropped until
        // the guard is also dropped.
        let guard: RwLockReadGuard<'static, Store> = unsafe { std::mem::transmute(guard) };

        Ok(MemKvRoTx { guard, _store: self.map.clone() })
    }

    fn writer(&self) -> Result<Self::RwTx, HotKvError> {
        let guard = self.map.try_write().map_err(|_| HotKvError::WriteLocked)?;

        // SAFETY: This is safe-ish, as we ensure the map is not dropped until
        // the guard is also dropped.
        let guard: RwLockWriteGuard<'static, Store> = unsafe { std::mem::transmute(guard) };

        Ok(MemKvRwTx { guard, _store: self.map.clone(), queued_ops: OpStore::new() })
    }
}

impl HotKvRead for MemKvRoTx {
    type Error = HotKvError;

    fn get_raw<'a>(
        &'a self,
        table: &str,
        key: &[u8],
    ) -> Result<Option<Cow<'a, [u8]>>, Self::Error> {
        // Check queued operations first (read-your-writes consistency)
        let key = MemKv::key(key);

        // SAFETY: The guard ensures the map remains valid

        Ok(self
            .guard
            .get(table)
            .and_then(|t| t.get(&key))
            .map(|bytes| Cow::Borrowed(bytes.as_ref())))
    }
}

impl HotKvRead for MemKvRwTx {
    type Error = HotKvError;

    fn get_raw<'a>(
        &'a self,
        table: &str,
        key: &[u8],
    ) -> Result<Option<Cow<'a, [u8]>>, Self::Error> {
        // Check queued operations first (read-your-writes consistency)
        let key = MemKv::key(key);

        if let Some(table) = self.queued_ops.get(table) {
            if table.is_clear() {
                return Ok(None);
            }

            match table.get(&key) {
                Some(QueuedKvOp::Put { value }) => {
                    return Ok(Some(Cow::Borrowed(value.as_ref())));
                }
                Some(QueuedKvOp::Delete) => {
                    return Ok(None);
                }
                None => {}
            }
        }

        // If not found in queued ops, check the underlying map
        Ok(self
            .guard
            .get(table)
            .and_then(|t| t.get(&key))
            .map(|bytes| Cow::Borrowed(bytes.as_ref())))
    }
}

impl HotKvWrite for MemKvRwTx {
    fn queue_raw_put(&mut self, table: &str, key: &[u8], value: &[u8]) -> Result<(), Self::Error> {
        let key = MemKv::key(key);

        let value_bytes = bytes::Bytes::copy_from_slice(value);

        self.queued_ops
            .entry(table.to_owned())
            .or_default()
            .put(key, QueuedKvOp::Put { value: value_bytes });
        Ok(())
    }

    fn queue_raw_delete(&mut self, table: &str, key: &[u8]) -> Result<(), Self::Error> {
        let key = MemKv::key(key);

        self.queued_ops.entry(table.to_owned()).or_default().delete(key);
        Ok(())
    }

    fn queue_raw_clear(&mut self, table: &str) -> Result<(), Self::Error> {
        self.queued_ops
            .insert(table.to_owned(), QueuedTableOp::Clear { new_table: TableOp::new() });
        Ok(())
    }

    fn queue_raw_create(&mut self, _table: &str) -> Result<(), Self::Error> {
        Ok(())
    }

    fn raw_commit(mut self) -> Result<(), Self::Error> {
        // Apply all queued operations to the map
        self.commit_inner();

        // The write guard is automatically dropped here, releasing the lock
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::Table;
    use alloy::primitives::{Address, U256};
    use bytes::Bytes;

    // Test table definitions
    #[derive(Debug)]
    struct TestTable;

    impl Table for TestTable {
        const NAME: &'static str = "test_table";

        type Key = u64;
        type Value = Bytes;
    }

    #[derive(Debug)]
    struct AddressTable;

    impl Table for AddressTable {
        const NAME: &'static str = "addresses";
        type Key = Address;
        type Value = U256;
    }

    #[test]
    fn test_new_store() {
        let store = MemKv::new();
        let reader = store.reader().unwrap();

        // Empty store should return None for any key
        assert!(reader.get_raw("test", &[1, 2, 3]).unwrap().is_none());
    }

    #[test]
    fn test_basic_put_get() {
        let store = MemKv::new();

        // Write some data
        {
            let mut writer = store.writer().unwrap();
            writer.queue_raw_put("table1", &[1, 2, 3], b"value1").unwrap();
            writer.queue_raw_put("table1", &[4, 5, 6], b"value2").unwrap();
            writer.raw_commit().unwrap();
        }

        // Read the data back
        {
            let reader = store.reader().unwrap();
            let value1 = reader.get_raw("table1", &[1, 2, 3]).unwrap();
            let value2 = reader.get_raw("table1", &[4, 5, 6]).unwrap();
            let missing = reader.get_raw("table1", &[7, 8, 9]).unwrap();

            assert_eq!(value1.as_deref(), Some(b"value1" as &[u8]));
            assert_eq!(value2.as_deref(), Some(b"value2" as &[u8]));
            assert!(missing.is_none());
        }
    }

    #[test]
    fn test_multiple_tables() {
        let store = MemKv::new();

        // Write to different tables
        {
            let mut writer = store.writer().unwrap();
            writer.queue_raw_put("table1", &[1], b"table1_value").unwrap();
            writer.queue_raw_put("table2", &[1], b"table2_value").unwrap();
            writer.raw_commit().unwrap();
        }

        // Read from different tables
        {
            let reader = store.reader().unwrap();
            let value1 = reader.get_raw("table1", &[1]).unwrap();
            let value2 = reader.get_raw("table2", &[1]).unwrap();

            assert_eq!(value1.as_deref(), Some(b"table1_value" as &[u8]));
            assert_eq!(value2.as_deref(), Some(b"table2_value" as &[u8]));
        }
    }

    #[test]
    fn test_overwrite_value() {
        let store = MemKv::new();

        // Write initial value
        {
            let mut writer = store.writer().unwrap();
            writer.queue_raw_put("table1", &[1], b"original").unwrap();
            writer.raw_commit().unwrap();
        }

        // Overwrite with new value
        {
            let mut writer = store.writer().unwrap();
            writer.queue_raw_put("table1", &[1], b"updated").unwrap();
            writer.raw_commit().unwrap();
        }

        // Check the value was updated
        {
            let reader = store.reader().unwrap();
            let value = reader.get_raw("table1", &[1]).unwrap();
            assert_eq!(value.as_deref(), Some(b"updated" as &[u8]));
        }
    }

    #[test]
    fn test_read_your_writes() {
        let store = MemKv::new();
        let mut writer = store.writer().unwrap();

        // Queue some operations but don't commit yet
        writer.queue_raw_put("table1", &[1], b"queued_value").unwrap();

        // Should be able to read the queued value
        let value = writer.get_raw("table1", &[1]).unwrap();
        assert_eq!(value.as_deref(), Some(b"queued_value" as &[u8]));

        writer.raw_commit().unwrap();

        // After commit, other readers should see it
        {
            let reader = store.reader().unwrap();
            let value = reader.get_raw("table1", &[1]).unwrap();
            assert_eq!(value.as_deref(), Some(b"queued_value" as &[u8]));
        }
    }

    #[test]
    fn test_typed_operations() {
        let store = MemKv::new();

        // Write using typed interface
        {
            let mut writer = store.writer().unwrap();
            writer.queue_put::<TestTable>(&42u64, &Bytes::from_static(b"hello world")).unwrap();
            writer.queue_put::<TestTable>(&100u64, &Bytes::from_static(b"another value")).unwrap();
            writer.raw_commit().unwrap();
        }

        // Read using typed interface
        {
            let reader = store.reader().unwrap();
            let value1 = reader.get::<TestTable>(&42u64).unwrap();
            let value2 = reader.get::<TestTable>(&100u64).unwrap();
            let missing = reader.get::<TestTable>(&999u64).unwrap();

            assert_eq!(value1, Some(Bytes::from_static(b"hello world")));
            assert_eq!(value2, Some(Bytes::from_static(b"another value")));
            assert!(missing.is_none());
        }
    }

    #[test]
    fn test_address_table() {
        let store = MemKv::new();

        let addr1 = Address::from([0x11; 20]);
        let addr2 = Address::from([0x22; 20]);
        let balance1 = U256::from(1000u64);
        let balance2 = U256::from(2000u64);

        // Write address data
        {
            let mut writer = store.writer().unwrap();
            writer.queue_put::<AddressTable>(&addr1, &balance1).unwrap();
            writer.queue_put::<AddressTable>(&addr2, &balance2).unwrap();
            writer.raw_commit().unwrap();
        }

        // Read address data
        {
            let reader = store.reader().unwrap();
            let bal1 = reader.get::<AddressTable>(&addr1).unwrap();
            let bal2 = reader.get::<AddressTable>(&addr2).unwrap();

            assert_eq!(bal1, Some(balance1));
            assert_eq!(bal2, Some(balance2));
        }
    }

    #[test]
    fn test_batch_operations() {
        let store = MemKv::new();

        let entries = [
            (1u64, Bytes::from_static(b"first")),
            (2u64, Bytes::from_static(b"second")),
            (3u64, Bytes::from_static(b"third")),
        ];

        // Write batch
        {
            let mut writer = store.writer().unwrap();
            let entry_refs: Vec<_> = entries.iter().map(|(k, v)| (k, v)).collect();
            writer.queue_put_many::<TestTable, _>(entry_refs).unwrap();
            writer.raw_commit().unwrap();
        }

        // Read batch
        {
            let reader = store.reader().unwrap();
            let keys: Vec<_> = entries.iter().map(|(k, _)| k).collect();
            let values = reader.get_many::<TestTable, _>(keys).unwrap();

            assert_eq!(values.len(), 3);
            assert_eq!(values[0], Some(Bytes::from_static(b"first")));
            assert_eq!(values[1], Some(Bytes::from_static(b"second")));
            assert_eq!(values[2], Some(Bytes::from_static(b"third")));
        }
    }

    #[test]
    fn test_concurrent_readers() {
        let store = MemKv::new();

        // Write some initial data
        {
            let mut writer = store.writer().unwrap();
            writer.queue_raw_put("table1", &[1], b"value1").unwrap();
            writer.raw_commit().unwrap();
        }

        // Multiple readers should be able to read concurrently
        let reader1 = store.reader().unwrap();
        let reader2 = store.reader().unwrap();

        let value1 = reader1.get_raw("table1", &[1]).unwrap();
        let value2 = reader2.get_raw("table1", &[1]).unwrap();

        assert_eq!(value1.as_deref(), Some(b"value1" as &[u8]));
        assert_eq!(value2.as_deref(), Some(b"value1" as &[u8]));
    }

    #[test]
    fn test_write_lock_exclusivity() {
        let store = MemKv::new();

        // Get a writer
        let _writer1 = store.writer().unwrap();

        // Second writer should fail
        match store.writer() {
            Err(HotKvError::WriteLocked) => {} // Expected
            Ok(_) => panic!("Should not be able to get second writer"),
            Err(e) => panic!("Unexpected error: {:?}", e),
        }
    }

    #[test]
    fn test_empty_values() {
        let store = MemKv::new();

        {
            let mut writer = store.writer().unwrap();
            writer.queue_raw_put("table1", &[1], b"").unwrap();
            writer.raw_commit().unwrap();
        }

        {
            let reader = store.reader().unwrap();
            let value = reader.get_raw("table1", &[1]).unwrap();
            assert_eq!(value.as_deref(), Some(b"" as &[u8]));
        }
    }

    #[test]
    fn test_multiple_operations_same_transaction() {
        let store = MemKv::new();

        {
            let mut writer = store.writer().unwrap();

            // Multiple operations on same key - last one should win
            writer.queue_raw_put("table1", &[1], b"first").unwrap();
            writer.queue_raw_put("table1", &[1], b"second").unwrap();
            writer.queue_raw_put("table1", &[1], b"third").unwrap();

            // Read-your-writes should return the latest value
            let value = writer.get_raw("table1", &[1]).unwrap();
            assert_eq!(value.as_deref(), Some(b"third" as &[u8]));

            writer.raw_commit().unwrap();
        }

        {
            let reader = store.reader().unwrap();
            let value = reader.get_raw("table1", &[1]).unwrap();
            assert_eq!(value.as_deref(), Some(b"third" as &[u8]));
        }
    }

    #[test]
    fn test_isolation() {
        let store = MemKv::new();

        // Write initial value
        {
            let mut writer = store.writer().unwrap();
            writer.queue_raw_put("table1", &[1], b"original").unwrap();
            writer.raw_commit().unwrap();
        }

        // Start a read transaction
        {
            let reader = store.reader().unwrap();
            let original_value = reader.get_raw("table1", &[1]).unwrap();
            assert_eq!(original_value.as_deref(), Some(b"original" as &[u8]));
        }

        // Update the value in a separate transaction
        {
            let mut writer = store.writer().unwrap();
            writer.queue_raw_put("table1", &[1], b"updated").unwrap();
            writer.raw_commit().unwrap();
        }

        // The value should now be latest for new readers
        {
            // New reader should see the updated value
            let new_reader = store.reader().unwrap();
            let updated_value = new_reader.get_raw("table1", &[1]).unwrap();
            assert_eq!(updated_value.as_deref(), Some(b"updated" as &[u8]));
        }
    }

    #[test]
    fn test_rollback_on_drop() {
        let store = MemKv::new();

        {
            let mut writer = store.writer().unwrap();
            writer.queue_raw_put("table1", &[1], b"should_not_persist").unwrap();
            // Drop without committing
        }

        // Value should not be persisted
        {
            let reader = store.reader().unwrap();
            let value = reader.get_raw("table1", &[1]).unwrap();
            assert!(value.is_none());
        }
    }

    #[test]
    fn write_two_tables() {
        let store = MemKv::new();

        {
            let mut writer = store.writer().unwrap();
            writer.queue_raw_put("table1", &[1], b"value1").unwrap();
            writer.queue_raw_put("table2", &[2], b"value2").unwrap();
            writer.raw_commit().unwrap();
        }

        {
            let reader = store.reader().unwrap();
            let value1 = reader.get_raw("table1", &[1]).unwrap();
            let value2 = reader.get_raw("table2", &[2]).unwrap();

            assert_eq!(value1.as_deref(), Some(b"value1" as &[u8]));
            assert_eq!(value2.as_deref(), Some(b"value2" as &[u8]));
        }
    }

    #[test]
    fn test_downgrades() {
        let store = MemKv::new();
        {
            // Write some data
            // Start a read-write transaction
            let mut rw_tx = store.writer().unwrap();
            rw_tx.queue_raw_put("table1", &[1, 2, 3], b"value1").unwrap();
            rw_tx.queue_raw_put("table1", &[4, 5, 6], b"value2").unwrap();

            let ro_tx = rw_tx.commit_downgrade();

            // Read the data back
            let value1 = ro_tx.get_raw("table1", &[1, 2, 3]).unwrap();
            let value2 = ro_tx.get_raw("table1", &[4, 5, 6]).unwrap();

            assert_eq!(value1.as_deref(), Some(b"value1" as &[u8]));
            assert_eq!(value2.as_deref(), Some(b"value2" as &[u8]));
        }

        {
            // Start another read-write transaction
            let mut rw_tx = store.writer().unwrap();
            rw_tx.queue_raw_put("table2", &[7, 8, 9], b"value3").unwrap();

            // Value should not be set
            let ro_tx = rw_tx.downgrade();

            // Read the data back
            let value3 = ro_tx.get_raw("table2", &[7, 8, 9]).unwrap();

            assert!(value3.is_none());
        }
    }

    #[test]
    fn test_clear_table() {
        let store = MemKv::new();

        {
            let mut writer = store.writer().unwrap();
            writer.queue_raw_put("table1", &[1], b"value1").unwrap();
            writer.queue_raw_put("table1", &[2], b"value2").unwrap();
            writer.raw_commit().unwrap();
        }

        {
            let reader = store.reader().unwrap();

            let value1 = reader.get_raw("table1", &[1]).unwrap();
            let value2 = reader.get_raw("table1", &[2]).unwrap();

            assert_eq!(value1.as_deref(), Some(b"value1" as &[u8]));
            assert_eq!(value2.as_deref(), Some(b"value2" as &[u8]));
        }

        {
            let mut writer = store.writer().unwrap();

            let value1 = writer.get_raw("table1", &[1]).unwrap();
            let value2 = writer.get_raw("table1", &[2]).unwrap();

            assert_eq!(value1.as_deref(), Some(b"value1" as &[u8]));
            assert_eq!(value2.as_deref(), Some(b"value2" as &[u8]));

            writer.queue_raw_clear("table1").unwrap();

            let value1 = writer.get_raw("table1", &[1]).unwrap();
            let value2 = writer.get_raw("table1", &[2]).unwrap();

            assert!(value1.is_none());
            assert!(value2.is_none());

            writer.raw_commit().unwrap();
        }

        {
            let reader = store.reader().unwrap();
            let value1 = reader.get_raw("table1", &[1]).unwrap();
            let value2 = reader.get_raw("table1", &[2]).unwrap();

            assert!(value1.is_none());
            assert!(value2.is_none());
        }
    }
}
