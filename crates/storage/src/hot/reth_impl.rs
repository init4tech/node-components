use std::borrow::Cow;

use crate::{hot::HotKvRead, ser::DeserError};
use reth_db::mdbx::{TransactionKind, tx::Tx};
use reth_db_api::DatabaseError;

impl From<DeserError> for DatabaseError {
    fn from(value: DeserError) -> Self {
        DatabaseError::Other(value.to_string())
    }
}

impl<K> HotKvRead for Tx<K>
where
    K: TransactionKind,
{
    type Error = DatabaseError;

    fn get_raw<'a>(
        &'a self,
        table: &str,
        key: &[u8],
    ) -> Result<Option<Cow<'a, [u8]>>, Self::Error> {
        let dbi = self
            .inner
            .open_db(Some(table))
            .map(|db| db.dbi())
            .map_err(|e| DatabaseError::Open(e.into()))?;

        self.inner.get(dbi, key.as_ref()).map_err(|err| DatabaseError::Read(err.into()))
    }
}
