use crate::{constants::TEST_CONSTANTS, context::SignetTestContext};
use alloy::{
    consensus::{SignableTransaction, TxEip1559, constants::GWEI_TO_WEI},
    primitives::{Address, B256, FixedBytes, Sealable, TxKind, U256},
    signers::{SignerSync, local::PrivateKeySigner},
    uint,
};
use reth::{
    chainspec::ChainSpec,
    primitives::{Block, BlockBody, Header, RecoveredBlock, Transaction, TransactionSigned},
    providers::{ProviderFactory, providers::StaticFileProvider},
};
use reth_db::test_utils::{create_test_rw_db, create_test_static_files_dir};
use reth_exex_test_utils::TmpDB;
use signet_node_types::SignetNodeTypes;
use signet_zenith::Zenith;
use std::{panic, sync::Once};
use tracing_subscriber::EnvFilter;

/// Make a fake block with a specific number.
pub fn fake_block(number: u64) -> RecoveredBlock<Block> {
    let header = Header {
        difficulty: U256::from(0x4000_0000),
        number,
        mix_hash: B256::repeat_byte(0xed),
        nonce: FixedBytes::repeat_byte(0xbe),
        timestamp: 1716555586, // the time when i wrote this function lol
        excess_blob_gas: Some(0),
        ..Default::default()
    };
    let (header, hash) = header.seal_slow().into_parts();

    let senders = vec![];
    RecoveredBlock::new(
        Block::new(header, BlockBody { transactions: vec![], ommers: vec![], withdrawals: None }),
        senders,
        hash,
    )
}

/// Sign a transaction with a wallet.
pub fn sign_tx_with_key_pair(wallet: &PrivateKeySigner, tx: Transaction) -> TransactionSigned {
    let signature = wallet.sign_hash_sync(&tx.signature_hash()).unwrap();
    TransactionSigned::new_unhashed(tx, signature)
}

/// Make a wallet with a deterministic keypair.
pub fn make_wallet(i: u8) -> PrivateKeySigner {
    PrivateKeySigner::from_bytes(&B256::repeat_byte(i)).unwrap()
}

/// Make a simple send transaction.
pub fn simple_send(to: Address, amount: U256, nonce: u64) -> Transaction {
    TxEip1559 {
        nonce,
        gas_limit: 21_000,
        to: TxKind::Call(to),
        value: amount,
        chain_id: TEST_CONSTANTS.ru_chain_id(),
        max_fee_per_gas: GWEI_TO_WEI as u128 * 100,
        max_priority_fee_per_gas: GWEI_TO_WEI as u128,
        ..Default::default()
    }
    .into()
}

/// Make a zenith header with a specific sequence number.
pub fn zenith_header(host_height: u64) -> Zenith::BlockHeader {
    Zenith::BlockHeader {
        rollupChainId: U256::from(TEST_CONSTANTS.ru_chain_id()),
        hostBlockNumber: U256::from(host_height),
        gasLimit: U256::from(100_000_000),
        rewardAddress: Default::default(),
        blockDataHash: Default::default(),
    }
}

/// Run a test with a context and Signet Node instance.
pub async fn run_test<F, Fut>(f: F)
where
    F: FnOnce(SignetTestContext) -> Fut,
    Fut: std::future::Future<Output = ()> + Send,
{
    static TRACING_INIT: Once = Once::new();

    TRACING_INIT.call_once(|| {
        tracing_subscriber::fmt::fmt()
            .with_max_level(None)
            .with_env_filter(
                EnvFilter::from_default_env().add_directive("reth_tasks=off".parse().unwrap()),
            )
            .init();
    });

    let (ctx, signet) = SignetTestContext::new().await;

    f(ctx).await;

    signet.abort();
    match signet.await {
        Ok(res) => res.unwrap(),
        Err(err) => {
            if let Ok(reason) = err.try_into_panic() {
                panic::resume_unwind(reason);
            }
        }
    }
}

/// Adjust the amount of USD to the correct decimal places.
///
/// This is calculated as `amount * 10^(18 - decimals)`.
pub fn adjust_usd_decimals_u256(amount: U256, decimals: u8) -> U256 {
    uint! {
        amount * 10_U256.pow(18_U256 - U256::from(decimals))
    }
}

/// Adjust the amount of USD to the correct decimal places for a usize input.
///
/// This is calculated as `amount * 10^(18 - decimals)`.
pub fn adjust_usd_decimals(amount: usize, decimals: u8) -> U256 {
    adjust_usd_decimals_u256(U256::from(amount), decimals)
}

/// Create a provider factory with a chain spec
pub fn create_test_provider_factory_with_chain_spec(
    chain_spec: std::sync::Arc<ChainSpec>,
) -> ProviderFactory<SignetNodeTypes<TmpDB>> {
    let (static_dir, _) = create_test_static_files_dir();
    let db = create_test_rw_db();
    ProviderFactory::new(
        db,
        chain_spec,
        StaticFileProvider::read_write(static_dir.keep()).expect("static file provider"),
    )
}
