use crate::{
    hot::{HotKvRead, HotKvReadError, HotKvWrite},
    ser::DeserError,
};
use reth_db::mdbx::{RW, TransactionKind, WriteFlags, tx::Tx};
use reth_db_api::DatabaseError;
use std::borrow::Cow;

/// Error type for reth-libmdbx based hot storage.
#[derive(Debug, thiserror::Error)]
pub enum MdbxError {
    /// Inner error
    #[error(transparent)]
    Mdbx(#[from] reth_libmdbx::Error),
    /// Deser.
    #[error(transparent)]
    Deser(#[from] DeserError),
}

impl HotKvReadError for MdbxError {
    fn into_hot_kv_error(self) -> super::HotKvError {
        match self {
            MdbxError::Mdbx(e) => super::HotKvError::from_err(e),
            MdbxError::Deser(e) => super::HotKvError::Deser(e),
        }
    }
}

impl From<DeserError> for DatabaseError {
    fn from(value: DeserError) -> Self {
        DatabaseError::Other(value.to_string())
    }
}

impl<K> HotKvRead for Tx<K>
where
    K: TransactionKind,
{
    type Error = MdbxError;

    fn get_raw<'a>(
        &'a self,
        table: &str,
        key: &[u8],
    ) -> Result<Option<Cow<'a, [u8]>>, Self::Error> {
        let dbi = self.inner.open_db(Some(table)).map(|db| db.dbi())?;

        self.inner.get(dbi, key.as_ref()).map_err(MdbxError::Mdbx)
    }
}

impl HotKvWrite for Tx<RW> {
    fn queue_raw_put(&mut self, table: &str, key: &[u8], value: &[u8]) -> Result<(), Self::Error> {
        let dbi = self.inner.open_db(Some(table)).map(|db| db.dbi())?;

        self.inner.put(dbi, key, value, WriteFlags::UPSERT).map(|_| ()).map_err(MdbxError::Mdbx)
    }

    fn queue_raw_delete(&mut self, table: &str, key: &[u8]) -> Result<(), Self::Error> {
        let dbi = self.inner.open_db(Some(table)).map(|db| db.dbi())?;

        self.inner.del(dbi, key, None).map(|_| ()).map_err(MdbxError::Mdbx)
    }

    fn raw_commit(self) -> Result<(), Self::Error> {
        // when committing, mdbx returns true on failure
        self.inner.commit().map(drop).map_err(MdbxError::Mdbx)
    }
}
