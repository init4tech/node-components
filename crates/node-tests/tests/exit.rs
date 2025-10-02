use alloy::{
    eips::BlockNumberOrTag, primitives::U256, providers::Provider,
    rpc::types::eth::TransactionRequest, sol_types::SolCall,
};
use serial_test::serial;
use signet_node_tests::{HostBlockSpec, SignetTestContext, run_test};
use signet_types::constants::HostPermitted;
use signet_zenith::RollupPassage;

alloy::sol! {
    #[derive(Debug)]
    #[sol(rpc)]
    contract Erc20 {
        function balanceOf(address) public view returns (uint256);
        function approve(address, uint256) public returns (bool);
        function totalSupply() public returns (uint256);
        function allowance(address, address) public view returns (uint256);
    }
}

#[serial]
#[tokio::test]
async fn test_simple_exit() {
    run_test(|ctx| async move {
        let user = ctx.addresses[0];

        let mut bal = ctx.track_balance(user, Some("user"));
        let mut passage_bal = ctx.track_balance(ctx.constants.rollup().passage(), Some("passage"));
        passage_bal.assert_eq(U256::ZERO);

        let amount = U256::from(31999);

        let tx = TransactionRequest::default()
            .from(user)
            .to(ctx.constants.rollup().passage())
            .value(amount);

        ctx.process_alloy_tx(&tx).await.unwrap();

        bal.assert_decrease_at_least(amount);
        passage_bal.assert_no_change();

        let block = ctx
            .alloy_provider
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await
            .unwrap()
            .unwrap();

        let tx = *block.transactions.as_hashes().unwrap().first().unwrap();

        let passage = ctx.ru_passage_contract();
        let events = passage.Exit_filter().query().await.unwrap();
        let event = &events[0];

        assert_eq!(event.1.transaction_hash, Some(tx));
        assert_eq!(event.0, RollupPassage::Exit { hostRecipient: user, amount });
        assert_eq!(event.0.hostRecipient, user);
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_erc20_exits() {
    run_test(|mut ctx| async move {
        ctx = test_inner(ctx, HostPermitted::Weth).await;
        test_inner(ctx, HostPermitted::Wbtc).await;
    })
    .await;

    async fn test_inner(ctx: SignetTestContext, token: HostPermitted) -> SignetTestContext {
        // setup
        let amount = 100_000_000;
        let passage = ctx.ru_passage_contract();
        let user_a = ctx.addresses[0];

        let host_token = ctx.constants.host().tokens().address_for(token);
        let ru_token = Erc20::Erc20Instance::new(
            ctx.constants.rollup().tokens().address_for(token.into()),
            ctx.alloy_provider.clone(),
        );

        // Mint some tokens on the rollup for the user
        ctx.process_block(
            HostBlockSpec::new(ctx.constants()).enter_token(user_a, amount, host_token),
        )
        .await
        .unwrap();

        // Check that mint occurred
        let mint_bal = ru_token.balanceOf(user_a).call().await.unwrap();
        assert_eq!(mint_bal, U256::from(amount));
        let pre_supply = ru_token.totalSupply().call().await.unwrap();

        // Approve
        let approve_tx = TransactionRequest::default().from(user_a).to(*ru_token.address()).input(
            Erc20::approveCall { _0: *passage.address(), _1: U256::MAX }.abi_encode().into(),
        );
        let (_, receipt) = ctx.process_alloy_tx(&approve_tx).await.unwrap();
        assert!(receipt.status(), "approval tx reverted");

        // Now exit
        let exit_tx = TransactionRequest::default().from(user_a).to(*passage.address()).input(
            RollupPassage::exitTokenCall {
                hostRecipient: user_a,
                token: *ru_token.address(),
                amount: U256::from(amount),
            }
            .abi_encode()
            .into(),
        );

        ctx.process_alloy_tx(&exit_tx).await.unwrap();
        assert!(receipt.status(), "exit tx reverted");

        // Burn occurred
        assert_eq!(ru_token.balanceOf(user_a).call().await.unwrap(), U256::ZERO);
        assert_eq!(ru_token.balanceOf(*passage.address()).call().await.unwrap(), U256::ZERO);
        let post_supply = ru_token.totalSupply().call().await.unwrap();
        assert_eq!(
            post_supply,
            pre_supply - U256::from(amount),
            "total supply should decrease by the amount burned"
        );

        // Check that RPC records the events
        let get_block_by_number = ctx.alloy_provider.get_block_by_number(BlockNumberOrTag::Latest);
        let block = get_block_by_number.await.unwrap().unwrap();
        let tx = block.transactions.as_hashes().unwrap()[0];

        let events = passage.ExitToken_filter().query().await.unwrap();
        let event = &events[0];

        assert_eq!(event.1.transaction_hash, Some(tx));
        assert_eq!(
            event.0,
            RollupPassage::ExitToken {
                hostRecipient: user_a,
                token: *ru_token.address(),
                amount: U256::from(amount)
            }
        );

        ctx
    }
}
