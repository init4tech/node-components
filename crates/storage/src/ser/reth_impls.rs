use crate::ser::{DeserError, KeySer, MAX_KEY_SIZE, ValSer};
use alloy::primitives::{Address, B256};
use reth::primitives::{Account, Bytecode, Header, Log, StorageEntry, TransactionSigned, TxType};
use reth_db_api::{
    BlockNumberList,
    models::{
        AccountBeforeTx, CompactU256, ShardedKey, StoredBlockBodyIndices,
        storage_sharded_key::StorageShardedKey,
    },
    table::{Compress, Decompress},
};

macro_rules! simple_delegate_compress {
    ($ty:ty) => {
        impl ValSer for $ty {
            fn encode_value_to<B>(&self, buf: &mut B)
            where
                B: bytes::BufMut + AsMut<[u8]>,
            {
                self.compress_to_buf(buf);
            }

            fn decode_value(data: &[u8]) -> Result<Self, DeserError>
            where
                Self: Sized,
            {
                Decompress::decompress(data).map_err(DeserError::from)
            }
        }
    };

    ($($ty:ty),* $(,)?) => {
        $(
            simple_delegate_compress!($ty);
        )+
    };
}

simple_delegate_compress!(
    Header,
    Account,
    Log,
    TxType,
    StorageEntry,
    StoredBlockBodyIndices,
    Bytecode,
    AccountBeforeTx,
    TransactionSigned,
    CompactU256,
    BlockNumberList,
);

impl<T: KeySer> KeySer for ShardedKey<T> {
    const SIZE: usize = T::SIZE + u64::SIZE;

    fn encode_key<'a: 'c, 'b: 'c, 'c>(&'a self, buf: &'b mut [u8; MAX_KEY_SIZE]) -> &'c [u8] {
        let mut scratch = [0u8; MAX_KEY_SIZE];

        T::encode_key(&self.key, &mut scratch);
        scratch[T::SIZE..Self::SIZE].copy_from_slice(&self.highest_block_number.to_be_bytes());
        *buf = scratch;

        &buf[0..Self::SIZE]
    }

    fn decode_key(data: &[u8]) -> Result<Self, DeserError> {
        if data.len() < Self::SIZE {
            return Err(DeserError::InsufficientData { needed: Self::SIZE, available: data.len() });
        }

        let key = T::decode_key(&data[0..T::SIZE])?;
        let highest_block_number = u64::decode_key(&data[T::SIZE..T::SIZE + 8])?;
        Ok(Self { key, highest_block_number })
    }
}

impl KeySer for StorageShardedKey {
    const SIZE: usize = Address::SIZE + B256::SIZE + u64::SIZE;

    fn encode_key<'a: 'c, 'b: 'c, 'c>(&'a self, buf: &'b mut [u8; MAX_KEY_SIZE]) -> &'c [u8] {
        buf[0..Address::SIZE].copy_from_slice(self.address.as_slice());
        buf[Address::SIZE..Address::SIZE + B256::SIZE]
            .copy_from_slice(self.sharded_key.key.as_slice());
        buf[Address::SIZE + B256::SIZE..Self::SIZE]
            .copy_from_slice(&self.sharded_key.highest_block_number.to_be_bytes());

        &buf[0..Self::SIZE]
    }

    fn decode_key(mut data: &[u8]) -> Result<Self, DeserError> {
        if data.len() < Self::SIZE {
            return Err(DeserError::InsufficientData { needed: Self::SIZE, available: data.len() });
        }

        let address = Address::from_slice(&data[0..Address::SIZE]);
        data = &data[Address::SIZE..];

        let storage_key = B256::from_slice(&data[0..B256::SIZE]);
        data = &data[B256::SIZE..];

        let highest_block_number = u64::from_be_bytes(data[0..8].try_into().unwrap());

        Ok(Self { address, sharded_key: ShardedKey { key: storage_key, highest_block_number } })
    }
}
