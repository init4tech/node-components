use std::borrow::Cow;

use crate::{
    hot::{HotKvError, HotKvReadError},
    ser::{KeySer, MAX_KEY_SIZE, ValSer},
    tables::Table,
};

/// Trait for hot storage. This is a KV store with read/write transactions.
pub trait HotKv {
    /// The read-only transaction type.
    type RoTx: HotKvRead;
    /// The read-write transaction type.
    type RwTx: HotKvWrite;

    /// Create a read-only transaction.
    fn reader(&self) -> Result<Self::RoTx, HotKvError>;

    /// Create a read-write transaction.
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
}

/// Trait for hot storage read transactions.
pub trait HotKvRead {
    /// Error type for read operations.
    type Error: HotKvReadError;

    /// Get a raw value from a specific table.
    ///
    /// The `key` buf must be <= [`MAX_KEY_SIZE`] bytes. Implementations are
    /// allowed to panic if this is not the case.
    fn get_raw<'a>(&'a self, table: &str, key: &[u8])
    -> Result<Option<Cow<'a, [u8]>>, Self::Error>;

    /// Get a value from a specific table.
    fn get<T: Table>(&self, key: &T::Key) -> Result<Option<T::Value>, Self::Error> {
        let mut key_buf = [0u8; MAX_KEY_SIZE];
        let key_bytes = key.encode_key(&mut key_buf);
        debug_assert!(
            key_bytes.len() == T::Key::SIZE,
            "Encoded key length does not match expected size"
        );

        let Some(value_bytes) = self.get_raw(T::NAME, key_bytes)? else {
            return Ok(None);
        };
        let data = &value_bytes[..];
        T::Value::decode_value(data).map(Some).map_err(Into::into)
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
            .map(|key| self.get_raw(T::NAME, key.encode_key(&mut key_buf)))
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

    /// Queue a raw delete operation.
    ///
    /// The `key` buf must be <= [`MAX_KEY_SIZE`] bytes. Implementations are
    /// allowed to panic if this is not the case.
    fn queue_raw_delete(&mut self, table: &str, key: &[u8]) -> Result<(), Self::Error>;

    /// Queue a put operation for a specific table.
    fn queue_put<T: Table>(&mut self, key: &T::Key, value: &T::Value) -> Result<(), Self::Error> {
        let mut key_buf = [0u8; MAX_KEY_SIZE];
        let key_bytes = key.encode_key(&mut key_buf);
        let value_bytes = value.encoded();

        self.queue_raw_put(T::NAME, key_bytes, &value_bytes)
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

    /// Commit the queued operations.
    fn raw_commit(self) -> Result<(), Self::Error>;
}
