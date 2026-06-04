use alloy::{
    consensus::{
        SidecarBuilder, SimpleCoder, TxType,
        constants::{ETH_TO_WEI, GWEI_TO_WEI},
    },
    network::{TransactionBuilder, TransactionBuilder4844, TransactionBuilder7702},
    primitives::{Address, B256, U256},
    providers::Provider,
    rpc::types::eth::{AccessList, AccessListItem, TransactionRequest},
    signers::Signer,
};
use core::{sync::atomic::Ordering, time::Duration};
use serial_test::serial;
use signet_constants::{KnownChains, RollupPermitted};
use signet_genesis::GenesisSpec;
use signet_node_tests::{
    HostBlockSpec, NotificationWithSidecars, SignetTestContext, run_test,
    utils::adjust_usd_decimals,
};

const SOME_USER: Address = Address::repeat_byte(0x39);

// Tests must be serial, as reth test exex context binds a peer discovery port
#[serial]
#[tokio::test]
async fn test_simple_enter() {
    run_test(|ctx| async move {
        let mut bal = ctx.track_balance(SOME_USER, Some("user"));

        let enter_amnt = 31999;
        let block = HostBlockSpec::new(ctx.constants()).enter_token(
            SOME_USER,
            enter_amnt,
            ctx.constants().host().tokens().usdc(),
        );

        ctx.process_block(block).await.unwrap();

        let expected = adjust_usd_decimals(enter_amnt, 6);

        bal.assert_increase_exact(expected);
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_basic_reorg() {
    run_test(|ctx| async move {
        // Reorg to height 0 is unsupported (the journal chain's ring buffer never stores
        // genesis); `on_host_revert` bails when storage would be wiped to 0. Process an
        // unrelated warmup block at height 1 first so the revert only takes us back to
        // height 1, not height 0.
        let warmup = HostBlockSpec::new(ctx.constants()).enter_token(
            Address::repeat_byte(0x40),
            1,
            ctx.constants().host().tokens().usdc(),
        );
        ctx.process_block(warmup).await.unwrap();

        let mut bal = ctx.track_balance(SOME_USER, Some("user"));

        let enter_amnt = 31999;
        let block = HostBlockSpec::new(ctx.constants()).enter_token(
            SOME_USER,
            enter_amnt,
            ctx.constants().host().tokens().usdc(),
        );

        ctx.process_block(block.clone()).await.unwrap();

        let change = adjust_usd_decimals(enter_amnt, 6);

        bal.assert_increase_exact(change);

        ctx.revert_block(block).await.unwrap();

        bal.assert_decrease_exact(change);

        // Process a fresh block on top of the surviving warmup. This exercises the post-revert
        // journal-hash continuity path: `previous_journal_hash` must read the persisted hash
        // from storage at height 1 so the chain can validate the replacement journal at
        // height 2 as a `Reorg`. If persistence is broken, this would fail with
        // `PreviousHashMismatch` or `ReorgParentEvicted`.
        let replacement_amnt = 12345;
        let replacement = HostBlockSpec::new(ctx.constants()).enter_token(
            SOME_USER,
            replacement_amnt,
            ctx.constants().host().tokens().usdc(),
        );
        ctx.process_block(replacement).await.unwrap();
        bal.assert_increase_exact(adjust_usd_decimals(replacement_amnt, 6));
    })
    .await;
}

// Run directly (not via `run_test`) because the node task is expected to terminate with an
// error and `run_test`'s wrapper would convert that into a test failure. `ctx.revert_block`
// also isn't usable here - it polls the RPC after the revert, but the bail tears the RPC
// down before that poll can complete.
#[serial]
#[tokio::test]
async fn test_revert_to_genesis_bails() {
    let (ctx, signet) = SignetTestContext::new().await;

    // Process a single block so the journal chain's ring buffer holds a tip but does not yet
    // contain an anchor at height 0.
    let block = HostBlockSpec::new(ctx.constants()).enter_token(
        SOME_USER,
        1,
        ctx.constants().host().tokens().usdc(),
    );
    let for_revert = block.clone();
    ctx.process_block(block).await.unwrap();

    // Send the revert directly so we don't depend on RPC liveness after the bail. Storage
    // drains to 0, tags rewind, reorg fires, then the node bails because the chain cannot
    // anchor the next post-revert journal at `target == 0`. Mirror `revert_block`'s height
    // bookkeeping: `fetch_sub` returns the pre-decrement height (= the block being reverted)
    // and rewinds `ctx.height` so any post-bail inspection sees a consistent value.
    for_revert.set_block_number(ctx.height.fetch_sub(1, Ordering::SeqCst));
    ctx.send_notification(NotificationWithSidecars::revert_single_block(for_revert)).await;

    let join_result = tokio::time::timeout(Duration::from_secs(10), signet)
        .await
        .expect("node did not bail within 10s")
        .expect("node task panicked");
    let error = join_result.expect_err("expected the node to bail after revert-to-genesis");
    let rendered = format!("{error:#}");
    assert!(rendered.contains("ring buffer no longer holds"), "unexpected error: {rendered}");
}

#[serial]
#[tokio::test]
async fn test_genesis_allocs() {
    run_test(|ctx| async move {
        let genesis =
            GenesisSpec::Known(KnownChains::Test).load_genesis().expect("Failed to load genesis");
        ctx.verify_allocs(&genesis.rollup);
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_legacy_tx_support() {
    run_test(|ctx| async move {
        let send_val = U256::from(ETH_TO_WEI);
        let mut bal = ctx.track_balance(ctx.addresses[1], Some("recipient"));

        let tx = TransactionRequest::default()
            .from(ctx.addresses[0])
            .to(ctx.addresses[1])
            .value(send_val)
            .gas_limit(21_000)
            .with_gas_price(GWEI_TO_WEI as u128);
        let (envelope, _receipt) = ctx.process_alloy_tx(&tx).await.unwrap();

        assert_eq!(envelope.tx_type(), TxType::Legacy);
        bal.assert_increase_exact(send_val);
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_eip1559_tx_support() {
    run_test(|ctx| async move {
        let send_val = U256::from(ETH_TO_WEI);
        let mut bal = ctx.track_balance(ctx.addresses[1], Some("recipient"));

        let tx = TransactionRequest::default()
            .from(ctx.addresses[0])
            .to(ctx.addresses[1])
            .value(send_val)
            .gas_limit(21_000);
        let (envelope, _receipt) = ctx.process_alloy_tx(&tx).await.unwrap();

        assert_eq!(envelope.tx_type(), TxType::Eip1559);
        bal.assert_increase_exact(send_val);
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_eip2930_tx_support() {
    run_test(|ctx| async move {
        let send_val = U256::from(ETH_TO_WEI);
        let mut bal = ctx.track_balance(ctx.addresses[1], Some("recipient"));

        let tx = TransactionRequest::default()
            .from(ctx.addresses[0])
            .to(ctx.addresses[1])
            .value(send_val)
            .with_gas_price(GWEI_TO_WEI as u128)
            .with_access_list(AccessList::from(vec![AccessListItem {
                address: ctx.addresses[0],
                storage_keys: vec![B256::repeat_byte(3)],
            }]));
        let (envelope, _receipt) = ctx.process_alloy_tx(&tx).await.unwrap();

        assert_eq!(envelope.tx_type(), TxType::Eip2930);
        bal.assert_increase_exact(send_val);
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_eip4844_tx_unsupported() {
    run_test(|ctx| async move {
        let send_val = U256::from(ETH_TO_WEI);

        let sidecar = SidecarBuilder::<SimpleCoder>::from_slice(&[1, 2, 3, 4]).build().unwrap();

        let tx = TransactionRequest::default()
            .from(ctx.addresses[0])
            .to(ctx.addresses[1])
            .value(send_val)
            .with_blob_sidecar(sidecar);

        assert!(ctx.process_alloy_tx(&tx).await.is_err());
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_eip7702_tx_support() {
    run_test(|ctx| async move {
        let alice = signet_test_utils::users::TEST_SIGNERS[0].clone();
        let bob = signet_test_utils::users::TEST_SIGNERS[1].clone();

        // Deploy the log contract
        let log = ctx.deploy_log(alice.address()).await;

        // Create the authorization that bob will sign
        let authorization = alloy::eips::eip7702::Authorization {
            chain_id: U256::ZERO,
            // Reference to the contract that will be set as code for the authority
            address: *log.address(),
            nonce: ctx.alloy_provider.get_transaction_count(bob.address()).await.unwrap(),
        };

        let signature = bob.sign_hash(&authorization.signature_hash()).await.unwrap();
        let authorization = authorization.into_signed(signature);

        let tx = TransactionRequest::default()
            .from(alice.address())
            .to(bob.address())
            .with_authorization_list(vec![authorization])
            .with_input(log.emitHello().calldata().to_owned());

        let (envelope, receipt) = ctx.process_alloy_tx(&tx).await.unwrap();

        assert_eq!(envelope.tx_type(), TxType::Eip7702);
        assert!(receipt.status());
        assert_eq!(receipt.logs().len(), 1);
        assert_eq!(receipt.logs()[0].address(), bob.address());
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_predeployed_tokens() {
    run_test(|ctx| async move {
        let wbtc = ctx.token_instance(RollupPermitted::Wbtc);
        assert_eq!(wbtc.name().call().await.unwrap(), "Wrapped BTC");
        assert_eq!(wbtc.symbol().call().await.unwrap(), "WBTC");
        assert_eq!(wbtc.decimals().call().await.unwrap(), 8);

        let weth = ctx.token_instance(RollupPermitted::Weth);
        assert_eq!(weth.name().call().await.unwrap(), "Wrapped Ether");
        assert_eq!(weth.symbol().call().await.unwrap(), "WETH");
        assert_eq!(weth.decimals().call().await.unwrap(), 18);
    })
    .await;
}
