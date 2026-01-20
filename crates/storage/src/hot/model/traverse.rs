//! Cursor traversal traits and typed wrappers for database navigation.

use crate::hot::{
    model::{DualKeyValue, HotKvReadError, KeyValue, RawDualKeyValue, RawKeyValue, RawValue},
    ser::{KeySer, MAX_KEY_SIZE},
    tables::{DualKey, Table},
};
use std::ops::Range;

/// Trait for traversing key-value pairs in the database.
pub trait KvTraverse<E: HotKvReadError> {
    /// Set position to the first key-value pair in the database, and return
    /// the KV pair.
    fn first<'a>(&'a mut self) -> Result<Option<RawKeyValue<'a>>, E>;

    /// Set position to the last key-value pair in the database, and return the
    /// KV pair.
    fn last<'a>(&'a mut self) -> Result<Option<RawKeyValue<'a>>, E>;

    /// Set the cursor to specific key in the database, and return the EXACT KV
    /// pair if it exists.
    fn exact<'a>(&'a mut self, key: &[u8]) -> Result<Option<RawValue<'a>>, E>;

    /// Seek to the next key-value pair AT OR ABOVE the specified key in the
    /// database, and return that KV pair.
    fn lower_bound<'a>(&'a mut self, key: &[u8]) -> Result<Option<RawKeyValue<'a>>, E>;

    /// Get the next key-value pair in the database, and advance the cursor.
    ///
    /// Returning `Ok(None)` indicates the cursor is past the end of the
    /// database.
    fn read_next<'a>(&'a mut self) -> Result<Option<RawKeyValue<'a>>, E>;

    /// Get the previous key-value pair in the database, and move the cursor.
    ///
    /// Returning `Ok(None)` indicates the cursor is before the start of the
    /// database.
    fn read_prev<'a>(&'a mut self) -> Result<Option<RawKeyValue<'a>>, E>;
}

/// Trait for traversing key-value pairs in the database with mutation
/// capabilities.
pub trait KvTraverseMut<E: HotKvReadError>: KvTraverse<E> {
    /// Delete the current key-value pair in the database.
    fn delete_current(&mut self) -> Result<(), E>;

    /// Delete a range of key-value pairs in the database, from `start_key`
    fn delete_range(&mut self, range: Range<&[u8]>) -> Result<(), E> {
        let _ = self.exact(range.start)?;
        while let Some((key, _value)) = self.read_next()? {
            if key.as_ref() >= range.end {
                break;
            }
            self.delete_current()?;
        }
        Ok(())
    }
}

/// Trait for traversing dual-keyed key-value pairs in the database.
pub trait DualKeyTraverse<E: HotKvReadError>: KvTraverse<E> {
    /// Set the cursor to specific dual key in the database, and return the
    /// EXACT KV pair if it exists.
    ///
    /// Returning `Ok(None)` indicates the exact dual key does not exist.
    fn exact_dual<'a>(&'a mut self, key1: &[u8], key2: &[u8]) -> Result<Option<RawValue<'a>>, E>;

    /// Seek to the next key-value pair AT or ABOVE the specified dual key in
    /// the database, and return that KV pair.
    ///
    /// Returning `Ok(None)` indicates there are no more key-value pairs above
    /// the specified dual key.
    fn next_dual_above<'a>(
        &'a mut self,
        key1: &[u8],
        key2: &[u8],
    ) -> Result<Option<RawDualKeyValue<'a>>, E>;

    /// Move the cursor to the next distinct key1, and return the first
    /// key-value pair with that key1.
    ///
    /// Returning `Ok(None)` indicates there are no more distinct key1 values.
    fn next_k1<'a>(&'a mut self) -> Result<Option<RawDualKeyValue<'a>>, E>;

    /// Move the cursor to the next distinct key2 for the current key1, and
    /// return the first key-value pair with that key2.
    fn next_k2<'a>(&'a mut self) -> Result<Option<RawDualKeyValue<'a>>, E>;
}

// ============================================================================
// Typed Extension Traits
// ============================================================================

/// Extension trait for typed table traversal.
///
/// This trait provides type-safe access to table entries by encoding keys
/// and decoding values according to the table's schema.
pub trait TableTraverse<T: Table, E: HotKvReadError>: KvTraverse<E> {
    /// Get the first key-value pair in the table.
    fn first(&mut self) -> Result<Option<KeyValue<T>>, E> {
        KvTraverse::first(self)?.map(T::decode_kv_tuple).transpose().map_err(Into::into)
    }

    /// Get the last key-value pair in the table.
    fn last(&mut self) -> Result<Option<KeyValue<T>>, E> {
        KvTraverse::last(self)?.map(T::decode_kv_tuple).transpose().map_err(Into::into)
    }

    /// Set the cursor to a specific key and return the EXACT value if it exists.
    fn exact(&mut self, key: &T::Key) -> Result<Option<T::Value>, E> {
        let mut key_buf = [0u8; MAX_KEY_SIZE];
        let key_bytes = key.encode_key(&mut key_buf);

        KvTraverse::exact(self, key_bytes)?.map(T::decode_value).transpose().map_err(Into::into)
    }

    /// Seek to the next key-value pair AT OR ABOVE the specified key.
    fn lower_bound(&mut self, key: &T::Key) -> Result<Option<KeyValue<T>>, E> {
        let mut key_buf = [0u8; MAX_KEY_SIZE];
        let key_bytes = key.encode_key(&mut key_buf);

        KvTraverse::lower_bound(self, key_bytes)?
            .map(T::decode_kv_tuple)
            .transpose()
            .map_err(Into::into)
    }

    /// Get the next key-value pair and advance the cursor.
    fn read_next(&mut self) -> Result<Option<KeyValue<T>>, E> {
        KvTraverse::read_next(self)?.map(T::decode_kv_tuple).transpose().map_err(Into::into)
    }

    /// Get the previous key-value pair and move the cursor backward.
    fn read_prev(&mut self) -> Result<Option<KeyValue<T>>, E> {
        KvTraverse::read_prev(self)?.map(T::decode_kv_tuple).transpose().map_err(Into::into)
    }
}

/// Blanket implementation of `TableTraverse` for any cursor that implements `KvTraverse`.
impl<C, T, E> TableTraverse<T, E> for C
where
    C: KvTraverse<E>,
    T: Table,
    E: HotKvReadError,
{
}

/// Extension trait for typed table traversal with mutation capabilities.
pub trait TableTraverseMut<T: Table, E: HotKvReadError>: KvTraverseMut<E> {
    /// Delete the current key-value pair.
    fn delete_current(&mut self) -> Result<(), E> {
        KvTraverseMut::delete_current(self)
    }

    /// Delete a range of key-value pairs.
    fn delete_range(&mut self, range: Range<T::Key>) -> Result<(), E> {
        let mut start_key_buf = [0u8; MAX_KEY_SIZE];
        let mut end_key_buf = [0u8; MAX_KEY_SIZE];
        let start_key_bytes = range.start.encode_key(&mut start_key_buf);
        let end_key_bytes = range.end.encode_key(&mut end_key_buf);

        KvTraverseMut::delete_range(self, start_key_bytes..end_key_bytes)
    }
}

/// Blanket implementation of `TableTraverseMut` for any cursor that implements `KvTraverseMut`.
impl<C, T, E> TableTraverseMut<T, E> for C
where
    C: KvTraverseMut<E>,
    T: Table,
    E: HotKvReadError,
{
}

/// A typed cursor wrapper for traversing dual-keyed tables.
///
/// This is an extension trait rather than a wrapper struct because MDBX
/// requires specialized implementations for DUPSORT tables that need access
/// to the table type `T` to handle fixed-size values correctly.
pub trait DualTableTraverse<T: DualKey, E: HotKvReadError> {
    /// Return the EXACT value for the specified dual key if it exists.
    fn exact_dual(&mut self, key1: &T::Key, key2: &T::Key2) -> Result<Option<T::Value>, E> {
        let Some((k1, k2, v)) = self.next_dual_above(key1, key2)? else {
            return Ok(None);
        };

        if k1 == *key1 && k2 == *key2 { Ok(Some(v)) } else { Ok(None) }
    }

    /// Seek to the next key-value pair AT or ABOVE the specified dual key.
    fn next_dual_above(
        &mut self,
        key1: &T::Key,
        key2: &T::Key2,
    ) -> Result<Option<DualKeyValue<T>>, E>;

    /// Seek to the next distinct key1, and return the first key-value pair with that key1.
    fn next_k1(&mut self) -> Result<Option<DualKeyValue<T>>, E>;

    /// Seek to the next distinct key2 for the current key1.
    fn next_k2(&mut self) -> Result<Option<DualKeyValue<T>>, E>;
}

// ============================================================================
// Wrapper Structs
// ============================================================================

use core::marker::PhantomData;

/// A wrapper struct for typed table traversal.
///
/// This struct wraps a raw cursor and provides type-safe access to table
/// entries. It implements `TableTraverse<T, E>` by delegating to the inner
/// cursor.
#[derive(Debug)]
pub struct TableCursor<C, T, E> {
    inner: C,
    _marker: PhantomData<fn() -> (T, E)>,
}

impl<C, T, E> TableCursor<C, T, E> {
    /// Create a new typed table cursor wrapper.
    pub const fn new(cursor: C) -> Self {
        Self { inner: cursor, _marker: PhantomData }
    }

    /// Get a reference to the inner cursor.
    pub const fn inner(&self) -> &C {
        &self.inner
    }

    /// Get a mutable reference to the inner cursor.
    pub const fn inner_mut(&mut self) -> &mut C {
        &mut self.inner
    }

    /// Consume the wrapper and return the inner cursor.
    pub fn into_inner(self) -> C {
        self.inner
    }
}

impl<C, T, E> TableCursor<C, T, E>
where
    C: KvTraverse<E>,
    T: Table,
    E: HotKvReadError,
{
    /// Get the first key-value pair in the table.
    pub fn first(&mut self) -> Result<Option<KeyValue<T>>, E> {
        TableTraverse::<T, E>::first(&mut self.inner)
    }

    /// Get the last key-value pair in the table.
    pub fn last(&mut self) -> Result<Option<KeyValue<T>>, E> {
        TableTraverse::<T, E>::last(&mut self.inner)
    }

    /// Set the cursor to a specific key and return the EXACT value if it exists.
    pub fn exact(&mut self, key: &T::Key) -> Result<Option<T::Value>, E> {
        TableTraverse::<T, E>::exact(&mut self.inner, key)
    }

    /// Seek to the next key-value pair AT OR ABOVE the specified key.
    pub fn lower_bound(&mut self, key: &T::Key) -> Result<Option<KeyValue<T>>, E> {
        TableTraverse::<T, E>::lower_bound(&mut self.inner, key)
    }

    /// Get the next key-value pair and advance the cursor.
    pub fn read_next(&mut self) -> Result<Option<KeyValue<T>>, E> {
        TableTraverse::<T, E>::read_next(&mut self.inner)
    }

    /// Get the previous key-value pair and move the cursor backward.
    pub fn read_prev(&mut self) -> Result<Option<KeyValue<T>>, E> {
        TableTraverse::<T, E>::read_prev(&mut self.inner)
    }
}

impl<C, T, E> TableCursor<C, T, E>
where
    C: KvTraverseMut<E>,
    T: Table,
    E: HotKvReadError,
{
    /// Delete the current key-value pair.
    pub fn delete_current(&mut self) -> Result<(), E> {
        TableTraverseMut::<T, E>::delete_current(&mut self.inner)
    }

    /// Delete a range of key-value pairs.
    pub fn delete_range(&mut self, range: Range<T::Key>) -> Result<(), E> {
        TableTraverseMut::<T, E>::delete_range(&mut self.inner, range)
    }
}

/// A wrapper struct for typed dual-keyed table traversal.
///
/// This struct wraps a raw cursor and provides type-safe access to dual-keyed
/// table entries. It delegates to the `DualTableTraverse<T, E>` trait
/// implementation on the inner cursor.
#[derive(Debug)]
pub struct DualTableCursor<C, T, E> {
    inner: C,
    _marker: PhantomData<fn() -> (T, E)>,
}

impl<C, T, E> DualTableCursor<C, T, E> {
    /// Create a new typed dual-keyed table cursor wrapper.
    pub const fn new(cursor: C) -> Self {
        Self { inner: cursor, _marker: PhantomData }
    }

    /// Get a reference to the inner cursor.
    pub const fn inner(&self) -> &C {
        &self.inner
    }

    /// Get a mutable reference to the inner cursor.
    pub const fn inner_mut(&mut self) -> &mut C {
        &mut self.inner
    }

    /// Consume the wrapper and return the inner cursor.
    pub fn into_inner(self) -> C {
        self.inner
    }
}

impl<C, T, E> DualTableCursor<C, T, E>
where
    C: DualTableTraverse<T, E>,
    T: DualKey,
    E: HotKvReadError,
{
    /// Return the EXACT value for the specified dual key if it exists.
    pub fn exact_dual(&mut self, key1: &T::Key, key2: &T::Key2) -> Result<Option<T::Value>, E> {
        DualTableTraverse::<T, E>::exact_dual(&mut self.inner, key1, key2)
    }

    /// Seek to the next key-value pair AT or ABOVE the specified dual key.
    pub fn next_dual_above(
        &mut self,
        key1: &T::Key,
        key2: &T::Key2,
    ) -> Result<Option<DualKeyValue<T>>, E> {
        DualTableTraverse::<T, E>::next_dual_above(&mut self.inner, key1, key2)
    }

    /// Seek to the next distinct key1, and return the first key-value pair with that key1.
    pub fn next_k1(&mut self) -> Result<Option<DualKeyValue<T>>, E> {
        DualTableTraverse::<T, E>::next_k1(&mut self.inner)
    }

    /// Seek to the next distinct key2 for the current key1.
    pub fn next_k2(&mut self) -> Result<Option<DualKeyValue<T>>, E> {
        DualTableTraverse::<T, E>::next_k2(&mut self.inner)
    }
}

// Also provide access to single-key traversal methods for dual-keyed cursors
impl<C, T, E> DualTableCursor<C, T, E>
where
    C: KvTraverse<E>,
    T: DualKey,
    E: HotKvReadError,
{
    /// Get the first key-value pair in the table (raw traversal).
    pub fn first(&mut self) -> Result<Option<KeyValue<T>>, E> {
        TableTraverse::<T, E>::first(&mut self.inner)
    }

    /// Get the last key-value pair in the table (raw traversal).
    pub fn last(&mut self) -> Result<Option<KeyValue<T>>, E> {
        TableTraverse::<T, E>::last(&mut self.inner)
    }

    /// Get the next key-value pair and advance the cursor.
    pub fn read_next(&mut self) -> Result<Option<KeyValue<T>>, E> {
        TableTraverse::<T, E>::read_next(&mut self.inner)
    }

    /// Get the previous key-value pair and move the cursor backward.
    pub fn read_prev(&mut self) -> Result<Option<KeyValue<T>>, E> {
        TableTraverse::<T, E>::read_prev(&mut self.inner)
    }
}

impl<C, T, E> DualTableCursor<C, T, E>
where
    C: KvTraverseMut<E>,
    T: DualKey,
    E: HotKvReadError,
{
    /// Delete the current key-value pair.
    pub fn delete_current(&mut self) -> Result<(), E> {
        TableTraverseMut::<T, E>::delete_current(&mut self.inner)
    }
}
