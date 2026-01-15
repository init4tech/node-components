use std::borrow::Cow;

use crate::{
    hot::{
        HotKvError, HotKvReadError,
        revm::{RevmRead, RevmWrite},
    },
    ser::{KeySer, MAX_KEY_SIZE, ValSer},
    tables::{DualKeyed, Table},
};

/// Trait for hot storage. This is a KV store with read/write transactions.
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
#[auto_impl::auto_impl(&, Arc, Box)]
pub trait HotKvRead {
    /// Error type for read operations.
    type Error: HotKvReadError;

    /// Get a raw value from a specific table.
    ///
    /// The `key` buf must be <= [`MAX_KEY_SIZE`] bytes. Implementations are
    /// allowed to panic if this is not the case.
    ///
    /// If the table is dual-keyed, the output may be implementation-defined.
    fn raw_get<'a>(&'a self, table: &str, key: &[u8])
    -> Result<Option<Cow<'a, [u8]>>, Self::Error>;

    /// Get a raw value from a specific table with dual keys.
    ///
    /// If the table is not dual-keyed, the output may be
    /// implementation-defined.
    fn raw_get_dual<'a>(
        &'a self,
        table: &str,
        key1: &[u8],
        key2: &[u8],
    ) -> Result<Option<Cow<'a, [u8]>>, Self::Error>;

    /// Get a value from a specific table.
    fn get<T: Table>(&self, key: &T::Key) -> Result<Option<T::Value>, Self::Error> {
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
    fn get_dual<T: DualKeyed>(
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
    /// A vector of `Option<T::Value>`, where each element corresponds to the
    /// value for the respective key in the input iterator. If a key does not
    /// exist in the table, the corresponding element will be `None`.
    ///
    /// If any error occurs during retrieval or deserialization, the entire
    /// operation will return an error.
    fn get_many<'a, T, I>(&self, keys: I) -> Result<Vec<Option<T::Value>>, Self::Error>
    where
        T::Key: 'a,
        T: Table,
        I: IntoIterator<Item = &'a T::Key>,
    {
        let mut key_buf = [0u8; MAX_KEY_SIZE];

        keys.into_iter()
            .map(|key| self.raw_get(T::NAME, key.encode_key(&mut key_buf)))
            .map(|maybe_val| {
                maybe_val
                    .and_then(|val| ValSer::maybe_decode_value(val.as_deref()).map_err(Into::into))
            })
            .collect()
    }
}

/// Trait for hot storage write transactions.
pub trait HotKvWrite: HotKvRead {
    /// Queue a raw put operation.
    ///
    /// The `key` buf must be <= [`MAX_KEY_SIZE`] bytes. Implementations are
    /// allowed to panic if this is not the case.
    fn queue_raw_put(&mut self, table: &str, key: &[u8], value: &[u8]) -> Result<(), Self::Error>;

    /// Queue a raw put operation for a dual-keyed table.
    ////
    /// The `key1` and `key2` buf must be <= [`MAX_KEY_SIZE`] bytes.
    /// Implementations are allowed to panic if this is not the case.
    fn queue_raw_put_dual(
        &mut self,
        table: &str,
        key1: &[u8],
        key2: &[u8],
        value: &[u8],
    ) -> Result<(), Self::Error>;

    /// Queue a raw delete operation.
    ///
    /// The `key` buf must be <= [`MAX_KEY_SIZE`] bytes. Implementations are
    /// allowed to panic if this is not the case.
    fn queue_raw_delete(&mut self, table: &str, key: &[u8]) -> Result<(), Self::Error>;

    /// Queue a raw clear operation for a specific table.
    fn queue_raw_clear(&mut self, table: &str) -> Result<(), Self::Error>;

    /// Queue a raw create operation for a specific table.
    ///
    /// This abstraction supports two table specializations:
    /// 1. `dual_key`: whether the table uses dual keys (interior maps, called
    ///    `DUPSORT` in LMDB/MDBX).
    /// 2. `fixed_val`: whether the table has fixed-size values.
    ///
    /// Database implementations can use this information for optimizations.
    fn queue_raw_create(
        &mut self,
        table: &str,
        dual_key: bool,
        fixed_val: bool,
    ) -> Result<(), Self::Error>;

    /// Queue a put operation for a specific table.
    fn queue_put<T: Table>(&mut self, key: &T::Key, value: &T::Value) -> Result<(), Self::Error> {
        let mut key_buf = [0u8; MAX_KEY_SIZE];
        let key_bytes = key.encode_key(&mut key_buf);
        let value_bytes = value.encoded();

        self.queue_raw_put(T::NAME, key_bytes, &value_bytes)
    }

    /// Queue a put operation for a specific dual-keyed table.
    fn queue_put_dual<T: DualKeyed>(
        &mut self,
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
    fn queue_delete<T: Table>(&mut self, key: &T::Key) -> Result<(), Self::Error> {
        let mut key_buf = [0u8; MAX_KEY_SIZE];
        let key_bytes = key.encode_key(&mut key_buf);

        self.queue_raw_delete(T::NAME, key_bytes)
    }

    /// Queue many put operations for a specific table.
    fn queue_put_many<'a, 'b, T, I>(&mut self, entries: I) -> Result<(), Self::Error>
    where
        T: Table,
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
    fn queue_create<T>(&mut self) -> Result<(), Self::Error>
    where
        T: Table,
    {
        self.queue_raw_create(T::NAME, T::DUAL_KEY, T::IS_FIXED_VAL)
    }

    /// Queue clearing all entries in a specific table.
    fn queue_clear<T>(&mut self) -> Result<(), Self::Error>
    where
        T: Table,
    {
        self.queue_raw_clear(T::NAME)
    }

    /// Commit the queued operations.
    fn raw_commit(self) -> Result<(), Self::Error>;
}
