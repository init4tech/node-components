use alloy::primitives::{Address, B256, BlockNumber, U256};
use reth::primitives::{Account, Bytecode, Header};
use reth_db::models::BlockNumberAddress;
use reth_db_api::{BlockNumberList, models::ShardedKey};

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
    /// Records plain storage states, keyed by address and storage key.
    PlainStorageState<Address => B256 => U256> is 32
}

table! {
    /// Records account state change history, keyed by address.
    AccountsHistory<Address => u64 => BlockNumberList>
}

table! {
    /// Records account states before transactions, keyed by (block_number, address).
    AccountChangeSets<BlockNumber => Address => Account> is 8 + 32 + 32
}

table! {
    /// Records storage state change history, keyed by address and storage key.
    StorageHistory<Address => ShardedKey<B256> => BlockNumberList>
}

table! {
    /// Records account states before transactions, keyed by (address, block number).
    StorageChangeSets<BlockNumberAddress => B256 => U256> is 32
}
