use alloy::{
    consensus::constants::GWEI_TO_WEI,
    network::TransactionBuilder,
    primitives::{Address, U256},
    providers::Provider,
    rpc::types::eth::TransactionRequest,
};
use serial_test::serial;
use signet_node_tests::{HostBlockSpec, RuBlockSpec, types::TestCounterInstance, utils::run_test};
use signet_test_utils::contracts::counter::{COUNTER_BYTECODE, COUNTER_DEPLOY_CODE};

const USER_A: Address = Address::repeat_byte(0x39);

#[serial]
#[tokio::test]
async fn test_submit_tx() {
    let amnt = U256::from(100000);

    run_test(|ctx| async move {
        let mut user_a_bal = ctx.track_balance(USER_A, Some("user"));

        let tx = TransactionRequest::default()
            .from(ctx.addresses[0])
            .max_priority_fee_per_gas(GWEI_TO_WEI.into())
            .max_fee_per_gas(GWEI_TO_WEI.into())
            .gas_limit(21_000)
            .to(USER_A)
            .value(amnt);

        // Sign the transaction
        let tx = ctx.fill_alloy_tx(&tx).await.unwrap();

        let ru_block = RuBlockSpec::new(ctx.constants()).alloy_tx(&tx);

        let host_block = HostBlockSpec::new(ctx.constants()).submit_block(ru_block);

        ctx.process_block(host_block).await.unwrap();
        user_a_bal.assert_increase_exact(amnt);
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_deploy_contract() {
    run_test(|ctx| async move {
        let deployer = ctx.addresses[0];
        let mut deployer_bal = ctx.track_balance(deployer, Some("deployer"));

        let tx = TransactionRequest::default()
            .to(Address::ZERO)
            .from(deployer)
            .max_priority_fee_per_gas(GWEI_TO_WEI.into())
            .max_fee_per_gas(GWEI_TO_WEI.into())
            .gas_limit(21_000_000)
            .with_deploy_code(COUNTER_DEPLOY_CODE);

        let (tx, receipt) = ctx.process_alloy_tx(&tx).await.unwrap();
        deployer_bal.assert_decrease_exact(U256::from(
            receipt.gas_used as u128 * receipt.effective_gas_price,
        ));

        // calculate contract address and assert it exists
        let nonce = tx.as_eip1559().unwrap().tx().nonce;
        let contract_addr = deployer.create(nonce);

        assert_eq!(ctx.alloy_provider.get_code_at(contract_addr).await.unwrap(), COUNTER_BYTECODE,);

        let contract = TestCounterInstance::new(contract_addr, ctx.alloy_provider.clone());

        assert_eq!(contract.count().call().await.unwrap(), U256::ZERO);

        // Now we increment it and assert the count changed
        let tx = contract.increment().from(deployer).into_transaction_request();
        let _ = ctx.process_alloy_tx(&tx).await.unwrap();

        assert_eq!(contract.count().call().await.unwrap(), U256::from(1));
    })
    .await;
}
