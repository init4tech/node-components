use crate::{
    hot::{HotKvRead, HotKvWrite},
    tables::hot::{self as tables},
};
use alloy::primitives::{Address, B256, U256};
use reth::primitives::{Account, Bytecode, Header, SealedHeader, StorageEntry};

/// Trait for database read operations.
pub trait HotDbReader: sealed::Sealed {
    /// The error type for read operations
    type Error: std::error::Error + Send + Sync + 'static + From<crate::ser::DeserError>;

    /// Read a block header by its number.
    fn get_header(&self, number: u64) -> Result<Option<Header>, Self::Error>;

    /// Read a block number by its hash.
    fn get_header_number(&self, hash: &B256) -> Result<Option<u64>, Self::Error>;

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

    /// Read a block header by its hash.
    fn header_by_hash(&self, hash: &B256) -> Result<Option<Header>, Self::Error> {
        let Some(number) = self.get_header_number(hash)? else {
            return Ok(None);
        };
        self.get_header(number)
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

    fn get_bytecode(&self, code_hash: &B256) -> Result<Option<Bytecode>, Self::Error> {
        self.get::<tables::Bytecodes>(code_hash)
    }

    fn get_account(&self, address: &Address) -> Result<Option<Account>, Self::Error> {
        self.get::<tables::PlainAccountState>(address)
    }

    fn get_storage(&self, address: &Address, key: &B256) -> Result<Option<U256>, Self::Error> {
        self.get_dual::<tables::PlainStorageState>(address, key)
    }
}

/// Trait for database write operations.
pub trait HotDbWriter: sealed::Sealed {
    /// The error type for write operations
    type Error: std::error::Error + Send + Sync + 'static + From<crate::ser::DeserError>;

    /// Write a block header. This will leave the DB in an inconsistent state
    /// until the corresponding header number is also written. Users should
    /// prefer [`Self::put_header`] instead.
    fn put_header_inconsistent(&mut self, header: &Header) -> Result<(), Self::Error>;

    /// Write a block number by its hash. This will leave the DB in an
    /// inconsistent state until the corresponding header is also written.
    /// Users should prefer [`Self::put_header`] instead.
    fn put_header_number_inconsistent(
        &mut self,
        hash: &B256,
        number: u64,
    ) -> Result<(), Self::Error>;

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

    /// Write a sealed block header (header + number).
    fn put_header(&mut self, header: &SealedHeader) -> Result<(), Self::Error> {
        self.put_header_inconsistent(header.header())
            .and_then(|_| self.put_header_number_inconsistent(&header.hash(), header.number))
    }

    /// Commit the write transaction.
    fn commit(self) -> Result<(), Self::Error>;
}

impl<T> HotDbWriter for T
where
    T: HotKvWrite,
{
    type Error = <T as HotKvRead>::Error;

    fn put_header_inconsistent(&mut self, header: &Header) -> Result<(), Self::Error> {
        self.queue_put::<tables::Headers>(&header.number, header)
    }

    fn put_header_number_inconsistent(
        &mut self,
        hash: &B256,
        number: u64,
    ) -> Result<(), Self::Error> {
        self.queue_put::<tables::HeaderNumbers>(hash, &number)
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
        self.queue_put_dual::<tables::PlainStorageState>(address, key, entry)
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
