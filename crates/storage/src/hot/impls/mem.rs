use crate::hot::{
    model::{
        DualKeyTraverse, DualKeyValue, DualTableTraverse, HotKv, HotKvError, HotKvRead,
        HotKvReadError, HotKvWrite, KvTraverse, KvTraverseMut, RawDualKeyValue, RawKeyValue,
        RawValue,
    },
    ser::{DeserError, KeySer, MAX_KEY_SIZE},
    tables::DualKey,
};
use bytes::Bytes;
use std::{
    borrow::Cow,
    collections::BTreeMap,
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

// Type aliases for store structure
type MemStoreKey = [u8; MAX_KEY_SIZE * 2];
type StoreTable = BTreeMap<MemStoreKey, Bytes>;
type Store = BTreeMap<String, StoreTable>;

// Type aliases for queued operations
type TableOp = BTreeMap<MemStoreKey, QueuedKvOp>;
type OpStore = BTreeMap<String, QueuedTableOp>;

/// A simple in-memory key-value store using [`BTreeMap`]s.
///
/// The store is backed by an [`RwLock`]. As a result, this implementation
/// supports concurrent multiple concurrent read transactions, but write
/// transactions are exclusive, and cannot overlap with other read or write
/// transactions.
///
/// This implementation is primarily intended for testing and
/// development purposes.
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
    fn key(k: &[u8]) -> MemStoreKey {
        assert!(k.len() <= MAX_KEY_SIZE * 2, "Key length exceeds MAX_KEY_SIZE");
        let mut buf = [0u8; MAX_KEY_SIZE * 2];
        buf[..k.len()].copy_from_slice(k);
        buf
    }

    #[track_caller]
    fn dual_key(k1: &[u8], k2: &[u8]) -> MemStoreKey {
        assert!(
            k1.len() + k2.len() <= MAX_KEY_SIZE * 2,
            "Combined key length exceeds MAX_KEY_SIZE"
        );
        let mut buf = [0u8; MAX_KEY_SIZE * 2];
        buf[..MAX_KEY_SIZE.min(k1.len())].copy_from_slice(k1);
        buf[MAX_KEY_SIZE..MAX_KEY_SIZE + k2.len()].copy_from_slice(k2);
        buf
    }

    /// SAFETY:
    /// Caller must ensure that `key` lives long enough.
    #[track_caller]
    fn split_dual_key<'a>(key: &[u8]) -> (Cow<'a, [u8]>, Cow<'a, [u8]>) {
        assert_eq!(key.len(), MAX_KEY_SIZE * 2, "Key length does not match expected dual key size");
        let k1 = &key[..MAX_KEY_SIZE];
        let k2 = &key[MAX_KEY_SIZE..];

        unsafe { std::mem::transmute((Cow::Borrowed(k1), Cow::Borrowed(k2))) }
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
    Put { value: Bytes },
}

impl QueuedKvOp {
    /// Apply the op to a table
    fn apply(self, table: &mut StoreTable, key: MemStoreKey) {
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

    fn get(&self, key: &MemStoreKey) -> Option<&QueuedKvOp> {
        match self {
            QueuedTableOp::Modify { ops } => ops.get(key),
            QueuedTableOp::Clear { new_table } => new_table.get(key),
        }
    }

    fn put(&mut self, key: MemStoreKey, op: QueuedKvOp) {
        match self {
            QueuedTableOp::Modify { ops } | QueuedTableOp::Clear { new_table: ops } => {
                ops.insert(key, op);
            }
        }
    }

    fn delete(&mut self, key: MemStoreKey) {
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
                let mut table = StoreTable::new();
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

/// Error type for MemKv operations
#[derive(Debug, thiserror::Error)]
pub enum MemKvError {
    /// Hot KV error
    #[error(transparent)]
    HotKv(#[from] HotKvError),

    /// Serialization error
    #[error(transparent)]
    Deser(#[from] DeserError),
}

impl trevm::revm::database::DBErrorMarker for MemKvError {}

impl HotKvReadError for MemKvError {
    fn into_hot_kv_error(self) -> HotKvError {
        match self {
            MemKvError::HotKv(e) => e,
            MemKvError::Deser(e) => HotKvError::Deser(e),
        }
    }
}

/// Memory cursor for traversing a BTreeMap
pub struct MemKvCursor<'a> {
    table: &'a StoreTable,
    current_key: Option<MemStoreKey>,
}

impl core::fmt::Debug for MemKvCursor<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MemKvCursor").finish()
    }
}

impl<'a> MemKvCursor<'a> {
    /// Create a new cursor for the given table
    pub const fn new(table: &'a StoreTable) -> Self {
        Self { table, current_key: None }
    }

    /// Get the current key the cursor is positioned at
    pub fn current_key(&self) -> MemStoreKey {
        self.current_key.unwrap_or([0u8; MAX_KEY_SIZE * 2])
    }

    /// Set the current key the cursor is positioned at
    pub const fn set_current_key(&mut self, key: MemStoreKey) {
        self.current_key = Some(key);
    }

    /// Clear the current key the cursor is positioned at
    pub const fn clear_current_key(&mut self) {
        self.current_key = None;
    }

    /// Get the current k1 the cursor is positioned at
    fn current_k1(&self) -> [u8; MAX_KEY_SIZE] {
        self.current_key
            .map(|key| key[..MAX_KEY_SIZE].try_into().unwrap())
            .unwrap_or([0u8; MAX_KEY_SIZE])
    }
}

impl<'a> KvTraverse<MemKvError> for MemKvCursor<'a> {
    fn first<'b>(&'b mut self) -> Result<Option<RawKeyValue<'b>>, MemKvError> {
        let Some((key, value)) = self.table.first_key_value() else {
            self.clear_current_key();
            return Ok(None);
        };
        self.current_key = Some(*key);
        Ok(Some((Cow::Borrowed(key), Cow::Borrowed(value.as_ref()))))
    }

    fn last<'b>(&'b mut self) -> Result<Option<RawKeyValue<'b>>, MemKvError> {
        let Some((key, value)) = self.table.last_key_value() else {
            self.clear_current_key();
            return Ok(None);
        };
        self.current_key = Some(*key);
        Ok(Some((Cow::Borrowed(key), Cow::Borrowed(value.as_ref()))))
    }

    fn exact<'b>(&'b mut self, key: &[u8]) -> Result<Option<RawValue<'b>>, MemKvError> {
        let search_key = MemKv::key(key);
        self.set_current_key(search_key);
        if let Some(value) = self.table.get(&search_key) {
            Ok(Some(Cow::Borrowed(value.as_ref())))
        } else {
            Ok(None)
        }
    }

    fn lower_bound<'b>(&'b mut self, key: &[u8]) -> Result<Option<RawKeyValue<'b>>, MemKvError> {
        let search_key = MemKv::key(key);

        // Use range to find the first key >= search_key
        if let Some((found_key, value)) = self.table.range(search_key..).next() {
            self.set_current_key(*found_key);
            Ok(Some((Cow::Borrowed(found_key), Cow::Borrowed(value.as_ref()))))
        } else {
            self.current_key = self.table.last_key_value().map(|(k, _)| *k);
            Ok(None)
        }
    }

    fn read_next<'b>(&'b mut self) -> Result<Option<RawKeyValue<'b>>, MemKvError> {
        use core::ops::Bound;
        let current = self.current_key();
        // Use Excluded bound to find strictly greater than current key
        let Some((found_key, value)) =
            self.table.range((Bound::Excluded(current), Bound::Unbounded)).next()
        else {
            return Ok(None);
        };
        self.set_current_key(*found_key);
        Ok(Some((Cow::Borrowed(found_key), Cow::Borrowed(value.as_ref()))))
    }

    fn read_prev<'b>(&'b mut self) -> Result<Option<RawKeyValue<'b>>, MemKvError> {
        let current = self.current_key();
        let Some((k, v)) = self.table.range(..current).next_back() else {
            self.clear_current_key();
            return Ok(None);
        };
        self.set_current_key(*k);
        Ok(Some((Cow::Borrowed(k), Cow::Borrowed(v.as_ref()))))
    }
}

// Implement DualKeyedTraverse (basic implementation - delegates to raw methods)
impl<'a> DualKeyTraverse<MemKvError> for MemKvCursor<'a> {
    fn exact_dual<'b>(
        &'b mut self,
        key1: &[u8],
        key2: &[u8],
    ) -> Result<Option<RawValue<'b>>, MemKvError> {
        let combined_key = MemKv::dual_key(key1, key2);
        KvTraverse::exact(self, &combined_key)
    }

    fn next_dual_above<'b>(
        &'b mut self,
        key1: &[u8],
        key2: &[u8],
    ) -> Result<Option<RawDualKeyValue<'b>>, MemKvError> {
        let combined_key = MemKv::dual_key(key1, key2);
        let Some((found_key, value)) = KvTraverse::lower_bound(self, &combined_key)? else {
            return Ok(None);
        };
        let (k1, k2) = MemKv::split_dual_key(found_key.as_ref());
        Ok(Some((k1, k2, value)))
    }

    fn next_k1<'b>(&'b mut self) -> Result<Option<RawDualKeyValue<'b>>, MemKvError> {
        // scan forward until finding a new k1
        let last_k1 = self.current_k1();

        DualKeyTraverse::next_dual_above(self, &last_k1, &[0xffu8; MAX_KEY_SIZE])
    }

    fn next_k2<'b>(&'b mut self) -> Result<Option<RawDualKeyValue<'b>>, MemKvError> {
        let current_key = self.current_key();
        let (current_k1, current_k2) = MemKv::split_dual_key(&current_key);

        // scan forward until finding a new k2 for the same k1
        DualKeyTraverse::next_dual_above(self, &current_k1, &current_k2)
    }
}

// Implement DualTableTraverse for typed dual-keyed table access
impl<'a, T> DualTableTraverse<T, MemKvError> for MemKvCursor<'a>
where
    T: DualKey,
{
    fn next_dual_above(
        &mut self,
        key1: &T::Key,
        key2: &T::Key2,
    ) -> Result<Option<DualKeyValue<T>>, MemKvError> {
        let mut key1_buf = [0u8; MAX_KEY_SIZE];
        let mut key2_buf = [0u8; MAX_KEY_SIZE];
        let key1_bytes = key1.encode_key(&mut key1_buf);
        let key2_bytes = key2.encode_key(&mut key2_buf);

        DualKeyTraverse::next_dual_above(self, key1_bytes, key2_bytes)?
            .map(T::decode_kkv_tuple)
            .transpose()
            .map_err(Into::into)
    }

    fn next_k1(&mut self) -> Result<Option<DualKeyValue<T>>, MemKvError> {
        DualKeyTraverse::next_k1(self)?.map(T::decode_kkv_tuple).transpose().map_err(Into::into)
    }

    fn next_k2(&mut self) -> Result<Option<DualKeyValue<T>>, MemKvError> {
        DualKeyTraverse::next_k2(self)?.map(T::decode_kkv_tuple).transpose().map_err(Into::into)
    }
}

/// Memory cursor for read-write operations
pub struct MemKvCursorMut<'a> {
    table: &'a StoreTable,
    queued_ops: &'a mut TableOp,
    is_cleared: bool,
    current_key: Option<MemStoreKey>,
}

impl core::fmt::Debug for MemKvCursorMut<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MemKvCursorMut").field("is_cleared", &self.is_cleared).finish()
    }
}

impl<'a> MemKvCursorMut<'a> {
    /// Create a new mutable cursor for the given table and queued operations
    const fn new(table: &'a StoreTable, queued_ops: &'a mut TableOp, is_cleared: bool) -> Self {
        Self { table, queued_ops, is_cleared, current_key: None }
    }

    /// Get the current key the cursor is positioned at
    pub fn current_key(&self) -> MemStoreKey {
        self.current_key.unwrap_or([0u8; MAX_KEY_SIZE * 2])
    }

    /// Set the current key the cursor is positioned at
    pub const fn set_current_key(&mut self, key: MemStoreKey) {
        self.current_key = Some(key);
    }

    /// Clear the current key the cursor is positioned at
    pub const fn clear_current_key(&mut self) {
        self.current_key = None;
    }

    /// Get the current k1 the cursor is positioned at
    fn current_k1(&self) -> [u8; MAX_KEY_SIZE] {
        self.current_key
            .map(|key| key[..MAX_KEY_SIZE].try_into().unwrap())
            .unwrap_or([0u8; MAX_KEY_SIZE])
    }

    /// Get value for a key, returning owned bytes
    fn get_owned(&self, key: &MemStoreKey) -> Option<Bytes> {
        if let Some(op) = self.queued_ops.get(key) {
            match op {
                QueuedKvOp::Put { value } => Some(value.clone()),
                QueuedKvOp::Delete => None,
            }
        } else if !self.is_cleared {
            self.table.get(key).cloned()
        } else {
            None
        }
    }

    /// Get the first key-value pair >= key, returning owned data
    fn get_range_owned(&self, key: &MemStoreKey) -> Option<(MemStoreKey, Bytes)> {
        let q = self.queued_ops.range(*key..).next();
        let c = if !self.is_cleared { self.table.range(*key..).next() } else { None };

        match (q, c) {
            (None, None) => None,
            (Some((qk, queued)), Some((ck, current))) => {
                if qk <= ck {
                    // Queued operation takes precedence
                    match queued {
                        QueuedKvOp::Put { value } => Some((*qk, value.clone())),
                        QueuedKvOp::Delete => {
                            // Skip deleted entry and look for next
                            let mut next_key = *qk;
                            for i in (0..next_key.len()).rev() {
                                if next_key[i] < u8::MAX {
                                    next_key[i] += 1;
                                    break;
                                }
                                next_key[i] = 0;
                            }
                            self.get_range_owned(&next_key)
                        }
                    }
                } else {
                    Some((*ck, current.clone()))
                }
            }
            (Some((qk, queued)), None) => match queued {
                QueuedKvOp::Put { value } => Some((*qk, value.clone())),
                QueuedKvOp::Delete => {
                    let mut next_key = *qk;
                    for i in (0..next_key.len()).rev() {
                        if next_key[i] < u8::MAX {
                            next_key[i] += 1;
                            break;
                        }
                        next_key[i] = 0;
                    }
                    self.get_range_owned(&next_key)
                }
            },
            (None, Some((ck, current))) => Some((*ck, current.clone())),
        }
    }

    /// Get the first key-value pair > key (strictly greater), returning owned data
    fn get_range_exclusive_owned(&self, key: &MemStoreKey) -> Option<(MemStoreKey, Bytes)> {
        use core::ops::Bound;

        let q = self.queued_ops.range((Bound::Excluded(*key), Bound::Unbounded)).next();
        let c = if !self.is_cleared {
            self.table.range((Bound::Excluded(*key), Bound::Unbounded)).next()
        } else {
            None
        };

        match (q, c) {
            (None, None) => None,
            (Some((qk, queued)), Some((ck, current))) => {
                if qk <= ck {
                    // Queued operation takes precedence
                    match queued {
                        QueuedKvOp::Put { value } => Some((*qk, value.clone())),
                        QueuedKvOp::Delete => {
                            // This key is deleted, recurse to find the next one
                            self.get_range_exclusive_owned(qk)
                        }
                    }
                } else {
                    // Check if the current key has a delete queued
                    if let Some(QueuedKvOp::Delete) = self.queued_ops.get(ck) {
                        self.get_range_exclusive_owned(ck)
                    } else {
                        Some((*ck, current.clone()))
                    }
                }
            }
            (Some((qk, queued)), None) => match queued {
                QueuedKvOp::Put { value } => Some((*qk, value.clone())),
                QueuedKvOp::Delete => self.get_range_exclusive_owned(qk),
            },
            (None, Some((ck, current))) => {
                // Check if the current key has a delete queued
                if let Some(QueuedKvOp::Delete) = self.queued_ops.get(ck) {
                    self.get_range_exclusive_owned(ck)
                } else {
                    Some((*ck, current.clone()))
                }
            }
        }
    }

    /// Get the last key-value pair < key, returning owned data
    fn get_range_reverse_owned(&self, key: &MemStoreKey) -> Option<(MemStoreKey, Bytes)> {
        let q = self.queued_ops.range(..*key).next_back();
        let c = if !self.is_cleared { self.table.range(..*key).next_back() } else { None };

        match (q, c) {
            (None, None) => None,
            (Some((qk, queued)), Some((ck, current))) => {
                if qk >= ck {
                    // Queued operation takes precedence
                    match queued {
                        QueuedKvOp::Put { value } => Some((*qk, value.clone())),
                        QueuedKvOp::Delete => self.get_range_reverse_owned(qk),
                    }
                } else {
                    Some((*ck, current.clone()))
                }
            }
            (Some((qk, queued)), None) => match queued {
                QueuedKvOp::Put { value } => Some((*qk, value.clone())),
                QueuedKvOp::Delete => self.get_range_reverse_owned(qk),
            },
            (None, Some((ck, current))) => Some((*ck, current.clone())),
        }
    }
}

impl<'a> KvTraverse<MemKvError> for MemKvCursorMut<'a> {
    fn first<'b>(&'b mut self) -> Result<Option<RawKeyValue<'b>>, MemKvError> {
        let start_key = [0u8; MAX_KEY_SIZE * 2];

        // Get the first effective key-value pair
        if let Some((key, value)) = self.get_range_owned(&start_key) {
            self.current_key = Some(key);
            Ok(Some((Cow::Owned(key.to_vec()), Cow::Owned(value.to_vec()))))
        } else {
            self.current_key = None;
            Ok(None)
        }
    }

    fn last<'b>(&'b mut self) -> Result<Option<RawKeyValue<'b>>, MemKvError> {
        let end_key = [0xffu8; MAX_KEY_SIZE * 2];

        if let Some((key, value)) = self.get_range_reverse_owned(&end_key) {
            self.current_key = Some(key);
            Ok(Some((Cow::Owned(key.to_vec()), Cow::Owned(value.to_vec()))))
        } else {
            self.current_key = None;
            Ok(None)
        }
    }

    fn exact<'b>(&'b mut self, key: &[u8]) -> Result<Option<RawValue<'b>>, MemKvError> {
        let search_key = MemKv::key(key);
        self.current_key = Some(search_key);

        if let Some(value) = self.get_owned(&search_key) {
            Ok(Some(Cow::Owned(value.to_vec())))
        } else {
            Ok(None)
        }
    }

    fn lower_bound<'b>(&'b mut self, key: &[u8]) -> Result<Option<RawKeyValue<'b>>, MemKvError> {
        let search_key = MemKv::key(key);

        if let Some((found_key, value)) = self.get_range_owned(&search_key) {
            self.current_key = Some(found_key);
            Ok(Some((Cow::Owned(found_key.to_vec()), Cow::Owned(value.to_vec()))))
        } else {
            self.current_key = None;
            Ok(None)
        }
    }

    fn read_next<'b>(&'b mut self) -> Result<Option<RawKeyValue<'b>>, MemKvError> {
        let current = self.current_key();

        // Use exclusive range to find strictly greater than current key
        if let Some((found_key, value)) = self.get_range_exclusive_owned(&current) {
            self.current_key = Some(found_key);
            Ok(Some((Cow::Owned(found_key.to_vec()), Cow::Owned(value.to_vec()))))
        } else {
            self.current_key = None;
            Ok(None)
        }
    }

    fn read_prev<'b>(&'b mut self) -> Result<Option<RawKeyValue<'b>>, MemKvError> {
        let current = self.current_key();

        if let Some((found_key, value)) = self.get_range_reverse_owned(&current) {
            self.current_key = Some(found_key);
            Ok(Some((Cow::Owned(found_key.to_vec()), Cow::Owned(value.to_vec()))))
        } else {
            self.current_key = None;
            Ok(None)
        }
    }
}

impl<'a> KvTraverseMut<MemKvError> for MemKvCursorMut<'a> {
    fn delete_current(&mut self) -> Result<(), MemKvError> {
        if let Some(key) = self.current_key {
            // Queue a delete operation
            self.queued_ops.insert(key, QueuedKvOp::Delete);
            Ok(())
        } else {
            Err(MemKvError::HotKv(HotKvError::Inner("No current key to delete".into())))
        }
    }
}

impl<'a> DualKeyTraverse<MemKvError> for MemKvCursorMut<'a> {
    fn exact_dual<'b>(
        &'b mut self,
        key1: &[u8],
        key2: &[u8],
    ) -> Result<Option<RawValue<'b>>, MemKvError> {
        let combined_key = MemKv::dual_key(key1, key2);
        KvTraverse::exact(self, &combined_key)
    }

    fn next_dual_above<'b>(
        &'b mut self,
        key1: &[u8],
        key2: &[u8],
    ) -> Result<Option<RawDualKeyValue<'b>>, MemKvError> {
        let combined_key = MemKv::dual_key(key1, key2);
        let Some((found_key, value)) = KvTraverse::lower_bound(self, &combined_key)? else {
            return Ok(None);
        };

        let (key1, key2) = MemKv::split_dual_key(found_key.as_ref());
        Ok(Some((key1, key2, value)))
    }

    fn next_k1<'b>(&'b mut self) -> Result<Option<RawDualKeyValue<'b>>, MemKvError> {
        // scan forward until finding a new k1
        let last_k1 = self.current_k1();

        DualKeyTraverse::next_dual_above(self, &last_k1, &[0xffu8; MAX_KEY_SIZE])
    }

    fn next_k2<'b>(&'b mut self) -> Result<Option<RawDualKeyValue<'b>>, MemKvError> {
        let current_key = self.current_key();
        let (current_k1, current_k2) = MemKv::split_dual_key(&current_key);

        // scan forward until finding a new k2 for the same k1
        DualKeyTraverse::next_dual_above(self, &current_k1, &current_k2)
    }
}

// Implement DualTableTraverse for typed dual-keyed table access
impl<'a, T> DualTableTraverse<T, MemKvError> for MemKvCursorMut<'a>
where
    T: DualKey,
{
    fn next_dual_above(
        &mut self,
        key1: &T::Key,
        key2: &T::Key2,
    ) -> Result<Option<DualKeyValue<T>>, MemKvError> {
        let mut key1_buf = [0u8; MAX_KEY_SIZE];
        let mut key2_buf = [0u8; MAX_KEY_SIZE];
        let key1_bytes = key1.encode_key(&mut key1_buf);
        let key2_bytes = key2.encode_key(&mut key2_buf);

        DualKeyTraverse::next_dual_above(self, key1_bytes, key2_bytes)?
            .map(T::decode_kkv_tuple)
            .transpose()
            .map_err(Into::into)
    }

    fn next_k1(&mut self) -> Result<Option<DualKeyValue<T>>, MemKvError> {
        DualKeyTraverse::next_k1(self)?.map(T::decode_kkv_tuple).transpose().map_err(Into::into)
    }

    fn next_k2(&mut self) -> Result<Option<DualKeyValue<T>>, MemKvError> {
        DualKeyTraverse::next_k2(self)?.map(T::decode_kkv_tuple).transpose().map_err(Into::into)
    }
}

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
    type Error = MemKvError;

    type Traverse<'a> = MemKvCursor<'a>;

    fn raw_traverse<'a>(&'a self, table: &str) -> Result<Self::Traverse<'a>, Self::Error> {
        let table_data = self.guard.get(table).unwrap_or(&EMPTY_TABLE);
        Ok(MemKvCursor::new(table_data))
    }

    fn raw_get<'a>(
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

    fn raw_get_dual<'a>(
        &'a self,
        table: &str,
        key1: &[u8],
        key2: &[u8],
    ) -> Result<Option<Cow<'a, [u8]>>, Self::Error> {
        let key = MemKv::dual_key(key1, key2);
        self.raw_get(table, &key)
    }
}

static EMPTY_TABLE: StoreTable = BTreeMap::new();

impl MemKvRoTx {
    /// Get a cursor for the specified table
    pub fn cursor<'a>(&'a self, table: &str) -> Result<MemKvCursor<'a>, MemKvError> {
        let table_data = self.guard.get(table).unwrap_or(&EMPTY_TABLE);
        Ok(MemKvCursor::new(table_data))
    }
}

impl HotKvRead for MemKvRwTx {
    type Error = MemKvError;

    type Traverse<'a> = MemKvCursor<'a>;

    fn raw_traverse<'a>(&'a self, table: &str) -> Result<Self::Traverse<'a>, Self::Error> {
        let table_data = self.guard.get(table).unwrap_or(&EMPTY_TABLE);
        Ok(MemKvCursor::new(table_data))
    }

    fn raw_get<'a>(
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

    fn raw_get_dual<'a>(
        &'a self,
        table: &str,
        key1: &[u8],
        key2: &[u8],
    ) -> Result<Option<Cow<'a, [u8]>>, Self::Error> {
        let key = MemKv::dual_key(key1, key2);
        self.raw_get(table, &key)
    }
}

impl MemKvRwTx {
    /// Get a read-only cursor for the specified table
    /// Note: This cursor will NOT see pending writes from this transaction
    pub fn cursor<'a>(&'a self, table: &str) -> Result<MemKvCursor<'a>, MemKvError> {
        if let Some(table_data) = self.guard.get(table) {
            Ok(MemKvCursor::new(table_data))
        } else {
            Err(MemKvError::HotKv(HotKvError::Inner(format!("Table '{}' not found", table).into())))
        }
    }

    /// Get a mutable cursor for the specified table
    /// This cursor will see both committed data and pending writes from this transaction
    pub fn cursor_mut<'a>(&'a mut self, table: &str) -> Result<MemKvCursorMut<'a>, MemKvError> {
        // Get or create the table data
        let table_data = self.guard.entry(table.to_owned()).or_default();

        // Get or create the queued operations for this table
        let table_ops = self.queued_ops.entry(table.to_owned()).or_default();

        let is_cleared = table_ops.is_clear();

        // Extract the inner TableOp from QueuedTableOp
        let ops = match table_ops {
            QueuedTableOp::Modify { ops } => ops,
            QueuedTableOp::Clear { new_table } => new_table,
        };

        Ok(MemKvCursorMut::new(table_data, ops, is_cleared))
    }
}

impl HotKvWrite for MemKvRwTx {
    type TraverseMut<'a>
        = MemKvCursorMut<'a>
    where
        Self: 'a;

    fn raw_traverse_mut<'a>(
        &'a mut self,
        table: &str,
    ) -> Result<Self::TraverseMut<'a>, Self::Error> {
        self.cursor_mut(table)
    }

    fn queue_raw_put(&mut self, table: &str, key: &[u8], value: &[u8]) -> Result<(), Self::Error> {
        let key = MemKv::key(key);

        let value_bytes = Bytes::copy_from_slice(value);

        self.queued_ops
            .entry(table.to_owned())
            .or_default()
            .put(key, QueuedKvOp::Put { value: value_bytes });
        Ok(())
    }

    fn queue_raw_put_dual(
        &mut self,
        table: &str,
        key1: &[u8],
        key2: &[u8],
        value: &[u8],
    ) -> Result<(), Self::Error> {
        let key = MemKv::dual_key(key1, key2);
        self.queue_raw_put(table, &key, value)
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

    fn queue_raw_create(
        &mut self,
        _table: &str,
        _dual_key: bool,
        _dual_fixed: bool,
    ) -> Result<(), Self::Error> {
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
    use crate::hot::{
        conformance::conformance,
        model::{DualTableTraverse, TableTraverse, TableTraverseMut},
        tables::{DualKey, SingleKey, Table},
    };
    use alloy::primitives::{Address, U256};
    use bytes::Bytes;

    // Test table definitions
    #[derive(Debug)]
    struct TestTable;

    impl SingleKey for TestTable {}

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

    impl SingleKey for AddressTable {}

    #[derive(Debug)]
    struct DualTestTable;

    impl Table for DualTestTable {
        const NAME: &'static str = "dual_test_table";
        type Key = u64;
        type Value = Bytes;
    }

    impl DualKey for DualTestTable {
        type Key2 = u32;
    }

    #[test]
    fn test_new_store() {
        let store = MemKv::new();
        let reader = store.reader().unwrap();

        // Empty store should return None for any key
        assert!(reader.raw_get("test", &[1, 2, 3]).unwrap().is_none());
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
            let value1 = reader.raw_get("table1", &[1, 2, 3]).unwrap();
            let value2 = reader.raw_get("table1", &[4, 5, 6]).unwrap();
            let missing = reader.raw_get("table1", &[7, 8, 9]).unwrap();

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
            let value1 = reader.raw_get("table1", &[1]).unwrap();
            let value2 = reader.raw_get("table2", &[1]).unwrap();

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
            let value = reader.raw_get("table1", &[1]).unwrap();
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
        let value = writer.raw_get("table1", &[1]).unwrap();
        assert_eq!(value.as_deref(), Some(b"queued_value" as &[u8]));

        writer.raw_commit().unwrap();

        // After commit, other readers should see it
        {
            let reader = store.reader().unwrap();
            let value = reader.raw_get("table1", &[1]).unwrap();
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
            assert_eq!(values[0], (&1u64, Some(Bytes::from_static(b"first"))));
            assert_eq!(values[1], (&2u64, Some(Bytes::from_static(b"second"))));
            assert_eq!(values[2], (&3u64, Some(Bytes::from_static(b"third"))));
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

        let value1 = reader1.raw_get("table1", &[1]).unwrap();
        let value2 = reader2.raw_get("table1", &[1]).unwrap();

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
            let value = reader.raw_get("table1", &[1]).unwrap();
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
            let value = writer.raw_get("table1", &[1]).unwrap();
            assert_eq!(value.as_deref(), Some(b"third" as &[u8]));

            writer.raw_commit().unwrap();
        }

        {
            let reader = store.reader().unwrap();
            let value = reader.raw_get("table1", &[1]).unwrap();
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
            let original_value = reader.raw_get("table1", &[1]).unwrap();
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
            let updated_value = new_reader.raw_get("table1", &[1]).unwrap();
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
            let value = reader.raw_get("table1", &[1]).unwrap();
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
            let value1 = reader.raw_get("table1", &[1]).unwrap();
            let value2 = reader.raw_get("table2", &[2]).unwrap();

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
            let value1 = ro_tx.raw_get("table1", &[1, 2, 3]).unwrap();
            let value2 = ro_tx.raw_get("table1", &[4, 5, 6]).unwrap();

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
            let value3 = ro_tx.raw_get("table2", &[7, 8, 9]).unwrap();

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

            let value1 = reader.raw_get("table1", &[1]).unwrap();
            let value2 = reader.raw_get("table1", &[2]).unwrap();

            assert_eq!(value1.as_deref(), Some(b"value1" as &[u8]));
            assert_eq!(value2.as_deref(), Some(b"value2" as &[u8]));
        }

        {
            let mut writer = store.writer().unwrap();

            let value1 = writer.raw_get("table1", &[1]).unwrap();
            let value2 = writer.raw_get("table1", &[2]).unwrap();

            assert_eq!(value1.as_deref(), Some(b"value1" as &[u8]));
            assert_eq!(value2.as_deref(), Some(b"value2" as &[u8]));

            writer.queue_raw_clear("table1").unwrap();

            let value1 = writer.raw_get("table1", &[1]).unwrap();
            let value2 = writer.raw_get("table1", &[2]).unwrap();

            assert!(value1.is_none());
            assert!(value2.is_none());

            writer.raw_commit().unwrap();
        }

        {
            let reader = store.reader().unwrap();
            let value1 = reader.raw_get("table1", &[1]).unwrap();
            let value2 = reader.raw_get("table1", &[2]).unwrap();

            assert!(value1.is_none());
            assert!(value2.is_none());
        }
    }

    // ========================================================================
    // Cursor Traversal Tests
    // ========================================================================

    #[test]
    fn test_cursor_basic_navigation() {
        let store = MemKv::new();

        // Setup test data using TestTable
        let test_data = vec![
            (1u64, Bytes::from_static(b"value_001")),
            (2u64, Bytes::from_static(b"value_002")),
            (3u64, Bytes::from_static(b"value_003")),
            (10u64, Bytes::from_static(b"value_010")),
            (20u64, Bytes::from_static(b"value_020")),
        ];

        // Insert data
        {
            let mut writer = store.writer().unwrap();
            for (key, value) in &test_data {
                writer.queue_put::<TestTable>(key, value).unwrap();
            }
            writer.raw_commit().unwrap();
        }

        // Test cursor navigation
        {
            let reader = store.reader().unwrap();
            let mut cursor = reader.cursor(TestTable::NAME).unwrap();

            // Test first()
            let (key, value) = TableTraverse::<TestTable, _>::first(&mut cursor).unwrap().unwrap();
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

            // Test next_above (range lookup)
            let range_result =
                TableTraverse::<TestTable, _>::lower_bound(&mut cursor, &5u64).unwrap();
            assert!(range_result.is_some());
            let (key, value) = range_result.unwrap();
            assert_eq!(key, test_data[3].0); // 10u64
            assert_eq!(value, test_data[3].1);
        }
    }

    #[test]
    fn test_cursor_sequential_navigation() {
        let store = MemKv::new();

        // Setup sequential test data using TestTable
        let test_data: Vec<(u64, Bytes)> = (1..=5)
            .map(|i| {
                let key = i;
                let value = Bytes::from(format!("value_{:03}", i));
                (key, value)
            })
            .collect();

        // Insert data
        {
            let mut writer = store.writer().unwrap();
            for (key, value) in &test_data {
                writer.queue_put::<TestTable>(key, value).unwrap();
            }
            writer.raw_commit().unwrap();
        }

        // Test sequential navigation
        {
            let reader = store.reader().unwrap();
            let mut cursor = reader.cursor(TestTable::NAME).unwrap();

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
    fn test_cursor_mut_operations() {
        let store = MemKv::new();

        let test_data = vec![
            (1u64, Bytes::from_static(b"delete_value_1")),
            (2u64, Bytes::from_static(b"delete_value_2")),
            (3u64, Bytes::from_static(b"delete_value_3")),
        ];

        // Insert initial data
        {
            let mut writer = store.writer().unwrap();
            for (key, value) in &test_data {
                writer.queue_put::<TestTable>(key, value).unwrap();
            }
            writer.raw_commit().unwrap();
        }

        // Test mutable cursor operations
        {
            let mut writer = store.writer().unwrap();
            let mut cursor = writer.cursor_mut(TestTable::NAME).unwrap();

            // Navigate to middle entry
            let first = TableTraverse::<TestTable, _>::first(&mut cursor).unwrap().unwrap();
            assert_eq!(first.0, test_data[0].0);

            let next = TableTraverse::<TestTable, _>::read_next(&mut cursor).unwrap().unwrap();
            assert_eq!(next.0, test_data[1].0);

            // Delete current entry (key 2)
            TableTraverseMut::<TestTable, _>::delete_current(&mut cursor).unwrap();

            writer.raw_commit().unwrap();
        }

        // Verify deletion
        {
            let reader = store.reader().unwrap();
            let mut cursor = reader.cursor(TestTable::NAME).unwrap();

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
    fn test_table_traverse_typed() {
        let store = MemKv::new();

        // Setup test data using the test table
        let test_data: Vec<(u64, bytes::Bytes)> = (0..5)
            .map(|i| {
                let key = i * 10;
                let value = bytes::Bytes::from(format!("test_value_{}", i));
                (key, value)
            })
            .collect();

        // Insert data
        {
            let mut writer = store.writer().unwrap();
            for (key, value) in &test_data {
                writer.queue_put::<TestTable>(key, value).unwrap();
            }
            writer.raw_commit().unwrap();
        }

        // Test typed table traversal
        {
            let reader = store.reader().unwrap();
            let mut cursor = reader.cursor(TestTable::NAME).unwrap();

            // Test first with type-safe operations
            let first_raw = TableTraverse::<TestTable, _>::first(&mut cursor).unwrap();
            assert!(first_raw.is_some());
            let (first_key, first_value) = first_raw.unwrap();
            assert_eq!(first_key, test_data[0].0);
            assert_eq!(first_value, test_data[0].1);

            // Test last
            let last_raw = TableTraverse::<TestTable, _>::last(&mut cursor).unwrap();
            assert!(last_raw.is_some());
            let (last_key, last_value) = last_raw.unwrap();
            assert_eq!(last_key, test_data.last().unwrap().0);
            assert_eq!(last_value, test_data.last().unwrap().1);

            // Test exact lookup
            let target_key = &test_data[2].0;
            let exact_value =
                TableTraverse::<TestTable, _>::exact(&mut cursor, target_key).unwrap();
            assert!(exact_value.is_some());
            assert_eq!(exact_value.unwrap(), test_data[2].1);

            // Test range lookup
            let range_key = 15u64; // Between entries 1 and 2

            let range_result =
                TableTraverse::<TestTable, _>::lower_bound(&mut cursor, &range_key).unwrap();
            assert!(range_result.is_some());
            let (found_key, found_value) = range_result.unwrap();
            assert_eq!(found_key, test_data[2].0); // key 20
            assert_eq!(found_value, test_data[2].1);
        }
    }

    #[test]
    fn test_cursor_empty_table() {
        let store = MemKv::new();

        // Create an empty table first
        {
            let mut writer = store.writer().unwrap();
            writer.queue_raw_create(TestTable::NAME, false, false).unwrap();
            writer.raw_commit().unwrap();
        }

        {
            let reader = store.reader().unwrap();
            let mut cursor = reader.cursor(TestTable::NAME).unwrap();

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
    fn test_cursor_state_management() {
        let store = MemKv::new();

        let test_data = vec![
            (1u64, Bytes::from_static(b"state_value_1")),
            (2u64, Bytes::from_static(b"state_value_2")),
            (3u64, Bytes::from_static(b"state_value_3")),
        ];

        {
            let mut writer = store.writer().unwrap();
            for (key, value) in &test_data {
                writer.queue_put::<TestTable>(key, value).unwrap();
            }
            writer.raw_commit().unwrap();
        }

        {
            let reader = store.reader().unwrap();
            let mut cursor = reader.cursor(TestTable::NAME).unwrap();

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

            // Use range lookup
            let range_lookup =
                TableTraverse::<TestTable, _>::lower_bound(&mut cursor, &1u64).unwrap().unwrap(); // Should find key 1
            assert_eq!(range_lookup.0, test_data[0].0);

            // Verify we can continue navigation from range position
            let next_after_range =
                TableTraverse::<TestTable, _>::read_next(&mut cursor).unwrap().unwrap();
            assert_eq!(next_after_range.0, test_data[1].0);
        }
    }

    #[test]
    fn test_dual_key_operations() {
        let store = MemKv::new();

        // Test dual key storage and retrieval using DualTestTable
        let dual_data = vec![
            (1u64, 100u32, Bytes::from_static(b"value1")),
            (1u64, 200u32, Bytes::from_static(b"value2")),
            (2u64, 100u32, Bytes::from_static(b"value3")),
        ];

        {
            let mut writer = store.writer().unwrap();
            for (key1, key2, value) in &dual_data {
                writer.queue_put_dual::<DualTestTable>(key1, key2, value).unwrap();
            }
            writer.raw_commit().unwrap();
        }

        // Test dual key traversal
        {
            let reader = store.reader().unwrap();
            let mut cursor = reader.cursor(DualTestTable::NAME).unwrap();

            // Test exact dual lookup
            let exact_result =
                DualTableTraverse::<DualTestTable, _>::exact_dual(&mut cursor, &1u64, &200u32)
                    .unwrap();
            assert!(exact_result.is_some());
            assert_eq!(exact_result.unwrap(), Bytes::from_static(b"value2"));

            // Test missing dual key
            let missing_result =
                DualTableTraverse::<DualTestTable, _>::exact_dual(&mut cursor, &3u64, &100u32)
                    .unwrap();
            assert!(missing_result.is_none());

            // Test next_dual_above
            let range_result =
                DualTableTraverse::<DualTestTable, _>::next_dual_above(&mut cursor, &1u64, &150u32)
                    .unwrap();
            assert!(range_result.is_some());
            let (k, k2, value) = range_result.unwrap();
            assert_eq!(k, 1u64);
            assert_eq!(k2, 200u32);
            assert_eq!(value, Bytes::from_static(b"value2"));

            // Test next_k1 to find next different first key
            let next_k1_result =
                DualTableTraverse::<DualTestTable, _>::next_k1(&mut cursor).unwrap();
            assert!(next_k1_result.is_some());
            let (k, k2, value) = next_k1_result.unwrap();
            assert_eq!(k, 2u64);
            assert_eq!(k2, 100u32);
            assert_eq!(value, Bytes::from_static(b"value3"));
        }
    }

    #[test]
    fn mem_conformance() {
        let hot_kv = MemKv::new();
        conformance(&hot_kv);
    }
}
