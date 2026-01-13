use std::borrow::Cow;

use crate::tables::Table;
use alloy::primitives::{Address, B256, BlockNumber, FixedBytes, U256};
use reth::primitives::{Account, Bytecode, Header};
use reth_db_api::{
    BlockNumberList,
    models::{AccountBeforeTx, ShardedKey, storage_sharded_key::StorageShardedKey},
};

tables! {
    /// Records recent block Headers, by their number.
    Headers<BlockNumber, Header>,

    /// Records block numbers by hash.
    HeaderNumbers<B256, BlockNumber>,

    /// Records the canonical chain header hashes, by height.
    CanonicalHeaders<BlockNumber, B256>,

    /// Records contract Bytecode, by its hash.
    Bytecodes<B256, Bytecode>,

    /// Records plain account states, keyed by address.
    PlainAccountState<Address, Account>,

    /// Records account state change history, keyed by address.
    AccountsHistory<ShardedKey<Address>, BlockNumberList>,

    /// Records storage state change history, keyed by address and storage key.
    StorageHistory<StorageShardedKey, BlockNumberList>,

    /// Records account change sets, keyed by block number.
    AccountChangeSets<BlockNumber, AccountBeforeTx>,
}

/// Records plain storage states, keyed by address and storage key.
#[derive(Debug, Copy, Clone, Eq, PartialEq, PartialOrd, Ord)]
pub struct PlainStorageState;

impl Table for PlainStorageState {
    const NAME: &'static str = "PlainStorageState";

    type Key = FixedBytes<52>;

    type Value = U256;
}

/// Key for the [`PlainStorageState`] table.
#[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord)]
pub struct AccountStorageKey<'a, 'b> {
    /// Address of the account.
    pub address: Cow<'a, Address>,
    /// Storage key.
    pub key: Cow<'b, B256>,
}

impl AccountStorageKey<'static, 'static> {
    /// Decode the key from the provided data.
    pub fn decode_key(data: &[u8]) -> Result<Self, crate::ser::DeserError> {
        if data.len() < Self::SIZE {
            return Err(crate::ser::DeserError::InsufficientData {
                needed: Self::SIZE,
                available: data.len(),
            });
        }

        let address = Address::from_slice(&data[0..20]);
        let key = B256::from_slice(&data[20..52]);

        Ok(Self { address: Cow::Owned(address), key: Cow::Owned(key) })
    }
}

impl<'a, 'b> AccountStorageKey<'a, 'b> {
    /// Size in bytes.
    pub const SIZE: usize = 20 + 32;

    /// Encode the key into the provided buffer.
    pub fn encode_key(&self) -> FixedBytes<52> {
        let mut buf = [0u8; Self::SIZE];
        buf[0..20].copy_from_slice(self.address.as_slice());
        buf[20..52].copy_from_slice(self.key.as_slice());
        buf.into()
    }
}
