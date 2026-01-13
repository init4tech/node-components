use crate::{
    hot::{HotKvRead, HotKvWrite},
    tables::hot::{self as tables, AccountStorageKey},
};
use alloy::primitives::{Address, B256, U256};
use reth::primitives::{Account, Bytecode, Header, SealedHeader, StorageEntry};
use std::borrow::Cow;

/// Trait for database read operations.
pub trait HotDbReader: sealed::Sealed {
    /// The error type for read operations
    type Error: std::error::Error + Send + Sync + 'static + From<crate::ser::DeserError>;

    /// Read a block header by its number.
    fn get_header(&self, number: u64) -> Result<Option<Header>, Self::Error>;

    /// Read a block number by its hash.
    fn get_header_number(&self, hash: &B256) -> Result<Option<u64>, Self::Error>;

    /// Read the canonical hash by block number.
    fn get_canonical_hash(&self, number: u64) -> Result<Option<B256>, Self::Error>;

    /// Read contract Bytecode by its hash.
    fn get_bytecode(&self, code_hash: &B256) -> Result<Option<Bytecode>, Self::Error>;

    /// Read an account by its address.
    fn get_account(&self, address: &Address) -> Result<Option<Account>, Self::Error>;

    /// Read a storage slot by its address and key.
    fn get_storage(&self, address: &Address, key: &B256) -> Result<Option<U256>, Self::Error>;

    /// Read a [`StorageEntry`] by its address and key.
    fn get_storage_entry(
        &self,
        address: &Address,
        key: &B256,
    ) -> Result<Option<StorageEntry>, Self::Error> {
        let opt = self.get_storage(address, key)?;
        Ok(opt.map(|value| StorageEntry { key: *key, value }))
    }
}

impl<T> HotDbReader for T
where
    T: HotKvRead,
{
    type Error = <T as HotKvRead>::Error;

    fn get_header(&self, number: u64) -> Result<Option<Header>, Self::Error> {
        self.get::<tables::Headers>(&number)
    }

    fn get_header_number(&self, hash: &B256) -> Result<Option<u64>, Self::Error> {
        self.get::<tables::HeaderNumbers>(hash)
    }

    fn get_canonical_hash(&self, number: u64) -> Result<Option<B256>, Self::Error> {
        self.get::<tables::CanonicalHeaders>(&number)
    }

    fn get_bytecode(&self, code_hash: &B256) -> Result<Option<Bytecode>, Self::Error> {
        self.get::<tables::Bytecodes>(code_hash)
    }

    fn get_account(&self, address: &Address) -> Result<Option<Account>, Self::Error> {
        self.get::<tables::PlainAccountState>(address)
    }

    fn get_storage(&self, address: &Address, key: &B256) -> Result<Option<U256>, Self::Error> {
        let storage_key = AccountStorageKey {
            address: std::borrow::Cow::Borrowed(address),
            key: std::borrow::Cow::Borrowed(key),
        };
        let key = storage_key.encode_key();
        self.get::<tables::PlainStorageState>(&key)
    }
}

/// Trait for database write operations.
pub trait HotDbWriter: sealed::Sealed {
    /// The error type for write operations
    type Error: std::error::Error + Send + Sync + 'static + From<crate::ser::DeserError>;

    /// Read the latest block header.
    fn put_header(&mut self, header: &Header) -> Result<(), Self::Error>;

    /// Write a block number by its hash.
    fn put_header_number(&mut self, hash: &B256, number: u64) -> Result<(), Self::Error>;

    /// Write the canonical hash by block number.
    fn put_canonical_hash(&mut self, number: u64, hash: &B256) -> Result<(), Self::Error>;

    /// Write contract Bytecode by its hash.
    fn put_bytecode(&mut self, code_hash: &B256, bytecode: &Bytecode) -> Result<(), Self::Error>;

    /// Write an account by its address.
    fn put_account(&mut self, address: &Address, account: &Account) -> Result<(), Self::Error>;

    /// Write a storage entry by its address and key.
    fn put_storage(
        &mut self,
        address: &Address,
        key: &B256,
        entry: &U256,
    ) -> Result<(), Self::Error>;

    /// Commit the write transaction.
    fn commit(self) -> Result<(), Self::Error>;

    /// Write a canonical header (header, number mapping, and canonical hash).
    fn put_canonical(&mut self, header: &SealedHeader) -> Result<(), Self::Error> {
        self.put_header(header)?;
        self.put_header_number(&header.hash(), header.number)?;
        self.put_canonical_hash(header.number, &header.hash())
    }
}

impl<T> HotDbWriter for T
where
    T: HotKvWrite,
{
    type Error = <T as HotKvRead>::Error;

    fn put_header(&mut self, header: &Header) -> Result<(), Self::Error> {
        self.queue_put::<tables::Headers>(&header.number, header)
    }

    fn put_header_number(&mut self, hash: &B256, number: u64) -> Result<(), Self::Error> {
        self.queue_put::<tables::HeaderNumbers>(hash, &number)
    }

    fn put_canonical_hash(&mut self, number: u64, hash: &B256) -> Result<(), Self::Error> {
        self.queue_put::<tables::CanonicalHeaders>(&number, hash)
    }

    fn put_bytecode(&mut self, code_hash: &B256, bytecode: &Bytecode) -> Result<(), Self::Error> {
        self.queue_put::<tables::Bytecodes>(code_hash, bytecode)
    }

    fn put_account(&mut self, address: &Address, account: &Account) -> Result<(), Self::Error> {
        self.queue_put::<tables::PlainAccountState>(address, account)
    }

    fn put_storage(
        &mut self,
        address: &Address,
        key: &B256,
        entry: &U256,
    ) -> Result<(), Self::Error> {
        let storage_key =
            AccountStorageKey { address: Cow::Borrowed(address), key: Cow::Borrowed(key) };
        self.queue_put::<tables::PlainStorageState>(&storage_key.encode_key(), entry)
    }

    fn commit(self) -> Result<(), Self::Error> {
        HotKvWrite::raw_commit(self)
    }
}

mod sealed {
    use crate::hot::HotKvRead;

    /// Sealed trait to prevent external implementations of HotDbReader and HotDbWriter.
    #[allow(dead_code, unreachable_pub)]
    pub trait Sealed {}
    impl<T> Sealed for T where T: HotKvRead {}
}
