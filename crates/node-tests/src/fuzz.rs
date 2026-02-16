//! Proptest strategies for generating fuzzed Ethereum transaction and block
//! types. Provides structurally aware fuzzing for use in integration tests.
//!
//! # Strategies
//!
//! - [`arb_tx_legacy`] — Legacy (pre-EIP-2718) transactions
//! - [`arb_tx_eip1559`] — EIP-1559 fee market transactions
//! - [`arb_tx_eip2930`] — EIP-2930 access list transactions
//! - [`arb_tx_eip4844`] — EIP-4844 blob transactions
//! - [`arb_transaction`] — Any supported transaction variant
//! - [`arb_header`] — Block headers
//! - [`arb_block_body`] — Block bodies (with transactions)
//! - [`arb_block`] — Complete blocks (header + body)
//!
//! # CI configuration
//!
//! Set `PROPTEST_CASES` to control iteration count (default 64, use lower
//! values in CI for faster runs).

use alloy::{
    consensus::{TxEip1559, TxEip2930, TxEip4844, TxLegacy},
    eips::eip2930::AccessList,
    primitives::{Address, B256, Bytes, TxKind, U256},
};
use proptest::prelude::*;
use reth::primitives::{Block, BlockBody, Header, Transaction};

/// Default number of proptest cases. Override with `PROPTEST_CASES` env var.
pub const DEFAULT_PROPTEST_CASES: u32 = 64;

/// Build a [`ProptestConfig`] that respects the `PROPTEST_CASES` env var,
/// falling back to [`DEFAULT_PROPTEST_CASES`].
pub fn proptest_config() -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PROPTEST_CASES);
    ProptestConfig { cases, ..ProptestConfig::default() }
}

// ── Primitive strategies ────────────────────────────────────────────

/// Strategy for an arbitrary [`Address`].
pub fn arb_address() -> impl Strategy<Value = Address> {
    any::<[u8; 20]>().prop_map(Address::from)
}

/// Strategy for an arbitrary [`B256`] hash.
pub fn arb_b256() -> impl Strategy<Value = B256> {
    any::<[u8; 32]>().prop_map(B256::from)
}

/// Strategy for an arbitrary [`U256`] (up to u128 range for sanity).
pub fn arb_u256() -> impl Strategy<Value = U256> {
    any::<u128>().prop_map(U256::from)
}

/// Strategy for [`TxKind`] — either a `Create` or `Call`.
pub fn arb_tx_kind() -> impl Strategy<Value = TxKind> {
    prop_oneof![Just(TxKind::Create), arb_address().prop_map(TxKind::Call),]
}

/// Strategy for calldata [`Bytes`] (0–256 bytes).
pub fn arb_input_bytes() -> impl Strategy<Value = Bytes> {
    proptest::collection::vec(any::<u8>(), 0..256).prop_map(Bytes::from)
}

/// Strategy for an [`AccessList`].
pub fn arb_access_list() -> impl Strategy<Value = AccessList> {
    proptest::collection::vec((arb_address(), proptest::collection::vec(arb_b256(), 0..4)), 0..4)
        .prop_map(|items| {
            AccessList::from(
                items
                    .into_iter()
                    .map(|(address, storage_keys)| alloy::eips::eip2930::AccessListItem {
                        address,
                        storage_keys,
                    })
                    .collect::<Vec<_>>(),
            )
        })
}

// ── Transaction strategies ──────────────────────────────────────────

/// Strategy for a Legacy transaction.
pub fn arb_tx_legacy() -> impl Strategy<Value = Transaction> {
    (
        any::<u64>(),         // nonce
        1..=500_000u64,       // gas_limit
        any::<u128>(),        // gas_price
        arb_u256(),           // value
        arb_tx_kind(),        // to
        arb_input_bytes(),    // input
        any::<Option<u64>>(), // chain_id
    )
        .prop_map(|(nonce, gas_limit, gas_price, value, to, input, chain_id)| {
            TxLegacy { nonce, gas_limit, gas_price, value, to, input, chain_id }.into()
        })
}

/// Strategy for an EIP-1559 transaction.
pub fn arb_tx_eip1559() -> impl Strategy<Value = Transaction> {
    (
        any::<u64>(),      // chain_id
        any::<u64>(),      // nonce
        1..=500_000u64,    // gas_limit
        any::<u128>(),     // max_fee_per_gas
        any::<u128>(),     // max_priority_fee_per_gas
        arb_u256(),        // value
        arb_tx_kind(),     // to
        arb_input_bytes(), // input
        arb_access_list(), // access_list
    )
        .prop_map(
            |(
                chain_id,
                nonce,
                gas_limit,
                max_fee_per_gas,
                max_priority_fee_per_gas,
                value,
                to,
                input,
                access_list,
            )| {
                TxEip1559 {
                    chain_id,
                    nonce,
                    gas_limit,
                    max_fee_per_gas,
                    max_priority_fee_per_gas,
                    value,
                    to,
                    input,
                    access_list,
                }
                .into()
            },
        )
}

/// Strategy for an EIP-2930 transaction.
pub fn arb_tx_eip2930() -> impl Strategy<Value = Transaction> {
    (
        any::<u64>(),      // chain_id
        any::<u64>(),      // nonce
        1..=500_000u64,    // gas_limit
        any::<u128>(),     // gas_price
        arb_u256(),        // value
        arb_tx_kind(),     // to
        arb_input_bytes(), // input
        arb_access_list(), // access_list
    )
        .prop_map(|(chain_id, nonce, gas_limit, gas_price, value, to, input, access_list)| {
            TxEip2930 { chain_id, nonce, gas_limit, gas_price, value, to, input, access_list }
                .into()
        })
}

/// Strategy for an EIP-4844 blob transaction (without the sidecar).
pub fn arb_tx_eip4844() -> impl Strategy<Value = Transaction> {
    (
        any::<u64>(),                                 // chain_id
        any::<u64>(),                                 // nonce
        1..=500_000u64,                               // gas_limit
        any::<u128>(),                                // max_fee_per_gas
        any::<u128>(),                                // max_priority_fee_per_gas
        any::<u128>(),                                // max_fee_per_blob_gas
        arb_u256(),                                   // value
        arb_address(),                                // to
        arb_input_bytes(),                            // input
        arb_access_list(),                            // access_list
        proptest::collection::vec(arb_b256(), 1..=4), // blob_versioned_hashes
    )
        .prop_map(
            |(
                chain_id,
                nonce,
                gas_limit,
                max_fee_per_gas,
                max_priority_fee_per_gas,
                max_fee_per_blob_gas,
                value,
                to,
                input,
                access_list,
                blob_versioned_hashes,
            )| {
                TxEip4844 {
                    chain_id,
                    nonce,
                    gas_limit,
                    max_fee_per_gas,
                    max_priority_fee_per_gas,
                    max_fee_per_blob_gas,
                    value,
                    to: TxKind::Call(to),
                    input,
                    access_list,
                    blob_versioned_hashes,
                }
                .into()
            },
        )
}

/// Strategy that produces any of the supported transaction types.
pub fn arb_transaction() -> impl Strategy<Value = Transaction> {
    prop_oneof![arb_tx_legacy(), arb_tx_eip1559(), arb_tx_eip2930(), arb_tx_eip4844(),]
}

// ── Block strategies ────────────────────────────────────────────────

/// Strategy for a block [`Header`].
pub fn arb_header() -> impl Strategy<Value = Header> {
    (
        arb_b256(),           // parent_hash
        arb_address(),        // beneficiary
        arb_b256(),           // state_root
        arb_b256(),           // transactions_root
        arb_b256(),           // receipts_root
        any::<u64>(),         // number
        1..=30_000_000u64,    // gas_limit
        any::<u64>(),         // gas_used
        any::<u64>(),         // timestamp
        arb_u256(),           // difficulty
        arb_b256(),           // mix_hash
        any::<u64>(),         // nonce
        any::<Option<u64>>(), // base_fee_per_gas
        any::<Option<u64>>(), // excess_blob_gas
    )
        .prop_map(
            |(
                parent_hash,
                beneficiary,
                state_root,
                transactions_root,
                receipts_root,
                number,
                gas_limit,
                gas_used,
                timestamp,
                difficulty,
                mix_hash,
                nonce,
                base_fee_per_gas,
                excess_blob_gas,
            )| {
                Header {
                    parent_hash,
                    beneficiary,
                    state_root,
                    transactions_root,
                    receipts_root,
                    number,
                    gas_limit,
                    gas_used,
                    timestamp,
                    difficulty,
                    mix_hash,
                    nonce: alloy::primitives::FixedBytes::from(nonce.to_be_bytes()),
                    base_fee_per_gas,
                    excess_blob_gas,
                    ..Default::default()
                }
            },
        )
}

/// Strategy for a [`BlockBody`] containing 0–8 fuzzed transactions.
pub fn arb_block_body() -> impl Strategy<Value = BlockBody> {
    proptest::collection::vec(arb_transaction(), 0..8).prop_map(|transactions| BlockBody {
        transactions,
        ommers: vec![],
        withdrawals: None,
    })
}

/// Strategy for a complete [`Block`] (header + body).
pub fn arb_block() -> impl Strategy<Value = Block> {
    (arb_header(), arb_block_body()).prop_map(|(header, body)| Block::new(header, body))
}
