use crate::hot::{
    model::{
        DualKeyTraverse, DualKeyValue, DualTableCursor, GetManyItem, HotKvError, HotKvReadError,
        KeyValue, KvTraverse, KvTraverseMut, TableCursor,
        revm::{RevmRead, RevmWrite},
    },
    ser::{KeySer, MAX_KEY_SIZE, ValSer},
    tables::{DualKey, SingleKey, Table},
};
use std::{borrow::Cow, ops::RangeInclusive};

/// Trait for hot storage. This is a KV store with read/write transactions.
///
/// This is the top-level trait for hot storage backends, providing
/// transactional access through read-only and read-write transactions.
#[auto_impl::auto_impl(&, Arc, Box)]
pub trait HotKv {
    /// The read-only transaction type.
    type RoTx: HotKvRead;
    /// The read-write transaction type.
    type RwTx: HotKvWrite;

    /// Create a read-only transaction.
    fn reader(&self) -> Result<Self::RoTx, HotKvError>;

    /// Create a read-only transaction, and wrap it in an adapter for the
    /// revm [`DatabaseRef`] trait. The resulting reader can be used directly
    /// with [`trevm`] and [`revm`].
    ///
    /// [`DatabaseRef`]: trevm::revm::database::DatabaseRef
    fn revm_reader(&self) -> Result<RevmRead<Self::RoTx>, HotKvError> {
        self.reader().map(RevmRead::new)
    }

    /// Create a read-write transaction.
    ///
    /// This is allowed to fail with [`Err(HotKvError::WriteLocked)`] if
    /// multiple write transactions are not supported concurrently.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(tx))` if the write transaction was created successfully.
    /// - [`Err(HotKvError::WriteLocked)`] if there is already a write
    ///   transaction in progress.
    /// - [`Err(HotKvError::Inner)`] if there was an error creating the
    ///   transaction.
    ///
    /// [`Err(HotKvError::Inner)`]: HotKvError::Inner
    /// [`Err(HotKvError::WriteLocked)`]: HotKvError::WriteLocked
    fn writer(&self) -> Result<Self::RwTx, HotKvError>;

    /// Create a read-write transaction, and wrap it in an adapter for the
    /// revm [`TryDatabaseCommit`] trait. The resulting writer can be used
    /// directly with [`trevm`] and [`revm`].
    ///
    ///
    /// [`revm`]: trevm::revm
    /// [`TryDatabaseCommit`]: trevm::revm::database::TryDatabaseCommit
    fn revm_writer(&self) -> Result<RevmWrite<Self::RwTx>, HotKvError> {
        self.writer().map(RevmWrite::new)
    }
}

/// Trait for hot storage read transactions.
///
/// This trait provides read-only access to hot storage tables. It should only
/// be imported if accessing custom tables, or when implementing new hot storage
/// backends.
#[auto_impl::auto_impl(&, Arc, Box)]
pub trait HotKvRead {
    /// Error type for read operations.
    type Error: HotKvReadError;

    /// The cursor type for traversing key-value pairs.
    type Traverse<'a>: KvTraverse<Self::Error> + DualKeyTraverse<Self::Error>
    where
        Self: 'a;

    /// Get a raw cursor to traverse the database.
    fn raw_traverse<'a>(&'a self, table: &'static str) -> Result<Self::Traverse<'a>, Self::Error>;

    /// Get a raw value from a specific table.
    ///
    /// The `key` buf must be <= [`MAX_KEY_SIZE`] bytes. Implementations are
    /// allowed to panic if this is not the case.
    ///
    /// If the table is dual-keyed, the output MAY be implementation-defined.
    fn raw_get<'a>(
        &'a self,
        table: &'static str,
        key: &[u8],
    ) -> Result<Option<Cow<'a, [u8]>>, Self::Error>;

    /// Get a raw value from a specific table with dual keys.
    ///
    /// If `key1` is present, but `key2` is not in the table, the output is
    /// implementation-defined. For sorted databases, it SHOULD return the value
    /// of the NEXT populated key. It MAY also return `None`, even if other
    /// subkeys are populated.
    ///
    /// If the table is not dual-keyed, the output MAY be
    /// implementation-defined.
    fn raw_get_dual<'a>(
        &'a self,
        table: &'static str,
        key1: &[u8],
        key2: &[u8],
    ) -> Result<Option<Cow<'a, [u8]>>, Self::Error>;

    /// Traverse a specific table. Returns a typed cursor wrapper.
    fn traverse<'a, T: SingleKey>(
        &'a self,
    ) -> Result<TableCursor<Self::Traverse<'a>, T, Self::Error>, Self::Error> {
        let cursor = self.raw_traverse(T::NAME)?;
        Ok(TableCursor::new(cursor))
    }

    /// Traverse a specific dual-keyed table. Returns a typed dual-keyed
    /// cursor wrapper.
    fn traverse_dual<'a, T: DualKey>(
        &'a self,
    ) -> Result<DualTableCursor<Self::Traverse<'a>, T, Self::Error>, Self::Error> {
        let cursor = self.raw_traverse(T::NAME)?;
        Ok(DualTableCursor::new(cursor))
    }

    /// Get a value from a specific table.
    fn get<T: SingleKey>(&self, key: &T::Key) -> Result<Option<T::Value>, Self::Error> {
        let mut key_buf = [0u8; MAX_KEY_SIZE];
        let key_bytes = key.encode_key(&mut key_buf);
        debug_assert!(
            key_bytes.len() == T::Key::SIZE,
            "Encoded key length does not match expected size"
        );

        let Some(value_bytes) = self.raw_get(T::NAME, key_bytes)? else {
            return Ok(None);
        };
        T::Value::decode_value(&value_bytes).map(Some).map_err(Into::into)
    }

    /// Get a value from a specific dual-keyed table.
    ///
    /// If `key1` is present, but `key2` is not in the table, the output is
    /// implementation-defined. For sorted databases, it SHOULD return the value
    /// of the NEXT populated key. It MAY also return `None`, even if other
    /// subkeys are populated.
    ///
    /// If the table is not dual-keyed, the output MAY be
    /// implementation-defined.
    fn get_dual<T: DualKey>(
        &self,
        key1: &T::Key,
        key2: &T::Key2,
    ) -> Result<Option<T::Value>, Self::Error> {
        let mut key1_buf = [0u8; MAX_KEY_SIZE];
        let mut key2_buf = [0u8; MAX_KEY_SIZE];

        let key1_bytes = key1.encode_key(&mut key1_buf);
        let key2_bytes = key2.encode_key(&mut key2_buf);

        let Some(value_bytes) = self.raw_get_dual(T::NAME, key1_bytes, key2_bytes)? else {
            return Ok(None);
        };
        T::Value::decode_value(&value_bytes).map(Some).map_err(Into::into)
    }

    /// Get many values from a specific table.
    ///
    /// # Arguments
    ///
    /// * `keys` - An iterator over keys to retrieve.
    ///
    /// # Returns
    ///
    /// A vector of [`KeyValue`] where each element
    /// corresponds to the value for the respective key in the input iterator.
    /// If a key does not exist in the table, the corresponding element will be
    /// `None`.
    ///
    /// Implementations ARE NOT required to preserve the order of the input
    /// keys in the output vector. Users should not rely on any specific
    /// ordering.
    ///
    /// If any error occurs during retrieval or deserialization, the entire
    /// operation will return an error.
    fn get_many<'a, T, I>(&self, keys: I) -> Result<Vec<GetManyItem<'a, T>>, Self::Error>
    where
        T::Key: 'a,
        T: SingleKey,
        I: IntoIterator<Item = &'a T::Key>,
    {
        let mut key_buf = [0u8; MAX_KEY_SIZE];

        keys.into_iter()
            .map(|key| (key, self.raw_get(T::NAME, key.encode_key(&mut key_buf))))
            .map(|(key, maybe_val)| {
                maybe_val
                    .and_then(|val| ValSer::maybe_decode_value(val.as_deref()).map_err(Into::into))
                    .map(|res| (key, res))
            })
            .collect()
    }
}

/// Trait for hot storage write transactions.
///
/// This extends the [`HotKvRead`] trait with write capabilities.
pub trait HotKvWrite: HotKvRead {
    /// The mutable cursor type for traversing key-value pairs.
    type TraverseMut<'a>: KvTraverseMut<Self::Error> + DualKeyTraverse<Self::Error>
    where
        Self: 'a;

    /// Get a raw mutable cursor to traverse the database.
    fn raw_traverse_mut<'a>(
        &'a self,
        table: &'static str,
    ) -> Result<Self::TraverseMut<'a>, Self::Error>;

    /// Queue a raw put operation.
    ///
    /// The `key` buf must be <= [`MAX_KEY_SIZE`] bytes. Implementations are
    /// allowed to panic if this is not the case.
    fn queue_raw_put(
        &self,
        table: &'static str,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), Self::Error>;

    /// Queue a raw put operation for a dual-keyed table.
    ////
    /// The `key1` and `key2` buf must be <= [`MAX_KEY_SIZE`] bytes.
    /// Implementations are allowed to panic if this is not the case.
    fn queue_raw_put_dual(
        &self,
        table: &'static str,
        key1: &[u8],
        key2: &[u8],
        value: &[u8],
    ) -> Result<(), Self::Error>;

    /// Queue a raw delete operation.
    ///
    /// The `key` buf must be <= [`MAX_KEY_SIZE`] bytes. Implementations are
    /// allowed to panic if this is not the case.
    fn queue_raw_delete(&self, table: &'static str, key: &[u8]) -> Result<(), Self::Error>;

    /// Queue a raw delete operation for a dual-keyed table.
    ///
    /// The `key1` and `key2` buf must be <= [`MAX_KEY_SIZE`] bytes.
    /// Implementations are allowed to panic if this is not the case.
    fn queue_raw_delete_dual(
        &self,
        table: &'static str,
        key1: &[u8],
        key2: &[u8],
    ) -> Result<(), Self::Error>;

    /// Queue a raw clear operation for a specific table.
    fn queue_raw_clear(&self, table: &'static str) -> Result<(), Self::Error>;

    /// Queue a raw create operation for a specific table.
    ///
    /// This abstraction supports two table specializations:
    /// 1. `dual_key_size` - whether the table is dual-keyed (i.e.,
    ///    `DUPSORT` in LMDB/MDBX). If so, the argument MUST be the
    ///    encoded size of the second key. If not, it MUST be `None`.
    /// 2. `fixed_val_size`: whether the table has fixed-size values.
    ///    If so, the argument MUST be the size of the fixed value.
    ///    If not, it MUST be `None`.
    ///
    /// Database implementations can use this information for optimizations.
    fn queue_raw_create(
        &self,
        table: &'static str,
        dual_key_size: Option<usize>,
        fixed_val_size: Option<usize>,
    ) -> Result<(), Self::Error>;

    /// Traverse a specific table. Returns a mutable typed cursor wrapper.
    /// If invoked for a dual-keyed table, it will traverse the primary keys
    /// only, and the return value may be implementation-defined.
    fn traverse_mut<'a, T: SingleKey>(
        &'a self,
    ) -> Result<TableCursor<Self::TraverseMut<'a>, T, Self::Error>, Self::Error> {
        let cursor = self.raw_traverse_mut(T::NAME)?;
        Ok(TableCursor::new(cursor))
    }

    /// Traverse a specific dual-keyed table. Returns a mutable typed
    /// dual-keyed cursor wrapper.
    fn traverse_dual_mut<'a, T: DualKey>(
        &'a self,
    ) -> Result<DualTableCursor<Self::TraverseMut<'a>, T, Self::Error>, Self::Error> {
        let cursor = self.raw_traverse_mut(T::NAME)?;
        Ok(DualTableCursor::new(cursor))
    }

    /// Queue a put operation for a specific table.
    fn queue_put<T: SingleKey>(&self, key: &T::Key, value: &T::Value) -> Result<(), Self::Error> {
        let mut key_buf = [0u8; MAX_KEY_SIZE];
        let key_bytes = key.encode_key(&mut key_buf);
        let value_bytes = value.encoded();

        self.queue_raw_put(T::NAME, key_bytes, &value_bytes)
    }

    /// Queue a put operation for a specific dual-keyed table.
    fn queue_put_dual<T: DualKey>(
        &self,
        key1: &T::Key,
        key2: &T::Key2,
        value: &T::Value,
    ) -> Result<(), Self::Error> {
        let mut key1_buf = [0u8; MAX_KEY_SIZE];
        let mut key2_buf = [0u8; MAX_KEY_SIZE];
        let key1_bytes = key1.encode_key(&mut key1_buf);
        let key2_bytes = key2.encode_key(&mut key2_buf);
        let value_bytes = value.encoded();

        self.queue_raw_put_dual(T::NAME, key1_bytes, key2_bytes, &value_bytes)
    }

    /// Queue a delete operation for a specific table.
    fn queue_delete<T: SingleKey>(&self, key: &T::Key) -> Result<(), Self::Error> {
        let mut key_buf = [0u8; MAX_KEY_SIZE];
        let key_bytes = key.encode_key(&mut key_buf);

        self.queue_raw_delete(T::NAME, key_bytes)
    }

    /// Queue a delete operation for a specific dual-keyed table.
    fn queue_delete_dual<T: DualKey>(
        &self,
        key1: &T::Key,
        key2: &T::Key2,
    ) -> Result<(), Self::Error> {
        let mut key1_buf = [0u8; MAX_KEY_SIZE];
        let mut key2_buf = [0u8; MAX_KEY_SIZE];
        let key1_bytes = key1.encode_key(&mut key1_buf);
        let key2_bytes = key2.encode_key(&mut key2_buf);

        self.queue_raw_delete_dual(T::NAME, key1_bytes, key2_bytes)
    }

    /// Queue many put operations for a specific table.
    fn queue_put_many<'a, 'b, T, I>(&self, entries: I) -> Result<(), Self::Error>
    where
        T: SingleKey,
        T::Key: 'a,
        T::Value: 'b,
        I: IntoIterator<Item = (&'a T::Key, &'b T::Value)>,
    {
        let mut key_buf = [0u8; MAX_KEY_SIZE];

        for (key, value) in entries {
            let key_bytes = key.encode_key(&mut key_buf);
            let value_bytes = value.encoded();

            self.queue_raw_put(T::NAME, key_bytes, &value_bytes)?;
        }

        Ok(())
    }

    /// Queue creation of a specific table.
    fn queue_create<T>(&self) -> Result<(), Self::Error>
    where
        T: Table,
    {
        self.queue_raw_create(T::NAME, T::DUAL_KEY_SIZE, T::FIXED_VAL_SIZE)
    }

    /// Queue clearing all entries in a specific table.
    fn queue_clear<T>(&self) -> Result<(), Self::Error>
    where
        T: Table,
    {
        self.queue_raw_clear(T::NAME)
    }

    /// Remove all data in the given range and return the removed keys.
    fn clear_with_op<T: SingleKey>(
        &self,
        range: RangeInclusive<T::Key>,
        mut op: impl FnMut(T::Key, T::Value),
    ) -> Result<(), Self::Error> {
        let mut cursor = self.traverse_mut::<T>()?;

        // Position cursor at first entry at or above range start
        let Some((key, value)) = cursor.lower_bound(range.start())? else {
            // No entries at or above range start
            return Ok(());
        };

        if !range.contains(&key) {
            // First entry is outside range
            return Ok(());
        }

        op(key, value);
        cursor.delete_current()?;

        // Iterate through remaining entries
        while let Some((key, value)) = cursor.read_next()? {
            if !range.contains(&key) {
                break;
            }
            op(key, value);
            cursor.delete_current()?;
        }

        Ok(())
    }

    /// Remove all data in the given range from the database.
    fn clear_range<T: SingleKey>(&self, range: RangeInclusive<T::Key>) -> Result<(), Self::Error> {
        self.clear_with_op::<T>(range, |_, _| {})
    }

    /// Remove all data in the given range and return the removed key-value
    /// pairs.
    fn take_range<T: SingleKey>(
        &self,
        range: RangeInclusive<T::Key>,
    ) -> Result<Vec<KeyValue<T>>, Self::Error> {
        let mut vec = Vec::new();
        self.clear_with_op::<T>(range, |key, value| vec.push((key, value)))?;
        Ok(vec)
    }

    /// Remove all dual-keyed data in the given range from the database.
    fn clear_range_dual_with_op<T: DualKey>(
        &self,
        range: RangeInclusive<(T::Key, T::Key2)>,
        mut op: impl FnMut(T::Key, T::Key2, T::Value),
    ) -> Result<(), Self::Error> {
        let mut cursor = self.traverse_dual_mut::<T>()?;

        let (start_k1, start_k2) = range.start();

        // Position at first entry at or above (range.start(), minimal_k2)
        let Some((k1, k2, value)) = cursor.next_dual_above(start_k1, start_k2)? else {
            // No entries at or above range start
            return Ok(());
        };

        // inline range contains to avoid moving k1,k2
        let (range_1, range_2) = range.start();
        if range_1 > &k1 || (range_1 == &k1 && range_2 > &k2) {
            // First entry is outside range
            return Ok(());
        }
        let (range_1, range_2) = range.end();
        if range_1 < &k1 || (range_1 == &k1 && range_2 < &k2) {
            // First entry is outside range
            return Ok(());
        }
        // end of inline range contains

        op(k1, k2, value);
        cursor.delete_current()?;

        // Iterate through all entries (both k1 and k2 changes)
        while let Some((k1, k2, value)) = cursor.next_k2()? {
            // inline range contains to avoid moving k1,k2
            let (range_1, range_2) = range.start();
            if range_1 > &k1 || (range_1 == &k1 && range_2 > &k2) {
                break;
            }
            let (range_1, range_2) = range.end();
            if range_1 < &k1 || (range_1 == &k1 && range_2 < &k2) {
                break;
            }
            // end of inline range contains
            op(k1, k2, value);
            cursor.delete_current()?;
        }

        Ok(())
    }

    /// Remove all dual-keyed data in the given k1,k2 range from the database.
    fn clear_range_dual<T: DualKey>(
        &self,
        range: RangeInclusive<(T::Key, T::Key2)>,
    ) -> Result<(), Self::Error> {
        self.clear_range_dual_with_op::<T>(range, |_, _, _| {})
    }

    /// Remove all dual-keyed data in the given k1,k2 range and return the
    /// removed key-key-value tuples.
    fn take_range_dual<T: DualKey>(
        &self,
        range: RangeInclusive<(T::Key, T::Key2)>,
    ) -> Result<Vec<DualKeyValue<T>>, Self::Error> {
        let mut vec = Vec::new();
        self.clear_range_dual_with_op::<T>(range, |k1, k2, value| {
            vec.push((k1, k2, value));
        })?;
        Ok(vec)
    }

    /// Commit the queued operations.
    fn raw_commit(self) -> Result<(), Self::Error>;
}
