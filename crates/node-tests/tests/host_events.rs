use alloy::{
    consensus::{
        Transaction,
        constants::{ETH_TO_WEI, GWEI_TO_WEI},
    },
    eips::BlockNumberOrTag,
    network::TransactionResponse,
    primitives::{Bytes, U256},
    providers::Provider,
    sol_types::SolCall,
};
use reth::providers::{BlockNumReader, DBProvider, HeaderProvider};
use serial_test::serial;
use signet_db::{DbSignetEvent, SignetEvents};
use signet_node_tests::{
    HostBlockSpec, SignetTestContext,
    aliases::{Counter, TestCounterInstance},
    constants::{DEFAULT_REWARD_ADDRESS, TEST_CONSTANTS},
    run_test,
    utils::{adjust_usd_decimals, adjust_usd_decimals_u256},
};
use signet_test_utils::{chain::USDC_RECORD, contracts::counter::COUNTER_BYTECODE};
use signet_types::constants::{HostPermitted, RollupPermitted};
use signet_zenith::{MINTER_ADDRESS, Passage, Transactor, mintCall};

alloy::sol! {
    #[derive(Debug)]
    #[sol(rpc)]
    contract Erc20 {
        function balanceOf(address) public view returns (uint256);
    }
}

#[serial]
#[tokio::test]
async fn test_enter_token() {
    run_test(|mut ctx| async move {
        ctx = test_inner_usd(ctx, HostPermitted::Usdc).await;
        ctx = test_inner_usd(ctx, HostPermitted::Usdt).await;
        ctx = test_inner_non_usd(ctx, HostPermitted::Weth).await;
        test_inner_non_usd(ctx, HostPermitted::Wbtc).await;
    })
    .await;

    async fn test_inner_usd(ctx: SignetTestContext, token: HostPermitted) -> SignetTestContext {
        let user_a = ctx.addresses[1];
        let host_token_addr = ctx.constants.host().tokens().address_for(token);
        let mut bal = ctx.track_balance(user_a, Some("user_a"));
        let mut nonce = ctx.track_nonce(MINTER_ADDRESS, Some("minter"));

        // Get the USD record for the host token, and compute the enter amount
        let usd_record = ctx.constants.host().usd_record(host_token_addr).unwrap();
        let enter_amount = U256::from(10_000);
        let expected_mint_amount = adjust_usd_decimals_u256(enter_amount, usd_record.decimals());

        let block =
            HostBlockSpec::new(ctx.constants()).enter_token(user_a, 10_000, host_token_addr);
        ctx.process_block(block).await.unwrap();

        nonce.assert_incremented();
        bal.assert_increase_exact(expected_mint_amount);

        let block = ctx
            .alloy_provider
            .get_block_by_number(BlockNumberOrTag::Latest)
            .full()
            .await
            .unwrap()
            .unwrap();

        let txns = block.transactions.as_transactions().unwrap();
        let enter_tx = &txns[0];

        assert_eq!(enter_tx.from(), MINTER_ADDRESS);
        assert_eq!(enter_tx.to().unwrap(), user_a);
        assert_eq!(enter_tx.value(), expected_mint_amount);
        assert_eq!(enter_tx.input(), &Bytes::default());

        // NB: this check is probably redundant, but it ensures the RPC
        // tx response matches the tx in the block
        let rpc_tx =
            ctx.alloy_provider.get_transaction_by_hash(enter_tx.tx_hash()).await.unwrap().unwrap();
        assert_eq!(&rpc_tx, enter_tx);
        ctx
    }

    async fn test_inner_non_usd(ctx: SignetTestContext, token: HostPermitted) -> SignetTestContext {
        let user_a = ctx.addresses[0];
        let host_token = ctx.constants.host().tokens().address_for(token);
        let ru_token = Erc20::Erc20Instance::new(
            ctx.constants.rollup().tokens().address_for(token.into()),
            ctx.alloy_provider.clone(),
        );

        // Track minter's nonce
        let mut nonce = ctx.track_nonce(MINTER_ADDRESS, Some("minter"));

        let block = HostBlockSpec::new(ctx.constants()).enter_token(user_a, 10_000, host_token);

        // Track block fees
        let mut base_fee_recipient =
            ctx.track_balance(ctx.constants.base_fee_recipient(), Some("base_fee"));
        let mut beneficiary = ctx.track_balance(DEFAULT_REWARD_ADDRESS, Some("beneficiary"));

        let pre_bal = ru_token.balanceOf(user_a).call().await.unwrap();

        ctx.process_block(block).await.unwrap();

        // Minter's nonce should increase
        nonce.assert_incremented();

        // User's token balance should increase
        let balance = ru_token.balanceOf(user_a).call().await.unwrap();
        assert_eq!(balance, pre_bal + U256::from(10_000));

        // No fee should be paid
        base_fee_recipient.assert_no_change();
        beneficiary.assert_no_change();

        // Block
        let block = ctx
            .alloy_provider
            .get_block_by_number(BlockNumberOrTag::Latest)
            .full()
            .await
            .unwrap()
            .unwrap();
        let tx = &block.transactions.as_transactions().unwrap()[0];
        assert_eq!(tx.from(), MINTER_ADDRESS);
        assert_eq!(tx.to().unwrap(), *ru_token.address());
        assert_eq!(tx.value(), U256::ZERO);
        assert_eq!(
            tx.input().as_ref(),
            signet_zenith::mintCall { to: user_a, amount: U256::from(10_000) }.abi_encode()
        );

        let tx = ctx.alloy_provider.get_transaction_by_hash(tx.tx_hash()).await.unwrap().unwrap();
        assert_eq!(tx.from(), MINTER_ADDRESS);
        assert_eq!(tx.to().unwrap(), *ru_token.address());
        assert_eq!(tx.value(), U256::ZERO);
        assert_eq!(
            tx.input().as_ref(),
            signet_zenith::mintCall { to: user_a, amount: U256::from(10_000) }.abi_encode()
        );
        ctx
    }
}

#[serial]
#[tokio::test]
async fn test_enters() {
    run_test(|ctx| async move {
        test_enter_inner(ctx).await;
    })
    .await;

    async fn test_enter_inner(ctx: SignetTestContext) -> SignetTestContext {
        let mut nonce = ctx.track_nonce(MINTER_ADDRESS, Some("minter"));
        let ru_token = Erc20::Erc20Instance::new(
            ctx.constants.rollup().tokens().address_for(RollupPermitted::Weth),
            ctx.alloy_provider.clone(),
        );

        let user_a = ctx.addresses[1];

        let enter_amount = U256::from(31999);
        let pre_bal = ru_token.balanceOf(user_a).call().await.unwrap();

        let block = HostBlockSpec::new(ctx.constants()).enter(user_a, 31999);

        ctx.process_block(block).await.unwrap();

        let balance = ru_token.balanceOf(user_a).call().await.unwrap();
        assert_eq!(balance, pre_bal + U256::from(enter_amount));

        nonce.assert_incremented();

        let block = ctx
            .alloy_provider
            .get_block_by_number(BlockNumberOrTag::Latest)
            .full()
            .await
            .unwrap()
            .unwrap();

        let txns = block.transactions.as_transactions().unwrap();
        let enter_tx = &txns[0];
        let to = ctx.addresses[1];

        assert_eq!(enter_tx.from(), MINTER_ADDRESS);
        assert_eq!(enter_tx.to().unwrap(), *ru_token.address());
        assert!(enter_tx.value().is_zero());
        assert_eq!(enter_tx.input(), &mintCall { to, amount: enter_amount }.abi_encode());

        // NB: this check is probably redundant, but it ensures the RPC
        // tx response matches the tx in the block
        let rpc_tx =
            ctx.alloy_provider.get_transaction_by_hash(enter_tx.tx_hash()).await.unwrap().unwrap();
        assert_eq!(&rpc_tx, enter_tx);
        ctx
    }
}

#[serial]
#[tokio::test]
async fn test_transact() {
    run_test(|ctx| async move {
        // set up user
        let user = ctx.addresses[0];
        let mut user_nonce = ctx.track_nonce(user, Some("user"));
        let mut user_bal = ctx.track_balance(user, Some("user"));

        // Deploy a contract to interact with
        let deployer = ctx.addresses[1];

        // Assert that the counter is zero
        let contract = ctx.deploy_counter(deployer).await;
        let contract_addr = *contract.address();
        assert_eq!(contract.count().call().await.unwrap(), U256::ZERO);

        // Transact that calls the context and increments it.
        let block = HostBlockSpec::new(ctx.constants()).simple_transact(
            user,
            contract_addr,
            Counter::incrementCall::SELECTOR,
            0,
        );

        ctx.process_block(block).await.unwrap();

        user_nonce.assert_incremented();
        user_bal.assert_decrease();

        assert_eq!(contract.count().call().await.unwrap(), U256::from(1));

        // check the RPC response
        let block = ctx
            .alloy_provider
            .get_block_by_number(BlockNumberOrTag::Latest)
            .full()
            .await
            .unwrap()
            .unwrap();

        let txns = block.transactions.as_transactions().unwrap();
        let transact_tx = &txns[0];

        assert_eq!(transact_tx.from(), user);
        assert_eq!(transact_tx.to().unwrap(), contract_addr);
        assert_eq!(transact_tx.value(), U256::ZERO);
        assert_eq!(transact_tx.input(), &Bytes::from(Counter::incrementCall::SELECTOR));

        let tx_hash = transact_tx.tx_hash();
        let transact_tx =
            ctx.alloy_provider.get_transaction_by_hash(tx_hash).await.unwrap().unwrap();

        assert_eq!(transact_tx.from(), user);
        assert_eq!(transact_tx.to().unwrap(), contract_addr);
        assert_eq!(transact_tx.value(), U256::ZERO);
        assert_eq!(transact_tx.input(), &Bytes::from(Counter::incrementCall::SELECTOR));
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_transact_underfunded_gas() {
    run_test(|ctx| async move {
        // set up user
        let user = ctx.addresses[0];
        let mut user_nonce = ctx.track_nonce(user, Some("user"));
        let mut user_bal = ctx.track_balance(user, Some("user"));

        // Deploy a contract to interact with
        let deployer = ctx.addresses[1];
        let contract = ctx.deploy_counter(deployer).await;
        let contract_addr = *contract.address();
        assert_eq!(ctx.alloy_provider.get_code_at(contract_addr).await.unwrap(), COUNTER_BYTECODE,);

        // assert that the counter is zero
        let contract = TestCounterInstance::new(contract_addr, ctx.alloy_provider.clone());
        assert_eq!(contract.count().call().await.unwrap(), U256::ZERO);

        // make a transact with a tiny gas limit so it will revert w/ OOG
        let tiny_gas = 10u64;
        let transact = Transactor::Transact {
            rollupChainId: U256::from(ctx.constants().ru_chain_id()),
            sender: user,
            to: contract_addr,
            data: Bytes::from(Counter::incrementCall::SELECTOR),
            value: U256::ZERO,
            gas: U256::from(tiny_gas),
            maxFeePerGas: U256::from(GWEI_TO_WEI),
        };

        let block = HostBlockSpec::new(ctx.constants()).transact(transact);

        // process the block to commit the results
        ctx.process_block(block).await.unwrap();

        // nonce and balance should NOT change because the transact was discarded for OOG
        let old_nonce = user_nonce.update_nonce();
        assert_eq!(user_nonce.previous_nonce(), old_nonce, "expected nonce not to change");
        user_bal.assert_no_change();

        // contract state should not change because the call ran out of gas
        assert_eq!(contract.count().call().await.unwrap(), U256::ZERO);

        // check signet events for the recorded transact and that the gas equals tiny_gas
        let last = ctx.factory.provider().unwrap().last_block_number().unwrap();
        let events = ctx.factory.provider().unwrap().get::<SignetEvents>(last..last + 1).unwrap();

        // Check that the block has no transactions, i.e. that the transact was
        // discarded
        let last_block = ctx
            .alloy_provider
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await
            .unwrap()
            .unwrap();
        assert!(last_block.transactions.is_empty());

        let found = events.iter().find(|(_, ev)| match ev {
            DbSignetEvent::Transact(_, Transactor::Transact { sender, to, gas, .. }) => {
                *sender == user && *to == contract_addr && *gas == U256::from(tiny_gas)
            }
            _ => false,
        });

        assert!(found.is_some(), "expected DbSignetEvent::Transact with tiny gas");
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_signet_events() {
    run_test(|ctx| async move {
        let user_a = ctx.addresses[0];
        let user_b = ctx.addresses[1];

        #[allow(non_snake_case)]
        let rollupChainId = U256::from(TEST_CONSTANTS.ru_chain_id());

        let mut user_a_bal = ctx.track_balance(user_a, Some("user_a"));
        let mut user_b_bal = ctx.track_balance(user_b, Some("user_b"));

        // First enter_token is 1 wbt to user_a
        let wbtc_address = ctx.constants.host().tokens().wbtc();
        let wbtc_enter_amount = GWEI_TO_WEI as usize;
        let wbtc_enter_amount_u256 = U256::from(wbtc_enter_amount);

        // Second enter_token is 1 USDC to user_a
        // This is expected to mint 1 USD
        let usdc_address = USDC_RECORD.address();
        let usdc_enter_amount = 10usize.pow(USDC_RECORD.decimals() as u32);
        let expected_usd_minted = adjust_usd_decimals(usdc_enter_amount, USDC_RECORD.decimals());

        // Third we enter 1 ETH to user_a
        let eth_enter_amount = ETH_TO_WEI as usize;
        let eth_enter_amount_u256 = U256::from(eth_enter_amount);

        // The last event is a transact from user_a to user_b for 1 USD with the data [0xab, 0xcd]
        let input = Bytes::from(vec![0xab, 0xcd]);

        // Spec and process the block
        let block = HostBlockSpec::new(ctx.constants())
            .enter_token(user_a, wbtc_enter_amount, wbtc_address)
            .enter_token(user_a, usdc_enter_amount, usdc_address)
            .enter(user_a, eth_enter_amount)
            .simple_transact(user_a, user_b, input, expected_usd_minted.to());
        ctx.process_block(block.clone()).await.unwrap();

        // Check the base fee
        let base_fee = ctx
            .factory
            .header_by_number(1)
            .unwrap()
            .unwrap()
            .base_fee_per_gas
            .map(U256::from)
            .unwrap();

        // NB:
        // user_a should have received 1 USD,
        // user_a then paid base_fee * gas to send 1 USD to user_b,
        // so user_a's balance decreases. The 1 in 1 out cancel out. So fee is
        // the only change in user_a's balance.
        //
        // user_b should have received 1 USD
        //
        // the 100_000 here is the gas limit used by `simple_transact()`
        user_a_bal.assert_decrease_exact(base_fee * U256::from(100_000));
        user_b_bal.assert_increase_exact(U256::from(expected_usd_minted));

        // Process the block again.
        ctx.process_block(block).await.unwrap();

        // Check the base fee
        let base_fee = ctx
            .factory
            .header_by_number(2)
            .unwrap()
            .unwrap()
            .base_fee_per_gas
            .map(U256::from)
            .unwrap();

        // This time works exactly the same as above.
        user_a_bal.assert_decrease_exact(base_fee * U256::from(100_000));
        user_b_bal.assert_increase_exact(U256::from(expected_usd_minted));

        let events = ctx.factory.provider().unwrap().get::<SignetEvents>(1..3).unwrap();
        assert_eq!(events.len(), 8);

        // The tuple is (block_number, event).
        // We expect 4 events per block
        assert_eq!(events[0].0, 1);
        assert_eq!(events[1].0, 1);
        assert_eq!(events[2].0, 1);
        assert_eq!(events[3].0, 1);
        assert_eq!(events[4].0, 2);
        assert_eq!(events[5].0, 2);
        assert_eq!(events[6].0, 2);
        assert_eq!(events[7].0, 2);

        assert_eq!(
            events[0].1,
            DbSignetEvent::Transact(
                3,
                Transactor::Transact {
                    rollupChainId,
                    sender: user_a,
                    to: user_b,
                    data: vec![0xab, 0xcd].into(),
                    value: U256::from(expected_usd_minted),
                    gas: U256::from(100_000),
                    maxFeePerGas: U256::from(GWEI_TO_WEI),
                }
            )
        );

        assert_eq!(
            events[1].1,
            DbSignetEvent::Enter(
                0,
                Passage::Enter {
                    rollupChainId,
                    rollupRecipient: user_a,
                    amount: eth_enter_amount_u256,
                }
            )
        );

        assert_eq!(
            events[2].1,
            DbSignetEvent::EnterToken(
                1,
                Passage::EnterToken {
                    rollupChainId,
                    rollupRecipient: user_a,
                    amount: wbtc_enter_amount_u256,
                    token: wbtc_address,
                }
            )
        );

        assert_eq!(
            events[3].1,
            DbSignetEvent::EnterToken(
                2,
                Passage::EnterToken {
                    rollupChainId,
                    rollupRecipient: user_a,
                    amount: U256::from(usdc_enter_amount),
                    token: usdc_address,
                }
            )
        );

        assert_eq!(
            events[4].1,
            DbSignetEvent::Transact(
                3,
                Transactor::Transact {
                    rollupChainId,
                    sender: user_a,
                    to: user_b,
                    data: vec![0xab, 0xcd].into(),
                    value: U256::from(expected_usd_minted),
                    gas: U256::from(100_000),
                    maxFeePerGas: U256::from(GWEI_TO_WEI),
                }
            )
        );

        assert_eq!(
            events[5].1,
            DbSignetEvent::Enter(
                0,
                Passage::Enter {
                    rollupChainId,
                    rollupRecipient: user_a,
                    amount: eth_enter_amount_u256,
                }
            )
        );

        assert_eq!(
            events[6].1,
            DbSignetEvent::EnterToken(
                1,
                Passage::EnterToken {
                    rollupChainId,
                    rollupRecipient: user_a,
                    amount: wbtc_enter_amount_u256,
                    token: wbtc_address,
                }
            )
        );

        assert_eq!(
            events[7].1,
            DbSignetEvent::EnterToken(
                2,
                Passage::EnterToken {
                    rollupChainId,
                    rollupRecipient: user_a,
                    amount: U256::from(usdc_enter_amount),
                    token: usdc_address,
                }
            )
        );
    })
    .await;
}
