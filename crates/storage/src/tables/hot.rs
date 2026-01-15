use alloy::primitives::{Address, B256, BlockNumber, U256};
use reth::primitives::{Account, Bytecode, Header, StorageEntry};
use reth_db::models::BlockNumberAddress;
use reth_db_api::{
    BlockNumberList,
    models::{AccountBeforeTx, ShardedKey, storage_sharded_key::StorageShardedKey},
};

tables! {
    /// Records recent block Headers, by their number.
    Headers<BlockNumber => Header>,

    /// Records block numbers by hash.
    HeaderNumbers<B256 => BlockNumber>,

    /// Records the canonical chain header hashes, by height.
    CanonicalHeaders<BlockNumber => B256>,

    /// Records contract Bytecode, by its hash.
    Bytecodes<B256 => Bytecode>,

    /// Records plain account states, keyed by address.
    PlainAccountState<Address => Account>,

    /// Records account state change history, keyed by address.
    AccountsHistory<ShardedKey<Address> => BlockNumberList>,

    /// Records storage state change history, keyed by address and storage key.
    StorageHistory<StorageShardedKey => BlockNumberList>,

}

tables! {
    /// Records plain storage states, keyed by address and storage key.
    PlainStorageState<Address => B256 => U256> size: Some(32 + 32),

    /// Records account states before transactions, keyed by (address, block number).
    StorageChangeSets<BlockNumberAddress => B256 => StorageEntry> size: Some(32 + 32 + 32),

    /// Records account states before transactions, keyed by (address, block number).
    AccountChangeSets<BlockNumberAddress => Address => AccountBeforeTx> size: None,
}
