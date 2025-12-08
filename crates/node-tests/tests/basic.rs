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
use serial_test::serial;
use signet_constants::RollupPermitted;
use signet_genesis::GenesisSpec;
use signet_node_tests::{HostBlockSpec, run_test, utils::adjust_usd_decimals};

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
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_genesis_allocs() {
    run_test(|ctx| async move {
        let genesis = GenesisSpec::Test.load_genesis().expect("Failed to load genesis");
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
