use alloy::primitives::{Address, B256, BlockNumber, U256};
use reth::primitives::{Account, Bytecode, Header, StorageEntry};
use reth_db::models::BlockNumberAddress;
use reth_db_api::{
    BlockNumberList,
    models::{AccountBeforeTx, ShardedKey, storage_sharded_key::StorageShardedKey},
};

table! {
    /// Records recent block Headers, by their number.
    Headers<BlockNumber => Header>
}

table! {
    /// Records block numbers by hash.
    HeaderNumbers<B256 => BlockNumber>
}

table! {
    /// Records contract Bytecode, by its hash.
    Bytecodes<B256 => Bytecode>
}

table! {
     /// Records plain account states, keyed by address.
    PlainAccountState<Address => Account>
}

table! {
    /// Records account state change history, keyed by address.
    AccountsHistory<ShardedKey<Address> => BlockNumberList>
}

table! {
    /// Records storage state change history, keyed by address and storage key.
    StorageHistory<StorageShardedKey => BlockNumberList>
}

table! {
    /// Records plain storage states, keyed by address and storage key.
    PlainStorageState<Address => B256 => U256> is 32
}

table! {
    /// Records account states before transactions, keyed by (address, block number).
    StorageChangeSets<BlockNumberAddress => B256 => StorageEntry> is 32 + 32
}

table! {
    /// Records account states before transactions, keyed by (address, block number).
    AccountChangeSets<BlockNumberAddress => Address => AccountBeforeTx>
}
