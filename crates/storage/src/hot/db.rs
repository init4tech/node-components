use crate::hot::{HotKv, HotKvError, HotKvRead, HotKvWrite};
use std::{
    borrow::Cow,
    sync::atomic::{AtomicBool, Ordering},
};

/// Hot database wrapper around a key-value storage.
#[derive(Debug)]
pub struct HotDb<Inner> {
    inner: Inner,

    write_locked: AtomicBool,
}

impl<Inner> HotDb<Inner> {
    /// Create a new HotDb wrapping the given inner KV storage.
    pub const fn new(inner: Inner) -> Self {
        Self { inner, write_locked: AtomicBool::new(false) }
    }

    /// Get a read-only handle.
    pub fn reader(&self) -> Result<Inner::RoTx, HotKvError>
    where
        Inner: HotKv,
    {
        self.inner.reader()
    }

    /// Get a write handle, if available. If not available, returns
    /// [`HotKvError::WriteLocked`].
    pub fn writer(&self) -> Result<WriteGuard<'_, Inner>, HotKvError>
    where
        Inner: HotKv,
    {
        if self.write_locked.swap(true, Ordering::AcqRel) {
            return Err(HotKvError::WriteLocked);
        }
        self.inner.writer().map(Some).map(|tx| WriteGuard { tx, db: self })
    }
}

/// Write guard for a write transaction.
#[derive(Debug)]
pub struct WriteGuard<'a, Inner>
where
    Inner: HotKv,
{
    tx: Option<Inner::RwTx>,
    db: &'a HotDb<Inner>,
}

impl<Inner> Drop for WriteGuard<'_, Inner>
where
    Inner: HotKv,
{
    fn drop(&mut self) {
        self.db.write_locked.store(false, Ordering::Release);
    }
}

impl<Inner> HotKvRead for WriteGuard<'_, Inner>
where
    Inner: HotKv,
{
    type Error = <<Inner as HotKv>::RwTx as HotKvRead>::Error;

    fn get_raw<'a>(
        &'a self,
        table: &str,
        key: &[u8],
    ) -> Result<Option<Cow<'a, [u8]>>, Self::Error> {
        self.tx.as_ref().expect("present until drop").get_raw(table, key)
    }
}

impl<Inner> HotKvWrite for WriteGuard<'_, Inner>
where
    Inner: HotKv,
{
    fn queue_raw_put(&mut self, table: &str, key: &[u8], value: &[u8]) -> Result<(), Self::Error> {
        self.tx.as_mut().expect("present until drop").queue_raw_put(table, key, value)
    }

    fn raw_commit(mut self) -> Result<(), Self::Error> {
        self.tx.take().expect("present until drop").raw_commit()
    }
}
