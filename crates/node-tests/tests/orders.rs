use alloy::{
    consensus::constants::GWEI_TO_WEI,
    eips::BlockNumberOrTag,
    primitives::{Address, U256},
    providers::Provider,
    rpc::types::eth::TransactionRequest,
    sol_types::{SolCall, SolEvent},
};
use serial_test::serial;
use signet_node_tests::{
    HostBlockSpec, RuBlockSpec, SignetTestContext, run_test,
    types::{Erc20, TestErc20Instance},
};
use signet_types::constants::HostPermitted;
use signet_zenith::RollupOrders;

#[serial]
#[tokio::test]
async fn test_eth_order_success() {
    run_test(|ctx| async move {
        let user = ctx.addresses[0];
        let host_token = Address::repeat_byte(0x38);
        let host_token_amnt = U256::from(100_000_000);
        let eth_in_amnt = U256::from(GWEI_TO_WEI);

        let mut bal = ctx.track_balance(user, Some("user"));
        let mut order_bal = ctx.track_balance(ctx.constants.rollup().orders(), Some("orders"));

        let input = RollupOrders::Input { token: Address::ZERO, amount: eth_in_amnt };
        let output = RollupOrders::Output {
            chainId: ctx.constants.host().chain_id() as u32,
            recipient: user,
            token: host_token,
            amount: host_token_amnt,
        };

        let tx = TransactionRequest::default()
            .from(user)
            .to(ctx.constants.rollup().orders())
            .value(eth_in_amnt)
            .input(
                RollupOrders::initiateCall {
                    deadline: U256::MAX,
                    inputs: vec![input],
                    outputs: vec![output],
                }
                .abi_encode()
                .into(),
            );
        let tx = ctx.fill_alloy_tx(&tx).await.unwrap();

        let block = HostBlockSpec::new(ctx.constants())
            .fill(host_token, user, 100_000_000)
            .submit_block(RuBlockSpec::new(ctx.constants()).alloy_tx(&tx));

        ctx.process_block(block).await.unwrap();

        order_bal.assert_increase_exact(eth_in_amnt);
        bal.assert_decrease_at_least(eth_in_amnt);

        let events = ctx.ru_orders_contract().Order_filter().query().await.unwrap();
        let order = &events[0].0;
        assert_eq!(order.inputs[0], input);
        assert_eq!(order.outputs[0], output);

        let block = ctx
            .alloy_provider
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await
            .unwrap()
            .unwrap();
        let tx = *block.transactions.as_hashes().unwrap().first().unwrap();
        let receipt = ctx.alloy_provider.get_transaction_receipt(tx).await.unwrap().unwrap();

        assert!(receipt.status());
        assert_eq!(receipt.inner.logs().len(), 1);
        assert_eq!(receipt.inner.logs()[0].address(), *ctx.ru_orders_contract().address());
        assert_eq!(receipt.inner.logs()[0].topic0(), Some(&RollupOrders::Order::SIGNATURE_HASH));
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_eth_order_failure() {
    run_test(|ctx| async move {
        let user = ctx.addresses[0];
        let host_token = Address::repeat_byte(0x38);
        let host_token_amnt = U256::from(100_000_000);
        let eth_in_amnt = U256::from(GWEI_TO_WEI);

        let mut bal = ctx.track_balance(user, Some("user"));
        let mut order_bal = ctx.track_balance(ctx.constants.rollup().orders(), Some("orders"));

        let input = RollupOrders::Input { token: Address::ZERO, amount: eth_in_amnt };
        let output = RollupOrders::Output {
            chainId: ctx.constants.host().chain_id() as u32,
            recipient: user,
            token: host_token,
            amount: host_token_amnt,
        };

        let tx = TransactionRequest::default()
            .from(user)
            .to(ctx.constants.rollup().orders())
            .value(eth_in_amnt)
            .input(
                RollupOrders::initiateCall {
                    deadline: U256::MAX,
                    inputs: vec![input],
                    outputs: vec![output],
                }
                .abi_encode()
                .into(),
            );

        let e = ctx.process_alloy_tx(&tx).await.unwrap_err();
        assert!(e.to_string().contains("no receipt"));

        order_bal.assert_no_change();
        bal.assert_no_change();

        let block = ctx
            .alloy_provider
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await
            .unwrap()
            .unwrap();

        // transaction did not occur
        assert!(block.transactions.as_hashes().unwrap().is_empty());
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_token_order_success() {
    run_test(|mut ctx| async move {
        ctx = test_inner(ctx, HostPermitted::Weth).await;
        test_inner(ctx, HostPermitted::Wbtc).await;
    })
    .await;

    async fn test_inner(ctx: SignetTestContext, token: HostPermitted) -> SignetTestContext {
        let user = ctx.addresses[0];
        let host_token_addr = ctx.constants.host().tokens().address_for(token);
        let amount = U256::from(10_000);

        let ru_token_addr = ctx.constants.rollup().tokens().address_for(token.into());
        let ru_token = TestErc20Instance::new(ru_token_addr, ctx.alloy_provider.clone());

        ctx.mint_token(token, user, amount.to()).await.unwrap();

        let input = RollupOrders::Input { token: ru_token_addr, amount };
        let output = RollupOrders::Output {
            chainId: ctx.constants.host().chain_id() as u32,
            recipient: user,
            token: host_token_addr,
            amount,
        };

        // approve the orders contract to spend tokens
        let approve_tx = TransactionRequest::default()
            .from(user)
            .to(ctx.constants.rollup().tokens().address_for(token.into()))
            .input(
                Erc20::approveCall { _0: ctx.constants.rollup().orders(), _1: U256::MAX }
                    .abi_encode()
                    .into(),
            );
        ctx.process_alloy_tx(&approve_tx).await.unwrap();

        let tx =
            TransactionRequest::default().from(user).to(ctx.constants.rollup().orders()).input(
                RollupOrders::initiateCall {
                    deadline: U256::MAX,
                    inputs: vec![input],
                    outputs: vec![output],
                }
                .abi_encode()
                .into(),
            );
        let tx = ctx.fill_alloy_tx(&tx).await.unwrap();

        let block = HostBlockSpec::new(ctx.constants())
            .fill(host_token_addr, user, 100_000_000)
            .submit_block(RuBlockSpec::new(ctx.constants()).alloy_tx(&tx));

        ctx.process_block(block).await.unwrap();

        // tokens are transferred to the orders contract
        assert_eq!(ru_token.balanceOf(user).call().await.unwrap(), U256::ZERO);
        assert_eq!(
            ru_token.balanceOf(ctx.constants.rollup().orders()).call().await.unwrap(),
            amount
        );

        // RPC shows the tx and receipt and event
        let block = ctx
            .alloy_provider
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await
            .unwrap()
            .unwrap();

        let tx = *block.transactions.as_hashes().unwrap().first().unwrap();

        let receipt = ctx.alloy_provider.get_transaction_receipt(tx).await.unwrap().unwrap();

        assert!(receipt.status());
        assert!(receipt.inner.logs().len() > 1);
        let order = receipt.inner.logs().last().unwrap();
        assert_eq!(order.address(), *ctx.ru_orders_contract().address());
        assert_eq!(order.topic0(), Some(&RollupOrders::Order::SIGNATURE_HASH));

        ctx
    }
}

#[serial]
#[tokio::test]
async fn test_token_order_failure() {
    run_test(|mut ctx| async move {
        ctx = test_inner(ctx, HostPermitted::Weth).await;
        test_inner(ctx, HostPermitted::Wbtc).await;
    })
    .await;

    async fn test_inner(ctx: SignetTestContext, token: HostPermitted) -> SignetTestContext {
        let user = ctx.addresses[0];
        let host_token_addr = ctx.constants.host().tokens().address_for(token);
        let amount = U256::from(10_000);

        let ru_token_addr = ctx.constants.rollup().tokens().address_for(token.into());
        let ru_token = TestErc20Instance::new(ru_token_addr, ctx.alloy_provider.clone());

        // mint some tokens
        ctx.mint_token(token, user, amount.to()).await.unwrap();

        // approve the orders contract to spend tokens
        let approve_tx = TransactionRequest::default()
            .from(user)
            .to(ctx.constants.rollup().tokens().address_for(token.into()))
            .input(
                Erc20::approveCall { _0: ctx.constants.rollup().orders(), _1: U256::MAX }
                    .abi_encode()
                    .into(),
            );
        ctx.process_alloy_tx(&approve_tx).await.unwrap();

        // track bal before tx, after approve
        let mut bal = ctx.track_balance(user, Some("user"));

        // initiate the order
        let input = RollupOrders::Input { token: ru_token_addr, amount };
        let output = RollupOrders::Output {
            chainId: ctx.constants.host().chain_id() as u32,
            recipient: user,
            token: host_token_addr,
            amount,
        };

        let tx =
            TransactionRequest::default().from(user).to(ctx.constants.rollup().orders()).input(
                RollupOrders::initiateCall {
                    deadline: U256::MAX,
                    inputs: vec![input],
                    outputs: vec![output],
                }
                .abi_encode()
                .into(),
            );
        let e = ctx.process_alloy_tx(&tx).await.unwrap_err();
        assert!(e.to_string().contains("no receipt"));

        // user did not pay gas
        bal.assert_no_change();

        // tokens are not transferred to the orders contract
        assert_eq!(
            ru_token.balanceOf(ctx.constants.rollup().orders()).call().await.unwrap(),
            U256::ZERO
        );
        assert_eq!(ru_token.balanceOf(user).call().await.unwrap(), amount);

        let block = ctx
            .alloy_provider
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await
            .unwrap()
            .unwrap();

        // transaction is not in the block
        assert!(block.transactions.as_hashes().unwrap().is_empty());

        ctx
    }
}

#[serial]
#[tokio::test]
async fn fill_two_orders() {
    run_test(|ctx| async move {
        let user = ctx.addresses[0];
        let user_2 = ctx.addresses[1];

        let host_token = Address::repeat_byte(0x38);
        let host_token_amnt = U256::from(100_000_000);

        let eth_in_amnt = U256::from(GWEI_TO_WEI);

        // Note that both orders pay to the same recipient
        let input = RollupOrders::Input { token: Address::ZERO, amount: eth_in_amnt };
        let output = RollupOrders::Output {
            chainId: ctx.constants.host().chain_id() as u32,
            recipient: user,
            token: host_token,
            amount: host_token_amnt,
        };

        let tx_1 = TransactionRequest::default()
            .from(user)
            .to(ctx.constants.rollup().orders())
            .value(eth_in_amnt)
            .input(
                RollupOrders::initiateCall {
                    deadline: U256::MAX,
                    inputs: vec![input],
                    outputs: vec![output],
                }
                .abi_encode()
                .into(),
            );
        let tx_1 = ctx.fill_alloy_tx(&tx_1).await.unwrap();

        let tx_2 = TransactionRequest::default()
            .from(user_2)
            .to(ctx.constants.rollup().orders())
            .value(eth_in_amnt)
            .input(
                RollupOrders::initiateCall {
                    deadline: U256::MAX,
                    inputs: vec![input],
                    outputs: vec![output],
                }
                .abi_encode()
                .into(),
            );
        let tx_2 = ctx.fill_alloy_tx(&tx_2).await.unwrap();

        // fill both orders at once
        let block = HostBlockSpec::new(ctx.constants())
            .fill(host_token, user, 100_000_000 * 2)
            .submit_block(RuBlockSpec::new(ctx.constants()).alloy_tx(&tx_1).alloy_tx(&tx_2));

        ctx.process_block(block).await.unwrap();

        // check that the orders executed
        let events = ctx.ru_orders_contract().Order_filter().query().await.unwrap();
        let order_1 = &events[0].0;
        let order_2 = &events[1].0;

        assert_eq!(order_1.inputs[0], input);
        assert_eq!(order_1.outputs[0], output);

        assert_eq!(order_2.inputs[0], input);
        assert_eq!(order_2.outputs[0], output);
    })
    .await;
}

// Test that if fill is insufficient, the first order will work, and the second
// won't
#[serial]
#[tokio::test]
async fn fill_two_orders_second_incomplete() {
    run_test(|ctx| async move {
        let user = ctx.addresses[0];
        let user_2 = ctx.addresses[1];

        let host_token = Address::repeat_byte(0x38);
        let host_token_amnt_1 = U256::from(100_000_000);
        let host_token_amnt_2 = U256::from(50_000_000);

        let eth_in_amnt = U256::from(GWEI_TO_WEI);

        let input = RollupOrders::Input { token: Address::ZERO, amount: eth_in_amnt };

        // Note that both orders pay to the same recipient
        let output_1 = RollupOrders::Output {
            chainId: ctx.constants.host().chain_id() as u32,
            recipient: user,
            token: host_token,
            amount: host_token_amnt_1,
        };
        let output_2 = RollupOrders::Output {
            chainId: ctx.constants.host().chain_id() as u32,
            recipient: user,
            token: host_token,
            amount: host_token_amnt_2,
        };

        let tx_1 = TransactionRequest::default()
            .from(user)
            .to(ctx.constants.rollup().orders())
            .value(eth_in_amnt)
            .input(
                RollupOrders::initiateCall {
                    deadline: U256::MAX,
                    inputs: vec![input],
                    outputs: vec![output_1],
                }
                .abi_encode()
                .into(),
            );

        let tx_2 = TransactionRequest::default()
            .from(user_2)
            .to(ctx.constants.rollup().orders())
            .value(eth_in_amnt)
            .input(
                RollupOrders::initiateCall {
                    deadline: U256::MAX,
                    inputs: vec![input],
                    outputs: vec![output_2],
                }
                .abi_encode()
                .into(),
            );

        let tx_1 = ctx.fill_alloy_tx(&tx_1).await.unwrap();
        let tx_2 = ctx.fill_alloy_tx(&tx_2).await.unwrap();

        // the fill is sufficient for the first order, but not the second
        let block = HostBlockSpec::new(ctx.constants())
            .fill(host_token, user, (host_token_amnt_1 + host_token_amnt_2 - U256::from(1)).to())
            .submit_block(RuBlockSpec::new(ctx.constants()).alloy_tx(&tx_1).alloy_tx(&tx_2));

        ctx.process_block(block).await.unwrap();

        let events = ctx.ru_orders_contract().Order_filter().query().await.unwrap();
        assert_eq!(events.len(), 1);

        let block = ctx
            .alloy_provider
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(block.transactions.as_hashes().unwrap().len(), 1);
        let tx = *block.transactions.as_hashes().unwrap().first().unwrap();

        let receipt = ctx.alloy_provider.get_transaction_receipt(tx).await.unwrap().unwrap();
        assert!(receipt.status());
    })
    .await;
}
