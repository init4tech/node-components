#![allow(non_snake_case)]

use alloy::{
    consensus::Transaction,
    eips::BlockNumberOrTag,
    network::TransactionResponse,
    primitives::{Address, B256, Bytes, LogData, U256},
    providers::Provider,
    pubsub::Subscription,
    rpc::types::{
        Header,
        eth::{Filter, FilterBlockOption, Log},
    },
    sol_types::SolEvent,
};
use reth::providers::{BlockNumReader, BlockReader, TransactionsProvider};
use serial_test::serial;
use signet_node_tests::{
    SignetTestContext,
    aliases::{Counter, TestCounterInstance},
    constants::TEST_CONSTANTS,
    rpc_test,
};
use signet_test_utils::contracts::counter::{COUNTER_BYTECODE, COUNTER_DEPLOY_CODE};
use tokio::try_join;

#[serial]
#[tokio::test]
async fn test_basic_rpc_calls() {
    rpc_test(|ctx, contract| async move {
        tokio::join!(
            test_eth_chainId(&ctx),
            test_eth_gasPrice(&ctx),
            test_eth_blockNumber(&ctx),
            test_eth_getBalance(&ctx, &contract),
            test_eth_getCode(&ctx, &contract),
            test_eth_call(&contract),
            test_eth_estimateGas(&ctx, &contract),
            test_eth_getBlockByHash(&ctx, &contract),
            test_eth_getBlockByNumber(&ctx, &contract),
            test_eth_getTransactionByHash(&ctx, &contract),
            test_eth_getTransactionReceipt(&ctx, &contract),
        );
        ctx
    })
    .await;
}

async fn test_eth_chainId(ctx: &SignetTestContext) {
    let chain_id = ctx.alloy_provider.get_chain_id().await.unwrap();
    assert_eq!(chain_id, TEST_CONSTANTS.ru_chain_id());
}

async fn test_eth_gasPrice(ctx: &SignetTestContext) {
    let gas_price = ctx.alloy_provider.get_gas_price().await.unwrap();
    assert_eq!(gas_price, 1_875_000_000);
}

async fn test_eth_blockNumber(ctx: &SignetTestContext) {
    let (block_number, block) = try_join!(
        ctx.alloy_provider.get_block_number(),
        ctx.alloy_provider.get_block_by_number(BlockNumberOrTag::Latest)
    )
    .unwrap();

    let block = block.unwrap();
    assert_eq!(block_number, 1);
    assert_eq!(block.header.number, block_number);
}

async fn test_eth_getBalance(ctx: &SignetTestContext, contract: &TestCounterInstance) {
    let balance = ctx.alloy_provider.get_balance(*contract.address()).await.unwrap();
    assert_eq!(balance, U256::ZERO);

    for address in ctx.addresses.iter() {
        let actual = ctx.alloy_provider.get_balance(*address).await.unwrap();
        let expected = ctx.balance_of(*address);
        assert_eq!(actual, expected);
    }
}

async fn test_eth_getCode(ctx: &SignetTestContext, contract: &TestCounterInstance) {
    assert_eq!(
        ctx.alloy_provider.get_code_at(*contract.address()).await.unwrap(),
        COUNTER_BYTECODE
    );
}

async fn test_eth_call(contract: &TestCounterInstance) {
    let result = contract.count().call().await.unwrap();
    assert_eq!(result, U256::ZERO);
}

async fn test_eth_estimateGas(ctx: &SignetTestContext, contract: &TestCounterInstance) {
    let deployer = ctx.addresses[0];
    let tx = contract.increment().from(deployer).into_transaction_request();
    let gas = ctx.alloy_provider.estimate_gas(tx).await.unwrap();
    assert!(gas > 40_000);
}

async fn test_eth_getBlockByHash(ctx: &SignetTestContext, _contract: &TestCounterInstance) {
    let genesis = ctx.factory.block(0.into()).unwrap().unwrap();

    let block = ctx.alloy_provider.get_block_by_hash(genesis.hash_slow()).await.unwrap().unwrap();
    assert_eq!(block.header.number, genesis.number);
    assert_eq!(block.header.timestamp, genesis.timestamp);
}

async fn test_eth_getBlockByNumber(ctx: &SignetTestContext, _contract: &TestCounterInstance) {
    let db_block = ctx.factory.block(1.into()).unwrap().unwrap();

    let rpc_block =
        ctx.alloy_provider.get_block_by_number(db_block.number.into()).await.unwrap().unwrap();
    assert_eq!(rpc_block.header.number, db_block.number);
    assert_eq!(rpc_block.header.timestamp, db_block.timestamp);
    assert_eq!(rpc_block.header.hash, db_block.hash_slow());
}

async fn test_eth_getTransactionByHash(ctx: &SignetTestContext, _contract: &TestCounterInstance) {
    let deployer = ctx.addresses[0];

    let deploy_tx = &ctx.factory.transactions_by_block(1.into()).unwrap().unwrap()[0];
    let tx_hash = *deploy_tx.hash();

    let rpc_tx = ctx.alloy_provider.get_transaction_by_hash(tx_hash).await.unwrap().unwrap();
    assert_eq!(rpc_tx.tx_hash(), tx_hash);
    assert_eq!(rpc_tx.from(), deployer);
    assert_eq!(rpc_tx.to(), None);
    assert_eq!(rpc_tx.value(), U256::ZERO);
    assert_eq!(rpc_tx.input(), &COUNTER_DEPLOY_CODE);
    assert_eq!(rpc_tx.block_number, Some(1));
    assert_eq!(rpc_tx.transaction_index, Some(0));
}

async fn test_eth_getTransactionReceipt(ctx: &SignetTestContext, contract: &TestCounterInstance) {
    let deployer = ctx.addresses[0];

    let deploy_tx = &ctx.factory.transactions_by_block(1.into()).unwrap().unwrap()[0];
    let tx_hash = *deploy_tx.hash();

    let receipt = ctx.alloy_provider.get_transaction_receipt(tx_hash).await.unwrap().unwrap();

    assert_eq!(receipt.transaction_hash, tx_hash);
    assert_eq!(receipt.transaction_index.unwrap(), 0);
    assert_eq!(receipt.block_number.unwrap(), 1);
    assert_eq!(receipt.from, deployer);
    assert_eq!(receipt.to, None);
    assert_eq!(receipt.contract_address, Some(*contract.address()));
    assert!(receipt.status());
}

// --- TESTS BELOW HERE RUN A TX ---
// These tests are split into pre-state assertions and post-state assertions.

#[serial]
#[tokio::test]
// tests
// - eth_getStorageAt
// - eth_getTransactionCount
// - eth_newBlockFilter
// - eth_getFilterChanges
// - eth_newFilter
// - eth_getLogs
async fn test_stateful_rpc_calls() {
    rpc_test(|ctx, contract| async move {
        let deployer = ctx.addresses[0];

        let (_, _, nonce, block_filter, event_filter) = tokio::join!(
            withBlock_pre(&ctx, &contract),
            getStorageAt_pre(&ctx, &contract),
            getTransactionCount_pre(&ctx, deployer),
            newBlockFilter_pre(&ctx),
            newFilter_pre(&ctx, &contract),
        );

        let latest_block = ctx.alloy_provider.get_block_number().await.unwrap();
        tracing::info!(latest_block, "latest block");

        let tx = contract.increment().from(deployer).into_transaction_request();
        let _ = ctx.process_alloy_tx(&tx).await.unwrap();

        tokio::join!(
            withBlock_post(&ctx, &contract),
            getStorageAt_post(&ctx, &contract),
            getTransactionCount_post(&ctx, deployer, nonce),
            newBlockFilter_post(&ctx, block_filter),
            newFilter_post(&ctx, &contract, event_filter),
            getLogs_post(&ctx, &contract),
        );

        ctx
    })
    .await;
}

async fn getLogs_post(ctx: &SignetTestContext, contract: &TestCounterInstance) {
    let latest_block = ctx.factory.last_block_number().unwrap();
    let latest_hash = ctx.factory.block(latest_block.into()).unwrap().unwrap().hash_slow();

    let logs = ctx
        .alloy_provider
        .get_logs(&Filter {
            block_option: FilterBlockOption::AtBlockHash(latest_hash),
            address: (*contract.address()).into(),
            topics: Default::default(),
        })
        .await
        .unwrap();

    assert_eq!(logs.len(), 1);
    let log_inner = &logs[0].inner;
    assert_eq!(log_inner.address, *contract.address());
    assert_eq!(log_inner.topics(), &[Counter::Count::SIGNATURE_HASH, B256::with_last_byte(1)]);
}

async fn newFilter_pre(ctx: &SignetTestContext, contract: &TestCounterInstance) -> U256 {
    ctx.alloy_provider
        .new_filter(&Filter {
            block_option: Default::default(),
            address: (*contract.address()).into(),
            topics: Default::default(),
        })
        .await
        .unwrap()
}

async fn newFilter_post(ctx: &SignetTestContext, contract: &TestCounterInstance, filter_id: U256) {
    let logs: Vec<Log<LogData>> = ctx.alloy_provider.get_filter_changes(filter_id).await.unwrap();
    assert_eq!(logs.len(), 1);
    let log_inner = &logs[0].inner;
    assert_eq!(log_inner.address, *contract.address());
    assert_eq!(log_inner.topics(), &[Counter::Count::SIGNATURE_HASH, B256::with_last_byte(1)]);
    assert_eq!(log_inner.data.data, Bytes::new());
}

async fn newBlockFilter_pre(ctx: &SignetTestContext) -> U256 {
    ctx.alloy_provider.new_block_filter().await.unwrap()
}

async fn newBlockFilter_post(ctx: &SignetTestContext, filter_id: U256) {
    let blocks: Vec<B256> = ctx.alloy_provider.get_filter_changes(filter_id).await.unwrap();
    let latest_block = ctx.factory.last_block_number().unwrap();
    let latest_hash = ctx.factory.block(latest_block.into()).unwrap().unwrap().hash_slow();

    tracing::info!(latest_block, "huh");

    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0], latest_hash);
}

async fn getTransactionCount_pre(ctx: &SignetTestContext, deployer: Address) -> u64 {
    // rpc returns correct nonce
    let nonce = ctx.nonce(deployer);
    assert_eq!(ctx.alloy_provider.get_transaction_count(deployer).await.unwrap(), nonce);
    nonce
}

async fn getTransactionCount_post(ctx: &SignetTestContext, deployer: Address, nonce: u64) {
    // nonce increased
    assert_eq!(ctx.alloy_provider.get_transaction_count(deployer).await.unwrap(), nonce + 1);
}

async fn getStorageAt_pre(ctx: &SignetTestContext, contract: &TestCounterInstance) {
    // rpc returns correct storage
    assert_eq!(
        ctx.alloy_provider.get_storage_at(*contract.address(), U256::ZERO).await.unwrap(),
        U256::ZERO
    );
}

async fn getStorageAt_post(ctx: &SignetTestContext, contract: &TestCounterInstance) {
    // storage updated
    assert_eq!(
        ctx.alloy_provider.get_storage_at(*contract.address(), U256::ZERO).await.unwrap(),
        U256::from(1)
    );
}

async fn withBlock_pre(ctx: &SignetTestContext, contract: &TestCounterInstance) {
    // We should be at block 1
    assert_eq!(ctx.alloy_provider.get_block_number().await.unwrap(), 1);

    // Get the block hashes for block 0 and block 1
    let bh_0 = ctx.alloy_provider.get_block_by_number(0.into()).await.unwrap().unwrap().hash();
    let bh_1 = ctx.alloy_provider.get_block_by_number(1.into()).await.unwrap().unwrap().hash();

    // Code at block 0, should be empty
    assert!(
        ctx.alloy_provider
            .get_code_at(*contract.address())
            .block_id(0.into())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        ctx.alloy_provider
            .get_code_at(*contract.address())
            .block_id(bh_0.into())
            .await
            .unwrap()
            .is_empty()
    );

    // Code at block 1, should be the counter bytecode
    assert_eq!(
        ctx.alloy_provider.get_code_at(*contract.address()).await.unwrap(),
        COUNTER_BYTECODE
    );
    assert_eq!(
        ctx.alloy_provider.get_code_at(*contract.address()).block_id(1.into()).await.unwrap(),
        COUNTER_BYTECODE
    );
    assert_eq!(
        ctx.alloy_provider.get_code_at(*contract.address()).block_id(bh_1.into()).await.unwrap(),
        COUNTER_BYTECODE
    );

    // The call at block 0 should fail
    assert!(matches!(
        contract.count().call().block(0.into()).await.unwrap_err(),
        alloy::contract::Error::ZeroData(_, _,)
    ));
    assert!(matches!(
        contract.count().call().block(bh_0.into()).await.unwrap_err(),
        alloy::contract::Error::ZeroData(_, _,)
    ));

    // The call at block 1 should return 0
    assert_eq!(contract.count().call().await.unwrap(), U256::ZERO);
    assert_eq!(contract.count().call().block(1.into()).await.unwrap(), U256::ZERO);
    assert_eq!(contract.count().call().block(bh_1.into()).await.unwrap(), U256::ZERO);
}

async fn withBlock_post(ctx: &SignetTestContext, contract: &TestCounterInstance) {
    // After the increment, we should be at block 2
    assert_eq!(ctx.alloy_provider.get_block_number().await.unwrap(), 2);

    // Get the block hashes for blocks 0..=2
    let bh_0 = ctx.alloy_provider.get_block_by_number(0.into()).await.unwrap().unwrap().hash();
    let bh_1 = ctx.alloy_provider.get_block_by_number(1.into()).await.unwrap().unwrap().hash();
    let bh_2 = ctx.alloy_provider.get_block_by_number(2.into()).await.unwrap().unwrap().hash();

    // Code at block 0, should be empty
    assert!(
        ctx.alloy_provider
            .get_code_at(*contract.address())
            .block_id(0.into())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        ctx.alloy_provider
            .get_code_at(*contract.address())
            .block_id(bh_0.into())
            .await
            .unwrap()
            .is_empty()
    );

    // Code at block 1, should be the counter bytecode
    assert_eq!(
        ctx.alloy_provider.get_code_at(*contract.address()).block_id(1.into()).await.unwrap(),
        COUNTER_BYTECODE
    );
    assert_eq!(
        ctx.alloy_provider.get_code_at(*contract.address()).block_id(bh_1.into()).await.unwrap(),
        COUNTER_BYTECODE
    );

    // Code at block 2, should be the counter bytecode
    assert_eq!(
        ctx.alloy_provider.get_code_at(*contract.address()).await.unwrap(),
        COUNTER_BYTECODE
    );
    assert_eq!(
        ctx.alloy_provider.get_code_at(*contract.address()).block_id(2.into()).await.unwrap(),
        COUNTER_BYTECODE
    );
    assert_eq!(
        ctx.alloy_provider.get_code_at(*contract.address()).block_id(bh_2.into()).await.unwrap(),
        COUNTER_BYTECODE
    );

    // The call at block 0 should fail
    assert!(matches!(
        contract.count().call().block(0.into()).await.unwrap_err(),
        alloy::contract::Error::ZeroData(_, _,)
    ));
    assert!(matches!(
        contract.count().call().block(bh_0.into()).await.unwrap_err(),
        alloy::contract::Error::ZeroData(_, _,)
    ));

    // The call at block 1 should return 0
    assert_eq!(contract.count().call().block(1.into()).await.unwrap(), U256::ZERO);
    assert_eq!(contract.count().call().block(bh_1.into()).await.unwrap(), U256::ZERO);

    // The call at block 2 should return 1
    assert_eq!(contract.count().call().await.unwrap(), U256::from(1));
    assert_eq!(contract.count().call().block(2.into()).await.unwrap(), U256::from(1));
    assert_eq!(contract.count().call().block(bh_2.into()).await.unwrap(), U256::from(1));
}

#[ignore = "This test is slow and should not run by default"]
#[serial]
#[tokio::test]
async fn test_withBlock_many_blocks() {
    rpc_test(|ctx, contract| async move {
        let deployer = ctx.addresses[0];

        // Process 100 transactions
        for i in 0..300 {
            let tx = contract.increment().from(deployer).into_transaction_request();
            let _ = ctx.process_alloy_tx(&tx).await.unwrap();
            if i % 10 == 0 {
                tracing::info!("Processed {} transactions", i);
            }
        }

        // We should have 10 blocks
        let block_no = ctx.alloy_provider.get_block_number().await.unwrap();
        assert!(block_no >= 300);

        // Make an eth_call at latest
        let counter = contract.count().call().await.unwrap();
        assert_eq!(counter, U256::from(300));

        for i in 1..=300 {
            let block_no = block_no - i;
            let counter = contract.count().call().block(block_no.into()).await.unwrap();
            assert_eq!(counter, U256::from(300 - i));
            if i % 10 == 0 {
                tracing::info!("Reverse {} transactions", i);
            }
        }

        ctx
    })
    .await;
}

// -- TESTS BELOW HERE TEST THE PUBSUB API --

#[serial]
#[tokio::test]
async fn test_rpc_pubsub() {
    rpc_test(|ctx, contract| async move {
        let deployer = ctx.addresses[0];

        let (block_sub, event_sub) =
            tokio::join!(subscribe_blocks_pre(&ctx), subscribe_logs_pre(&ctx, &contract),);

        let tx = contract.increment().from(deployer).into_transaction_request();
        let _ = ctx.process_alloy_tx(&tx).await.unwrap();

        tokio::join!(
            subscribe_blocks_post(&ctx, block_sub),
            subscribe_logs_post(&contract, event_sub),
        );

        ctx
    })
    .await;
}

async fn subscribe_blocks_pre(ctx: &SignetTestContext) -> Subscription<Header> {
    ctx.alloy_provider.subscribe_blocks().await.unwrap()
}

async fn subscribe_blocks_post(ctx: &SignetTestContext, mut sub: Subscription<Header>) {
    let block = sub.recv().await.unwrap();

    let latest_block = ctx.factory.last_block_number().unwrap();
    let latest_hash = ctx.factory.block(latest_block.into()).unwrap().unwrap().hash_slow();
    assert_eq!(block.number, latest_block);
    assert_eq!(block.hash, latest_hash);
}

async fn subscribe_logs_pre(
    ctx: &SignetTestContext,
    contract: &TestCounterInstance,
) -> Subscription<Log> {
    ctx.alloy_provider
        .subscribe_logs(&Filter {
            block_option: Default::default(),
            address: (*contract.address()).into(),
            topics: Default::default(),
        })
        .await
        .unwrap()
}

async fn subscribe_logs_post(contract: &TestCounterInstance, mut sub: Subscription<Log>) {
    let log = sub.recv().await.unwrap();
    let log_inner = log.inner;
    assert_eq!(log_inner.address, *contract.address());
    assert_eq!(log_inner.topics(), &[Counter::Count::SIGNATURE_HASH, B256::with_last_byte(1)]);
    assert_eq!(log_inner.data.data, Bytes::new());
}

// -- THIS IS TO TEST FILTER EDGES ACROSS A RANGE OF BLOCKS --
#[serial]
#[tokio::test]
async fn test_rpc_filter_edge_cases() {
    rpc_test(|ctx, contract| async move {
        let deployer = ctx.addresses[0];

        let (block_filter, event_filter) =
            tokio::join!(newBlockFilter_pre(&ctx), newFilter_pre(&ctx, &contract));

        let tx = contract.increment().from(deployer).into_transaction_request();
        let _ = ctx.process_alloy_tx(&tx).await.unwrap();

        // After this, each filter should have 1 item
        let (blocks, logs) = tokio::try_join!(
            ctx.alloy_provider.get_filter_changes::<B256>(block_filter),
            ctx.alloy_provider.get_filter_changes::<Log<LogData>>(event_filter),
        )
        .unwrap();

        assert_eq!(blocks.len(), 1);
        assert_eq!(logs.len(), 1);
        // the counter in the event should be 1
        assert_eq!(logs[0].inner.topics()[1], B256::with_last_byte(1));

        // Process 2 more transactions
        let _ = ctx.process_alloy_tx(&tx).await.unwrap();
        let _ = ctx.process_alloy_tx(&tx).await.unwrap();

        // After this, each filter should have 2 items
        let (blocks, logs) = tokio::try_join!(
            ctx.alloy_provider.get_filter_changes::<B256>(block_filter),
            ctx.alloy_provider.get_filter_changes::<Log<LogData>>(event_filter),
        )
        .unwrap();

        assert_eq!(blocks.len(), 2);
        assert_eq!(logs.len(), 2);
        // the counters in the events should be 2 and 3
        assert_eq!(logs[0].inner.topics()[1], B256::with_last_byte(2));
        assert_eq!(logs[1].inner.topics()[1], B256::with_last_byte(3));

        // Poll again
        let (blocks, logs) = tokio::try_join!(
            ctx.alloy_provider.get_filter_changes::<B256>(block_filter),
            ctx.alloy_provider.get_filter_changes::<Log<LogData>>(event_filter),
        )
        .unwrap();

        // Now they should be empty
        assert_eq!(blocks.len(), 0);
        assert_eq!(logs.len(), 0);

        ctx
    })
    .await;
}
