//! Cursor wrapper for libmdbx-sys.

use std::{
    borrow::Cow,
    ops::{Deref, DerefMut},
};

use crate::hot::{
    MAX_FIXED_VAL_SIZE, MAX_KEY_SIZE,
    impls::mdbx::{DbInfo, MdbxError},
    model::{DualKeyTraverse, KvTraverse, KvTraverseMut, RawDualKeyValue, RawKeyValue, RawValue},
};
use dashmap::mapref::one::Ref;
use reth_libmdbx::{RO, RW, TransactionKind};

/// Read only Cursor.
pub type CursorRO<'a> = Cursor<'a, RO>;

/// Read write cursor.
pub type CursorRW<'a> = Cursor<'a, RW>;

/// Cursor wrapper to access KV items.
pub struct Cursor<'a, K: TransactionKind> {
    /// Inner `libmdbx` cursor.
    pub(crate) inner: reth_libmdbx::Cursor<K>,

    /// Database flags that were used to open the database.
    db_info: Ref<'a, &'static str, DbInfo>,

    /// Scratch buffer for key2 operations in DUPSORT tables.
    /// Sized to hold key2 + fixed value for DUP_FIXED tables.
    buf: [u8; MAX_KEY_SIZE + MAX_FIXED_VAL_SIZE],
}

impl<K: TransactionKind + std::fmt::Debug> std::fmt::Debug for Cursor<'_, K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let flag_names =
            self.db_info.flags().iter_names().map(|t| t.0).collect::<Vec<_>>().join("|");
        f.debug_struct("Cursor")
            .field("inner", &self.inner)
            .field("db_flags", &flag_names)
            .field("buf", &self.buf)
            .finish()
    }
}

impl<K: TransactionKind> Deref for Cursor<'_, K> {
    type Target = reth_libmdbx::Cursor<K>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<'a> DerefMut for Cursor<'a, RW> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<'a, K: TransactionKind> Cursor<'a, K> {
    /// Creates a new `Cursor` wrapping the given `libmdbx` cursor.
    pub const fn new(inner: reth_libmdbx::Cursor<K>, db: Ref<'a, &'static str, DbInfo>) -> Self {
        Self { inner, db_info: db, buf: [0u8; MAX_KEY_SIZE + MAX_FIXED_VAL_SIZE] }
    }

    /// Returns the database info for this cursor.
    pub fn db_info(&self) -> &DbInfo {
        &self.db_info
    }
}

impl<K> KvTraverse<MdbxError> for Cursor<'_, K>
where
    K: TransactionKind,
{
    fn first<'a>(&'a mut self) -> Result<Option<RawKeyValue<'a>>, MdbxError> {
        self.inner.first().map_err(MdbxError::Mdbx)
    }

    fn last<'a>(&'a mut self) -> Result<Option<RawKeyValue<'a>>, MdbxError> {
        self.inner.last().map_err(MdbxError::Mdbx)
    }

    fn exact<'a>(&'a mut self, key: &[u8]) -> Result<Option<RawValue<'a>>, MdbxError> {
        self.inner.set(key).map_err(MdbxError::Mdbx)
    }

    fn lower_bound<'a>(&'a mut self, key: &[u8]) -> Result<Option<RawKeyValue<'a>>, MdbxError> {
        self.inner.set_range(key).map_err(MdbxError::Mdbx)
    }

    fn read_next<'a>(&'a mut self) -> Result<Option<RawKeyValue<'a>>, MdbxError> {
        self.inner.next().map_err(MdbxError::Mdbx)
    }

    fn read_prev<'a>(&'a mut self) -> Result<Option<RawKeyValue<'a>>, MdbxError> {
        self.inner.prev().map_err(MdbxError::Mdbx)
    }
}

impl KvTraverseMut<MdbxError> for Cursor<'_, RW> {
    fn delete_current(&mut self) -> Result<(), MdbxError> {
        self.inner.del(Default::default()).map_err(MdbxError::Mdbx)
    }
}

/// Splits a [`Cow`] slice at the given index, preserving borrowed status.
///
/// When the input is `Cow::Borrowed`, both outputs will be `Cow::Borrowed`
/// referencing subslices of the original data. When the input is `Cow::Owned`,
/// both outputs will be `Cow::Owned` with newly allocated vectors.
#[inline]
fn split_cow_at(cow: Cow<'_, [u8]>, at: usize) -> (Cow<'_, [u8]>, Cow<'_, [u8]>) {
    match cow {
        Cow::Borrowed(slice) => (Cow::Borrowed(&slice[..at]), Cow::Borrowed(&slice[at..])),
        Cow::Owned(mut vec) => {
            let right = vec.split_off(at);
            (Cow::Owned(vec), Cow::Owned(right))
        }
    }
}

impl<K> DualKeyTraverse<MdbxError> for Cursor<'_, K>
where
    K: TransactionKind,
{
    fn first<'a>(&'a mut self) -> Result<Option<RawDualKeyValue<'a>>, MdbxError> {
        if !self.db_info.is_dupsort() {
            return Err(MdbxError::NotDupSort);
        }

        match self.inner.first::<Cow<'_, [u8]>, Cow<'_, [u8]>>()? {
            Some((k1, v)) => {
                // For DUPSORT, the value contains key2 || actual_value.
                let Some(key2_size) = self.db_info.dup_fixed_val_size().key2_size() else {
                    return Err(MdbxError::UnknownFixedSize);
                };
                let (k2, val) = split_cow_at(v, key2_size);
                Ok(Some((k1, k2, val)))
            }
            None => Ok(None),
        }
    }

    fn last<'a>(&'a mut self) -> Result<Option<RawDualKeyValue<'a>>, MdbxError> {
        if !self.db_info.is_dupsort() {
            return Err(MdbxError::NotDupSort);
        }

        match self.inner.last::<Cow<'_, [u8]>, Cow<'_, [u8]>>()? {
            Some((k1, v)) => {
                // For DUPSORT, the value contains key2 || actual_value.
                let Some(key2_size) = self.db_info.dup_fixed_val_size().key2_size() else {
                    return Err(MdbxError::UnknownFixedSize);
                };
                let (k2, val) = split_cow_at(v, key2_size);
                Ok(Some((k1, k2, val)))
            }
            None => Ok(None),
        }
    }

    fn read_next<'a>(&'a mut self) -> Result<Option<RawDualKeyValue<'a>>, MdbxError> {
        if !self.db_info.is_dupsort() {
            return Err(MdbxError::NotDupSort);
        }

        match self.inner.next::<Cow<'_, [u8]>, Cow<'_, [u8]>>()? {
            Some((k1, v)) => {
                // For DUPSORT, the value contains key2 || actual_value.
                let Some(key2_size) = self.db_info.dup_fixed_val_size().key2_size() else {
                    return Err(MdbxError::UnknownFixedSize);
                };
                let (k2, val) = split_cow_at(v, key2_size);
                Ok(Some((k1, k2, val)))
            }
            None => Ok(None),
        }
    }

    fn read_prev<'a>(&'a mut self) -> Result<Option<RawDualKeyValue<'a>>, MdbxError> {
        if !self.db_info.is_dupsort() {
            return Err(MdbxError::NotDupSort);
        }

        match self.inner.prev::<Cow<'_, [u8]>, Cow<'_, [u8]>>()? {
            Some((k1, v)) => {
                // For DUPSORT, the value contains key2 || actual_value.
                let Some(key2_size) = self.db_info.dup_fixed_val_size().key2_size() else {
                    return Err(MdbxError::UnknownFixedSize);
                };
                let (k2, val) = split_cow_at(v, key2_size);
                Ok(Some((k1, k2, val)))
            }
            None => Ok(None),
        }
    }

    fn exact_dual<'a>(
        &'a mut self,
        key1: &[u8],
        key2: &[u8],
    ) -> Result<Option<RawValue<'a>>, MdbxError> {
        if !self.db_info.is_dupsort() {
            return Err(MdbxError::NotDupSort);
        }

        // For DUPSORT tables, we use get_both which finds exact (key1, key2) match.
        // The "value" in MDBX DUPSORT is key2 || actual_value, so we return that.
        // Prepare key2 (may need padding for DUP_FIXED)
        let fsi = self.db_info.dup_fixed_val_size();
        let key2_prepared = if let Some(total_size) = fsi.total_size() {
            // Copy key2 to scratch buffer and zero-pad to total fixed size
            self.buf[..key2.len()].copy_from_slice(key2);
            self.buf[key2.len()..total_size].fill(0);
            &self.buf[..total_size]
        } else {
            key2
        };
        self.inner.get_both(key1, key2_prepared).map_err(MdbxError::Mdbx)
    }

    fn next_dual_above<'a>(
        &'a mut self,
        key1: &[u8],
        key2: &[u8],
    ) -> Result<Option<RawDualKeyValue<'a>>, MdbxError> {
        if !self.db_info.is_dupsort() {
            return Err(MdbxError::NotDupSort);
        }

        let fsi = self.db_info.dup_fixed_val_size();
        let key2_size = fsi.key2_size().unwrap_or(key2.len());

        // Use set_range to find the first entry with key1 >= search_key1
        let Some((found_k1, v)) = self.inner.set_range::<Cow<'_, [u8]>, Cow<'_, [u8]>>(key1)?
        else {
            return Ok(None);
        };

        // If found_k1 > search_key1, we have our answer (first entry in next key1)
        if found_k1.as_ref() > key1 {
            let (k2, val) = split_cow_at(v, key2_size);
            return Ok(Some((found_k1, k2, val)));
        }

        // found_k1 == search_key1, so we need to filter by key2 >= search_key2
        // Use get_both_range to find entry with exact key1 and value >= key2
        let key2_prepared = if let Some(total_size) = fsi.total_size() {
            // Copy key2 to scratch buffer and zero-pad to total fixed size
            self.buf[..key2.len()].copy_from_slice(key2);
            self.buf[key2.len()..total_size].fill(0);
            &self.buf[..total_size]
        } else {
            key2
        };

        match self.inner.get_both_range::<RawValue<'_>>(key1, key2_prepared)? {
            Some(v) => {
                let (k2, val) = split_cow_at(v, key2_size);
                // key1 must be owned here since we're returning a reference to the input
                Ok(Some((Cow::Owned(key1.to_vec()), k2, val)))
            }
            None => {
                // No entry with key2 >= search_key2 in this key1, try next key1
                match self.inner.next_nodup::<Cow<'_, [u8]>, Cow<'_, [u8]>>()? {
                    Some((k1, v)) => {
                        let (k2, val) = split_cow_at(v, key2_size);
                        Ok(Some((k1, k2, val)))
                    }
                    None => Ok(None),
                }
            }
        }
    }

    fn next_k1<'a>(&'a mut self) -> Result<Option<RawDualKeyValue<'a>>, MdbxError> {
        // Move to the next distinct key1 (skip remaining duplicates for current key1)
        if self.db_info.is_dupsort() {
            match self.inner.next_nodup::<Cow<'_, [u8]>, Cow<'_, [u8]>>()? {
                Some((k1, v)) => {
                    // For DUPSORT, the value contains key2 || actual_value.
                    // Split using the known key2 size.
                    let Some(key2_size) = self.db_info.dup_fixed_val_size().key2_size() else {
                        return Err(MdbxError::UnknownFixedSize);
                    };
                    let (k2, val) = split_cow_at(v, key2_size);
                    Ok(Some((k1, k2, val)))
                }
                None => Ok(None),
            }
        } else {
            // Not a DUPSORT table - just get next entry
            match self.inner.next()? {
                Some((k, v)) => Ok(Some((k, Cow::Borrowed(&[] as &[u8]), v))),
                None => Ok(None),
            }
        }
    }

    fn next_k2<'a>(&'a mut self) -> Result<Option<RawDualKeyValue<'a>>, MdbxError> {
        // Move to the next duplicate (same key1, next key2)
        if self.db_info.is_dupsort() {
            match self.inner.next_dup::<Cow<'_, [u8]>, Cow<'_, [u8]>>()? {
                Some((k1, v)) => {
                    // For DUPSORT, the value contains key2 || actual_value.
                    // Split using the known key2 size.
                    let Some(key2_size) = self.db_info.dup_fixed_val_size().key2_size() else {
                        return Err(MdbxError::UnknownFixedSize);
                    };
                    let (k2, val) = split_cow_at(v, key2_size);
                    Ok(Some((k1, k2, val)))
                }
                None => Ok(None),
            }
        } else {
            // Not a DUPSORT table - no concept of "next duplicate"
            Ok(None)
        }
    }

    fn last_of_k1<'a>(&'a mut self, key1: &[u8]) -> Result<Option<RawDualKeyValue<'a>>, MdbxError> {
        if !self.db_info.is_dupsort() {
            return Err(MdbxError::NotDupSort);
        }

        // First, position at key1 (any duplicate)
        let Some(_) = self.inner.set::<Cow<'_, [u8]>>(key1)? else {
            return Ok(None);
        };

        // Then move to the last duplicate for this key1
        let Some(v) = self.inner.last_dup::<Cow<'_, [u8]>>()? else {
            return Ok(None);
        };

        // Split the value into key2 and actual value
        let Some(key2_size) = self.db_info.dup_fixed_val_size().key2_size() else {
            return Err(MdbxError::UnknownFixedSize);
        };
        let (k2, val) = split_cow_at(v, key2_size);

        // key1 must be owned here since we're returning a reference to the input
        Ok(Some((Cow::Owned(key1.to_vec()), k2, val)))
    }

    fn previous_k1<'a>(&'a mut self) -> Result<Option<RawDualKeyValue<'a>>, MdbxError> {
        if !self.db_info.is_dupsort() {
            return Err(MdbxError::NotDupSort);
        }

        // prev_nodup positions at the last data item of the previous key
        match self.inner.prev_nodup::<Cow<'_, [u8]>, Cow<'_, [u8]>>()? {
            Some((k1, v)) => {
                // For DUPSORT, prev_nodup already positions at the last duplicate
                // of the previous key. Split the value.
                let Some(key2_size) = self.db_info.dup_fixed_val_size().key2_size() else {
                    return Err(MdbxError::UnknownFixedSize);
                };
                let (k2, val) = split_cow_at(v, key2_size);
                Ok(Some((k1, k2, val)))
            }
            None => Ok(None),
        }
    }

    fn previous_k2<'a>(&'a mut self) -> Result<Option<RawDualKeyValue<'a>>, MdbxError> {
        if !self.db_info.is_dupsort() {
            return Err(MdbxError::NotDupSort);
        }

        // prev_dup positions at the previous duplicate of the current key
        match self.inner.prev_dup::<Cow<'_, [u8]>, Cow<'_, [u8]>>()? {
            Some((k1, v)) => {
                let Some(key2_size) = self.db_info.dup_fixed_val_size().key2_size() else {
                    return Err(MdbxError::UnknownFixedSize);
                };
                let (k2, val) = split_cow_at(v, key2_size);
                Ok(Some((k1, k2, val)))
            }
            None => Ok(None),
        }
    }
}
