//! Transaction wrapper for libmdbx-sys.
use crate::hot::{
    KeySer, MAX_FIXED_VAL_SIZE, MAX_KEY_SIZE, ValSer,
    impls::mdbx::{Cursor, DbCache, DbInfo, FixedSizeInfo, MdbxError},
    model::{DualTableTraverse, HotKvRead, HotKvWrite},
    tables::{DualKey, SingleKey, Table},
};
use alloy::primitives::B256;
use dashmap::mapref::one::Ref;
use reth_libmdbx::{DatabaseFlags, RW, Transaction, TransactionKind, WriteFlags};
use std::borrow::Cow;

const TX_BUFFER_SIZE: usize = MAX_KEY_SIZE + MAX_FIXED_VAL_SIZE;

/// Wrapper for the libmdbx transaction.
#[derive(Debug)]
pub struct Tx<K: TransactionKind> {
    /// Libmdbx-sys transaction.
    pub inner: Transaction<K>,

    /// Cached MDBX DBIs for reuse.
    dbs: DbCache,
}

impl<K: TransactionKind> Tx<K> {
    /// Creates new `Tx` object with a `RO` or `RW` transaction and optionally enables metrics.
    #[inline]
    pub(crate) const fn new(inner: Transaction<K>, dbis: DbCache) -> Self {
        Self { inner, dbs: dbis }
    }

    /// Gets the database handle for the DbInfo table.
    fn db_info_table_dbi(&self) -> Result<u32, MdbxError> {
        self.inner.open_db(None).map(|db| db.dbi()).map_err(MdbxError::Mdbx)
    }

    fn read_db_info_table(&self, name: &'static str) -> Result<DbInfo, MdbxError> {
        let mut key = B256::ZERO;
        let to_copy = core::cmp::min(32, name.len());
        key[..to_copy].copy_from_slice(&name.as_bytes()[..to_copy]);

        let db_info_dbi = self.db_info_table_dbi()?;
        self.inner
            .get::<Cow<'_, [u8]>>(db_info_dbi, key.as_slice())?
            .as_deref()
            .map(DbInfo::decode_value)
            .transpose()
            .map_err(MdbxError::Deser)?
            .ok_or(MdbxError::UnknownTable(name))
    }

    /// Cache the database info for a specific table by name.
    pub fn cache_db_info_raw(
        &self,
        table: &'static str,
    ) -> Result<Ref<'_, &'static str, DbInfo>, MdbxError> {
        if let Some(info) = self.dbs.get(table) {
            return Ok(info);
        }

        let db_info = self.read_db_info_table(table)?;

        self.dbs.insert(table, db_info);
        Ok(self.dbs.get(table).expect("Just inserted"))
    }

    /// Caches the database info for a specific table.
    pub fn cache_db_info<T: Table>(&self) -> Result<Ref<'_, &'static str, DbInfo>, MdbxError> {
        self.cache_db_info_raw(T::NAME)
    }

    /// Gets the database handle for the given table name.
    pub fn get_dbi_raw(&self, table: &'static str) -> Result<u32, MdbxError> {
        self.cache_db_info_raw(table).map(|info| info.dbi())
    }

    /// Gets the database handle for the given table.
    pub fn get_dbi<T: Table>(&self) -> Result<u32, MdbxError> {
        self.get_dbi_raw(T::NAME)
    }

    /// Gets this transaction ID.
    pub fn id(&self) -> Result<u64, MdbxError> {
        self.inner.id().map_err(MdbxError::Mdbx)
    }

    /// Create [`Cursor`] for raw table name.
    pub fn new_cursor_raw<'a>(&'a self, name: &'static str) -> Result<Cursor<'a, K>, MdbxError> {
        let info = self.cache_db_info_raw(name)?;

        let inner = self.inner.cursor_with_dbi(info.dbi())?;

        Ok(Cursor::new(inner, info))
    }

    /// Create a [`Cursor`] for the given table.
    pub fn new_cursor<'a, T: Table>(&'a self) -> Result<Cursor<'a, K>, MdbxError> {
        Self::new_cursor_raw(self, T::NAME)
    }
}

impl Tx<RW> {
    fn store_db_info(&self, table: &'static str, db_info: DbInfo) -> Result<(), MdbxError> {
        // This needs to be low-level to avoid issues
        let dbi = self.db_info_table_dbi()?;

        // reuse the scratch buffer for encoding the DbInfo key
        // The first 32 bytes are for the key, the rest for the value

        // SAFETY: The write buffer cannot be aliased while we have &self

        let mut key_buf = [0u8; MAX_KEY_SIZE];
        let mut value_buf: &mut [u8] = &mut [0u8; MAX_FIXED_VAL_SIZE];

        {
            let to_copy = core::cmp::min(32, table.len());
            key_buf[..to_copy].copy_from_slice(&table.as_bytes()[..to_copy]);
            key_buf[to_copy..32].fill(0);
        }
        {
            db_info.encode_value_to(&mut value_buf);
        }

        self.inner
            .put(dbi, key_buf, &value_buf[..db_info.encoded_size()], WriteFlags::UPSERT)
            .map(|_| ())
            .map_err(MdbxError::Mdbx)?;
        self.dbs.insert(table, db_info);

        Ok(())
    }
}

impl<K> HotKvRead for Tx<K>
where
    K: TransactionKind,
{
    type Error = MdbxError;

    type Traverse<'a> = Cursor<'a, K>;

    fn raw_traverse<'a>(&'a self, table: &'static str) -> Result<Self::Traverse<'a>, Self::Error> {
        self.new_cursor_raw(table)
    }

    fn raw_get<'a>(
        &'a self,
        table: &'static str,
        key: &[u8],
    ) -> Result<Option<Cow<'a, [u8]>>, Self::Error> {
        let dbi = self.get_dbi_raw(table)?;

        self.inner.get(dbi, key.as_ref()).map_err(MdbxError::Mdbx)
    }

    fn raw_get_dual<'a>(
        &'a self,
        _table: &'static str,
        _key1: &[u8],
        _key2: &[u8],
    ) -> Result<Option<Cow<'a, [u8]>>, Self::Error> {
        unimplemented!("Use DualTableTraverse for raw_get_dual");
    }

    fn get_dual<T: DualKey>(
        &self,
        key1: &T::Key,
        key2: &T::Key2,
    ) -> Result<Option<T::Value>, Self::Error> {
        let mut cursor = self.new_cursor::<T>()?;

        DualTableTraverse::<T, MdbxError>::exact_dual(&mut cursor, key1, key2)
    }
}

impl HotKvWrite for Tx<RW> {
    type TraverseMut<'a> = Cursor<'a, RW>;

    fn raw_traverse_mut<'a>(
        &'a self,
        table: &'static str,
    ) -> Result<Self::TraverseMut<'a>, Self::Error> {
        self.new_cursor_raw(table)
    }

    fn queue_raw_put(
        &self,
        table: &'static str,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), Self::Error> {
        let dbi = self.get_dbi_raw(table)?;

        self.inner.put(dbi, key, value, WriteFlags::UPSERT).map(|_| ()).map_err(MdbxError::Mdbx)
    }

    fn queue_raw_put_dual(
        &self,
        table: &'static str,
        key1: &[u8],
        key2: &[u8],
        value: &[u8],
    ) -> Result<(), Self::Error> {
        // Get the DBI and release the borrow, allowing us to write to buf
        let db_info = self.cache_db_info_raw(table)?;
        let fsi = db_info.dup_fixed_val_size();
        let dbi = db_info.dbi();
        drop(db_info);

        if let FixedSizeInfo::Size { key2_size, value_size } = fsi {
            debug_assert_eq!(
                key2.len(),
                key2_size,
                "Key2 length does not match fixed size for table {}",
                table
            );
            debug_assert_eq!(
                value.len(),
                value_size,
                "Value length does not match fixed size for table {}",
                table
            );
        }

        // For DUPSORT tables, the "value" is key2 concatenated with the actual
        // value.
        // If the value is fixed size, we can write directly into our scratch
        // buffer. Otherwise, we need to allocate
        //
        // NB: DUPSORT and RESERVE are incompatible :(
        if key2.len() + value.len() > TX_BUFFER_SIZE {
            // Allocate a buffer for the combined value
            let mut combined = Vec::with_capacity(key2.len() + value.len());
            combined.extend_from_slice(key2);
            combined.extend_from_slice(value);
            return self
                .inner
                .put(dbi, key1, &combined, WriteFlags::UPSERT)
                .map(|_| ())
                .map_err(MdbxError::Mdbx);
        } else {
            // Use the scratch buffer
            let mut buffer = [0u8; TX_BUFFER_SIZE];
            let buf = &mut buffer[..key2.len() + value.len()];
            buf[..key2.len()].copy_from_slice(key2);
            buf[key2.len()..].copy_from_slice(value);
            self.inner.put(dbi, key1, buf, Default::default())?;
        }

        Ok(())
    }

    fn queue_raw_delete(&self, table: &'static str, key: &[u8]) -> Result<(), Self::Error> {
        let dbi = self.get_dbi_raw(table)?;
        self.inner.del(dbi, key, None).map(|_| ()).map_err(MdbxError::Mdbx)
    }

    fn queue_raw_delete_dual(
        &self,
        table: &'static str,
        key1: &[u8],
        key2: &[u8],
    ) -> Result<(), Self::Error> {
        // Get the table info, then release the borrow
        let db_info = self.cache_db_info_raw(table)?;
        let fixed_val = db_info.dup_fixed_val_size();
        let dbi = db_info.dbi();
        drop(db_info);

        // For DUPSORT tables, the "value" is key2 concatenated with the actual
        // value. If the table is ALSO dupfixed, we need to pad key2 to the
        // fixed size
        if let Some(total_size) = fixed_val.total_size() {
            // Copy key2 to scratch buffer and zero-pad to total fixed size
            let mut buffer = [0u8; TX_BUFFER_SIZE];
            buffer[..key2.len()].copy_from_slice(key2);
            buffer[key2.len()..total_size].fill(0);
            let k2 = &buffer[..total_size];

            self.inner.del(dbi, key1, Some(k2)).map(|_| ()).map_err(MdbxError::Mdbx)
        } else {
            self.inner.del(dbi, key1, Some(key2)).map(|_| ()).map_err(MdbxError::Mdbx)
        }
    }

    fn queue_raw_clear(&self, table: &'static str) -> Result<(), Self::Error> {
        let dbi = self.get_dbi_raw(table)?;
        self.inner.clear_db(dbi).map(|_| ()).map_err(MdbxError::Mdbx)
    }

    fn queue_raw_create(
        &self,
        table: &'static str,
        dual_key: Option<usize>,
        fixed_val: Option<usize>,
    ) -> Result<(), Self::Error> {
        let mut flags = DatabaseFlags::default();

        let mut fsi = FixedSizeInfo::None;

        if let Some(ks) = dual_key {
            flags.set(reth_libmdbx::DatabaseFlags::DUP_SORT, true);
            if let Some(vs) = fixed_val {
                flags.set(reth_libmdbx::DatabaseFlags::DUP_FIXED, true);
                fsi = FixedSizeInfo::Size { key2_size: ks, value_size: vs };
            }
        }

        // no clone. sad.
        let flags2 = DatabaseFlags::from_bits(flags.bits()).unwrap();

        self.inner.create_db(Some(table), flags2).map(|_| ())?;
        let dbi = self.inner.open_db(Some(table))?.dbi();

        let db_info = DbInfo::new(flags, dbi, fsi);

        self.store_db_info(table, db_info)?;

        Ok(())
    }

    fn queue_put<T: SingleKey>(&self, key: &T::Key, value: &T::Value) -> Result<(), Self::Error> {
        let dbi = self.get_dbi::<T>()?;
        let mut key_buf = [0u8; MAX_KEY_SIZE];
        let key_bytes = key.encode_key(&mut key_buf);

        self.inner
            .reserve(dbi, key_bytes, value.encoded_size(), WriteFlags::UPSERT)
            .map_err(MdbxError::Mdbx)
            .map(|mut reserved| value.encode_value_to(&mut reserved))
    }

    fn raw_commit(self) -> Result<(), Self::Error> {
        // when committing, mdbx returns true on failure
        self.inner.commit().map(drop).map_err(MdbxError::Mdbx)
    }
}
