use alloy::{
    consensus::constants::ETH_TO_WEI,
    primitives::{Address, U256},
};
use reth::providers::AccountExtReader;
use reth_db::{
    AccountChangeSets, AccountsHistory, cursor::DbCursorRO, models::ShardedKey, transaction::DbTx,
};
use serial_test::serial;
use signet_constants::test_utils::HOST_USDC;
use signet_node_tests::{HostBlockSpec, SignetTestContext, run_test, utils::adjust_usd_decimals};
use signet_test_utils::chain::{USDC_RECORD, USDT_RECORD};

const USER_A: Address = Address::repeat_byte(0x39);
const USER_B: Address = Address::repeat_byte(0x40);
const USER_C: Address = Address::repeat_byte(0x41);
const USER_D: Address = Address::repeat_byte(0x42);
const USER_E: Address = Address::repeat_byte(0x43);
const USER_F: Address = Address::repeat_byte(0x44);

const ONE_HOST_USDC: usize = 1_000_000;
const ONE_RU_USDC: u128 = ETH_TO_WEI;

#[serial]
#[tokio::test]
async fn test_three_enters() {
    run_test(|ctx| async move {
        let usdc_record = USDC_RECORD;
        let usdt_record = USDT_RECORD;

        let usdc = usdc_record.address();
        let usdt = usdt_record.address();

        let usdc_decimals = usdc_record.decimals();
        let usdt_decimals = usdt_record.decimals();

        let mut a_total = U256::ZERO;
        let mut b_total = U256::ZERO;
        let mut c_total = U256::ZERO;

        let block_one = HostBlockSpec::new(ctx.constants())
            .with_block_number(1)
            .enter_token(USER_A, 100, usdc)
            .enter_token(USER_B, 200, usdc)
            .enter_token(USER_C, 300, usdc);

        a_total += adjust_usd_decimals(100, usdc_decimals);
        b_total += adjust_usd_decimals(200, usdc_decimals);
        c_total += adjust_usd_decimals(300, usdc_decimals);

        let block_two = HostBlockSpec::new(ctx.constants())
            .with_block_number(2)
            .enter_token(USER_A, 1000, usdc)
            .enter_token(USER_B, 2000, usdc)
            .enter_token(USER_C, 3000, usdc);

        a_total += adjust_usd_decimals(1000, usdc_decimals);
        b_total += adjust_usd_decimals(2000, usdc_decimals);
        c_total += adjust_usd_decimals(3000, usdc_decimals);

        let block_three = HostBlockSpec::new(ctx.constants())
            .with_block_number(3)
            .enter_token(USER_A, 10000, usdt)
            .enter_token(USER_B, 20000, usdt)
            .enter_token(USER_C, 30000, usdt);

        a_total += adjust_usd_decimals(10000, usdt_decimals);
        b_total += adjust_usd_decimals(20000, usdt_decimals);
        c_total += adjust_usd_decimals(30000, usdt_decimals);

        ctx.process_blocks(vec![block_one, block_two, block_three]).await.unwrap();

        assert_eq!(ctx.balance_of(USER_A), a_total);
        assert_eq!(ctx.balance_of(USER_B), b_total);
        assert_eq!(ctx.balance_of(USER_C), c_total);
    })
    .await;
}

// Processes 5 blocks, setting up accounts with initial balances and histories.
//
// After this, A will have 500, B will have 1000, C will have 1500. They will
// also have accounthistories entries for blocks 1 to 5.
async fn setup_accounts_history(ctx: SignetTestContext) -> SignetTestContext {
    let block = HostBlockSpec::new(ctx.constants())
        .enter_token(USER_A, ONE_HOST_USDC, HOST_USDC)
        .enter_token(USER_B, 2 * ONE_HOST_USDC, HOST_USDC)
        .enter_token(USER_C, 3 * ONE_HOST_USDC, HOST_USDC);

    ctx.process_blocks(vec![block.clone(); 5]).await.unwrap();

    ctx
}

#[serial]
#[tokio::test]
async fn test_write_account_histories() {
    run_test(|ctx| async move {
        let ctx = setup_accounts_history(ctx).await;

        let another_block = HostBlockSpec::new(ctx.constants())
            .enter_token(USER_D, 10 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_E, 20 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_F, 30 * ONE_HOST_USDC, HOST_USDC);
        ctx.process_blocks(vec![another_block]).await.unwrap();

        let provider = ctx.factory.provider().unwrap();

        let a_key = ShardedKey::new(USER_A, u64::MAX);
        let b_key = ShardedKey::new(USER_B, u64::MAX);
        let c_key = ShardedKey::new(USER_C, u64::MAX);
        let d_key = ShardedKey::new(USER_D, u64::MAX);
        let e_key = ShardedKey::new(USER_E, u64::MAX);
        let f_key = ShardedKey::new(USER_F, u64::MAX);

        let v = provider
            .tx_ref()
            .cursor_read::<AccountsHistory>()
            .unwrap()
            .walk_range(a_key.clone()..=f_key.clone())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(v.len(), 6);
        assert_eq!(v[0].0, a_key);
        assert_eq!(v[1].0, b_key);
        assert_eq!(v[2].0, c_key);
        for i in 1..5 {
            assert!(v[0].1.contains(i));
            assert!(v[1].1.contains(i));
            assert!(v[2].1.contains(i));
        }
        assert!(!v[0].1.contains(6));
        assert!(!v[1].1.contains(6));
        assert!(!v[2].1.contains(6));

        assert_eq!(v[3].0, d_key);
        assert_eq!(v[4].0, e_key);
        assert_eq!(v[5].0, f_key);
        for i in 1..5 {
            assert!(!v[3].1.contains(i));
            assert!(!v[4].1.contains(i));
            assert!(!v[5].1.contains(i));
        }
        assert!(v[3].1.contains(6));
        assert!(v[4].1.contains(6));
        assert!(v[5].1.contains(6));
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_write_account_histories_with_empty_block() {
    run_test(|ctx| async move {
        let ctx = setup_accounts_history(ctx).await;

        let another_block = HostBlockSpec::new(ctx.constants())
            .enter_token(USER_D, 10 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_E, 20 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_F, 30 * ONE_HOST_USDC, HOST_USDC);
        ctx.process_blocks(vec![another_block]).await.unwrap();

        let provider = ctx.factory.provider().unwrap();

        let a_key = ShardedKey::new(USER_A, u64::MAX);
        let b_key = ShardedKey::new(USER_B, u64::MAX);
        let c_key = ShardedKey::new(USER_C, u64::MAX);
        let d_key = ShardedKey::new(USER_D, u64::MAX);
        let e_key = ShardedKey::new(USER_E, u64::MAX);
        let f_key = ShardedKey::new(USER_F, u64::MAX);

        let v = provider
            .tx_ref()
            .cursor_read::<AccountsHistory>()
            .unwrap()
            .walk_range(a_key.clone()..=f_key.clone())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(v.len(), 6);
        assert_eq!(v[0].0, a_key);
        assert_eq!(v[1].0, b_key);
        assert_eq!(v[2].0, c_key);
        for i in 1..5 {
            assert!(v[0].1.contains(i));
            assert!(v[1].1.contains(i));
            assert!(v[2].1.contains(i));
        }
        assert!(!v[0].1.contains(6));
        assert!(!v[1].1.contains(6));
        assert!(!v[2].1.contains(6));

        assert_eq!(v[3].0, d_key);
        assert_eq!(v[4].0, e_key);
        assert_eq!(v[5].0, f_key);
        for i in 1..5 {
            assert!(!v[3].1.contains(i));
            assert!(!v[4].1.contains(i));
            assert!(!v[5].1.contains(i));
        }
        assert!(v[3].1.contains(6));
        assert!(v[4].1.contains(6));
        assert!(v[5].1.contains(6));

        let empty_block = HostBlockSpec::new(ctx.constants());
        ctx.process_block(empty_block).await.unwrap();

        // As we did not process a new RU block, the history should not change.
        let v = provider
            .tx_ref()
            .cursor_read::<AccountsHistory>()
            .unwrap()
            .walk_range(a_key.clone()..=f_key.clone())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(v.len(), 6);
        assert_eq!(v[0].0, a_key);
        assert_eq!(v[1].0, b_key);
        assert_eq!(v[2].0, c_key);
        for i in 1..5 {
            assert!(v[0].1.contains(i));
            assert!(v[1].1.contains(i));
            assert!(v[2].1.contains(i));
        }
        assert!(!v[0].1.contains(6));
        assert!(!v[1].1.contains(6));
        assert!(!v[2].1.contains(6));

        assert_eq!(v[3].0, d_key);
        assert_eq!(v[4].0, e_key);
        assert_eq!(v[5].0, f_key);
        for i in 1..5 {
            assert!(!v[3].1.contains(i));
            assert!(!v[4].1.contains(i));
            assert!(!v[5].1.contains(i));
        }
        assert!(v[3].1.contains(6));
        assert!(v[4].1.contains(6));
        assert!(v[5].1.contains(6));
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_write_account_histories_with_reorg_and_empty_blocks() {
    run_test(|ctx| async move {
        let ctx = setup_accounts_history(ctx).await;

        let provider = ctx.factory.provider().unwrap();

        let a_key = ShardedKey::new(USER_A, u64::MAX);
        let b_key = ShardedKey::new(USER_B, u64::MAX);
        let c_key = ShardedKey::new(USER_C, u64::MAX);

        let another_block = HostBlockSpec::new(ctx.constants())
            .enter_token(USER_A, 10 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_B, 2 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_C, 3 * ONE_HOST_USDC, HOST_USDC);
        ctx.process_blocks(vec![another_block.clone()]).await.unwrap();

        let v = provider
            .tx_ref()
            .cursor_read::<AccountsHistory>()
            .unwrap()
            .walk_range(a_key.clone()..=c_key.clone())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(v.len(), 3);
        assert_eq!(v[0].0, a_key);
        assert_eq!(v[1].0, b_key);
        assert_eq!(v[2].0, c_key);

        for i in 1..6 {
            assert!(v[0].1.contains(i));
            assert!(v[1].1.contains(i));
            assert!(v[2].1.contains(i));
        }

        // After reorg, the history should not contain the latest entries
        ctx.revert_block(another_block).await.unwrap();

        let v = provider
            .tx_ref()
            .cursor_read::<AccountsHistory>()
            .unwrap()
            .walk_range(a_key.clone()..=c_key.clone())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(v.len(), 3);
        assert_eq!(v[0].0, a_key);
        assert_eq!(v[1].0, b_key);
        assert_eq!(v[2].0, c_key);

        for i in 1..5 {
            assert!(v[0].1.contains(i));
            assert!(v[1].1.contains(i));
            assert!(v[2].1.contains(i));
        }
        assert!(!v[0].1.contains(6));
        assert!(!v[1].1.contains(6));
        assert!(!v[2].1.contains(6));

        // Now process an empty block.
        let empty_block = HostBlockSpec::new(ctx.constants());
        ctx.process_block(empty_block).await.unwrap();

        // As we did not process a new RU block, the history should not change.
        let v = provider
            .tx_ref()
            .cursor_read::<AccountsHistory>()
            .unwrap()
            .walk_range(a_key.clone()..=c_key.clone())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(v.len(), 3);
        assert_eq!(v[0].0, a_key);
        assert_eq!(v[1].0, b_key);
        assert_eq!(v[2].0, c_key);

        for i in 1..5 {
            assert!(v[0].1.contains(i));
            assert!(v[1].1.contains(i));
            assert!(v[2].1.contains(i));
        }
        assert!(!v[0].1.contains(6));
        assert!(!v[1].1.contains(6));
        assert!(!v[2].1.contains(6));

        // re-process the reorged block.
        let another_block = HostBlockSpec::new(ctx.constants())
            .enter_token(USER_A, 10 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_B, 2 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_C, 3 * ONE_HOST_USDC, HOST_USDC);
        ctx.process_blocks(vec![another_block.clone()]).await.unwrap();

        let v = provider
            .tx_ref()
            .cursor_read::<AccountsHistory>()
            .unwrap()
            .walk_range(a_key.clone()..=c_key.clone())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(v.len(), 3);
        assert_eq!(v[0].0, a_key);
        assert_eq!(v[1].0, b_key);
        assert_eq!(v[2].0, c_key);

        for i in 1..6 {
            assert!(v[0].1.contains(i));
            assert!(v[1].1.contains(i));
            assert!(v[2].1.contains(i));
        }
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_write_account_histories_with_reorg() {
    run_test(|ctx| async move {
        let ctx = setup_accounts_history(ctx).await;

        let provider = ctx.factory.provider().unwrap();

        let a_key = ShardedKey::new(USER_A, u64::MAX);
        let b_key = ShardedKey::new(USER_B, u64::MAX);
        let c_key = ShardedKey::new(USER_C, u64::MAX);

        let another_block = HostBlockSpec::new(ctx.constants())
            .enter_token(USER_A, 10 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_B, 2 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_C, 3 * ONE_HOST_USDC, HOST_USDC);
        ctx.process_blocks(vec![another_block.clone()]).await.unwrap();

        let v = provider
            .tx_ref()
            .cursor_read::<AccountsHistory>()
            .unwrap()
            .walk_range(a_key.clone()..=c_key.clone())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(v.len(), 3);
        assert_eq!(v[0].0, a_key);
        assert_eq!(v[1].0, b_key);
        assert_eq!(v[2].0, c_key);

        for i in 1..6 {
            assert!(v[0].1.contains(i));
            assert!(v[1].1.contains(i));
            assert!(v[2].1.contains(i));
        }

        // After reorg, the history should not contain the latest entries
        ctx.revert_block(another_block).await.unwrap();

        let v = provider
            .tx_ref()
            .cursor_read::<AccountsHistory>()
            .unwrap()
            .walk_range(a_key.clone()..=c_key.clone())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(v.len(), 3);
        assert_eq!(v[0].0, a_key);
        assert_eq!(v[1].0, b_key);
        assert_eq!(v[2].0, c_key);

        for i in 1..5 {
            assert!(v[0].1.contains(i));
            assert!(v[1].1.contains(i));
            assert!(v[2].1.contains(i));
        }
        assert!(!v[0].1.contains(6));
        assert!(!v[1].1.contains(6));
        assert!(!v[2].1.contains(6));
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_historical_state_provider(ctx: SignetTestContext) {
    run_test(|ctx| async move {
        let ctx = setup_accounts_history(ctx).await;

        let another_block = HostBlockSpec::new(ctx.constants())
            .enter_token(USER_A, 10 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_B, 20 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_C, 30 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_D, 40 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_E, 50 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_F, 60 * ONE_HOST_USDC, HOST_USDC);
        ctx.process_blocks(vec![another_block]).await.unwrap();

        let provider = ctx.factory.provider().unwrap();

        // NB: It is bizarre that reth has completely different APIs for
        // historical state and current state. basic_accounts is only on
        // current while account_balance is only on historical.
        let accounts =
            provider.basic_accounts([USER_A, USER_B, USER_C, USER_D, USER_E, USER_F]).unwrap();

        assert_eq!(accounts[0].1.as_ref().unwrap().balance, U256::from(15 * ONE_RU_USDC));
        assert_eq!(accounts[1].1.as_ref().unwrap().balance, U256::from(30 * ONE_RU_USDC));
        assert_eq!(accounts[2].1.as_ref().unwrap().balance, U256::from(45 * ONE_RU_USDC));
        assert_eq!(accounts[3].1.as_ref().unwrap().balance, U256::from(40 * ONE_RU_USDC));
        assert_eq!(accounts[4].1.as_ref().unwrap().balance, U256::from(50 * ONE_RU_USDC));
        assert_eq!(accounts[5].1.as_ref().unwrap().balance, U256::from(60 * ONE_RU_USDC));

        let historical = ctx.factory.history_by_block_number(5).unwrap();
        assert_eq!(
            historical.account_balance(&USER_A).unwrap().unwrap(),
            U256::from(5 * ONE_RU_USDC)
        );
        assert_eq!(
            historical.account_balance(&USER_B).unwrap().unwrap(),
            U256::from(10 * ONE_RU_USDC)
        );
        assert_eq!(
            historical.account_balance(&USER_C).unwrap().unwrap(),
            U256::from(15 * ONE_RU_USDC)
        );
        assert!(historical.account_balance(&USER_D).unwrap().is_none());
        assert!(historical.account_balance(&USER_E).unwrap().is_none());
        assert!(historical.account_balance(&USER_F).unwrap().is_none());
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_historical_state_provider_with_empty_blocks(ctx: SignetTestContext) {
    run_test(|ctx| async move {
        let ctx = setup_accounts_history(ctx).await;

        let another_block = HostBlockSpec::new(ctx.constants())
            .enter_token(USER_A, 10 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_B, 20 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_C, 30 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_D, 40 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_E, 50 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_F, 60 * ONE_HOST_USDC, HOST_USDC);
        ctx.process_blocks(vec![another_block]).await.unwrap();

        let provider = ctx.factory.provider().unwrap();

        // NB: It is bizarre that reth has completely different APIs for
        // historical state and current state. basic_accounts is only on
        // current while account_balance is only on historical.
        let accounts =
            provider.basic_accounts([USER_A, USER_B, USER_C, USER_D, USER_E, USER_F]).unwrap();

        assert_eq!(accounts[0].1.as_ref().unwrap().balance, U256::from(15 * ONE_RU_USDC));
        assert_eq!(accounts[1].1.as_ref().unwrap().balance, U256::from(30 * ONE_RU_USDC));
        assert_eq!(accounts[2].1.as_ref().unwrap().balance, U256::from(45 * ONE_RU_USDC));
        assert_eq!(accounts[3].1.as_ref().unwrap().balance, U256::from(40 * ONE_RU_USDC));
        assert_eq!(accounts[4].1.as_ref().unwrap().balance, U256::from(50 * ONE_RU_USDC));
        assert_eq!(accounts[5].1.as_ref().unwrap().balance, U256::from(60 * ONE_RU_USDC));

        let historical = ctx.factory.history_by_block_number(5).unwrap();
        assert_eq!(
            historical.account_balance(&USER_A).unwrap().unwrap(),
            U256::from(5 * ONE_RU_USDC)
        );
        assert_eq!(
            historical.account_balance(&USER_B).unwrap().unwrap(),
            U256::from(10 * ONE_RU_USDC)
        );
        assert_eq!(
            historical.account_balance(&USER_C).unwrap().unwrap(),
            U256::from(15 * ONE_RU_USDC)
        );
        assert!(historical.account_balance(&USER_D).unwrap().is_none());
        assert!(historical.account_balance(&USER_E).unwrap().is_none());
        assert!(historical.account_balance(&USER_F).unwrap().is_none());

        let empty_block = HostBlockSpec::new(ctx.constants());
        ctx.process_blocks(vec![empty_block; 2]).await.unwrap();

        // the historical state that we previously checked should not change, even after processing empty blocks.
        let accounts =
            provider.basic_accounts([USER_A, USER_B, USER_C, USER_D, USER_E, USER_F]).unwrap();

        assert_eq!(accounts[0].1.as_ref().unwrap().balance, U256::from(15 * ONE_RU_USDC));
        assert_eq!(accounts[1].1.as_ref().unwrap().balance, U256::from(30 * ONE_RU_USDC));
        assert_eq!(accounts[2].1.as_ref().unwrap().balance, U256::from(45 * ONE_RU_USDC));
        assert_eq!(accounts[3].1.as_ref().unwrap().balance, U256::from(40 * ONE_RU_USDC));
        assert_eq!(accounts[4].1.as_ref().unwrap().balance, U256::from(50 * ONE_RU_USDC));
        assert_eq!(accounts[5].1.as_ref().unwrap().balance, U256::from(60 * ONE_RU_USDC));

        let historical = ctx.factory.history_by_block_number(5).unwrap();
        assert_eq!(
            historical.account_balance(&USER_A).unwrap().unwrap(),
            U256::from(5 * ONE_RU_USDC)
        );
        assert_eq!(
            historical.account_balance(&USER_B).unwrap().unwrap(),
            U256::from(10 * ONE_RU_USDC)
        );
        assert_eq!(
            historical.account_balance(&USER_C).unwrap().unwrap(),
            U256::from(15 * ONE_RU_USDC)
        );
        assert!(historical.account_balance(&USER_D).unwrap().is_none());
        assert!(historical.account_balance(&USER_E).unwrap().is_none());
        assert!(historical.account_balance(&USER_F).unwrap().is_none());
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_historical_state_provider_with_reorg() {
    run_test(|ctx| async move {
        let ctx = setup_accounts_history(ctx).await;

        let another_block = HostBlockSpec::new(ctx.constants())
            .enter_token(USER_A, 10 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_B, 20 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_C, 30 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_D, 40 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_E, 50 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_F, 60 * ONE_HOST_USDC, HOST_USDC);
        ctx.process_blocks(vec![another_block.clone()]).await.unwrap();

        let provider = ctx.factory.provider().unwrap();

        // NB: It is bizarre that reth has completely different APIs for
        // historical state and current state. basic_accounts is only on
        // current while account_balance is only on historical.
        let accounts =
            provider.basic_accounts([USER_A, USER_B, USER_C, USER_D, USER_E, USER_F]).unwrap();

        assert_eq!(accounts[0].1.as_ref().unwrap().balance, U256::from(15 * ONE_RU_USDC));
        assert_eq!(accounts[1].1.as_ref().unwrap().balance, U256::from(30 * ONE_RU_USDC));
        assert_eq!(accounts[2].1.as_ref().unwrap().balance, U256::from(45 * ONE_RU_USDC));
        assert_eq!(accounts[3].1.as_ref().unwrap().balance, U256::from(40 * ONE_RU_USDC));
        assert_eq!(accounts[4].1.as_ref().unwrap().balance, U256::from(50 * ONE_RU_USDC));
        assert_eq!(accounts[5].1.as_ref().unwrap().balance, U256::from(60 * ONE_RU_USDC));

        let historical = ctx.factory.history_by_block_number(5).unwrap();
        assert_eq!(
            historical.account_balance(&USER_A).unwrap().unwrap(),
            U256::from(5 * ONE_RU_USDC)
        );
        assert_eq!(
            historical.account_balance(&USER_B).unwrap().unwrap(),
            U256::from(10 * ONE_RU_USDC)
        );
        assert_eq!(
            historical.account_balance(&USER_C).unwrap().unwrap(),
            U256::from(15 * ONE_RU_USDC)
        );
        assert!(historical.account_balance(&USER_D).unwrap().is_none());
        assert!(historical.account_balance(&USER_E).unwrap().is_none());
        assert!(historical.account_balance(&USER_F).unwrap().is_none());

        ctx.revert_block(another_block).await.unwrap();
        // Make the same assertions after reverting, the historical state should not change
        assert_eq!(
            historical.account_balance(&USER_A).unwrap().unwrap(),
            U256::from(5 * ONE_RU_USDC)
        );
        assert_eq!(
            historical.account_balance(&USER_B).unwrap().unwrap(),
            U256::from(10 * ONE_RU_USDC)
        );
        assert_eq!(
            historical.account_balance(&USER_C).unwrap().unwrap(),
            U256::from(15 * ONE_RU_USDC)
        );
        assert!(historical.account_balance(&USER_D).unwrap().is_none());
        assert!(historical.account_balance(&USER_E).unwrap().is_none());
        assert!(historical.account_balance(&USER_F).unwrap().is_none());

        let new_block_6 = HostBlockSpec::new(ctx.constants())
            .enter_token(USER_A, 10 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_B, 20 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_C, 30 * ONE_HOST_USDC, HOST_USDC);

        ctx.process_block(new_block_6).await.unwrap();

        // new current state assertions
        let provider = ctx.factory.provider().unwrap();
        let accounts =
            provider.basic_accounts([USER_A, USER_B, USER_C, USER_D, USER_E, USER_F]).unwrap();
        assert_eq!(accounts[0].1.as_ref().unwrap().balance, U256::from(15 * ONE_RU_USDC));
        assert_eq!(accounts[1].1.as_ref().unwrap().balance, U256::from(30 * ONE_RU_USDC));
        assert_eq!(accounts[2].1.as_ref().unwrap().balance, U256::from(45 * ONE_RU_USDC));
        assert!(accounts[3].1.is_none());
        assert!(accounts[4].1.is_none());
        assert!(accounts[5].1.is_none());

        // Make the same assertions after the new block 6, the historical state should not change
        assert_eq!(
            historical.account_balance(&USER_A).unwrap().unwrap(),
            U256::from(5 * ONE_RU_USDC)
        );
        assert_eq!(
            historical.account_balance(&USER_B).unwrap().unwrap(),
            U256::from(10 * ONE_RU_USDC)
        );
        assert_eq!(
            historical.account_balance(&USER_C).unwrap().unwrap(),
            U256::from(15 * ONE_RU_USDC)
        );
        assert!(historical.account_balance(&USER_D).unwrap().is_none());
        assert!(historical.account_balance(&USER_E).unwrap().is_none());
        assert!(historical.account_balance(&USER_F).unwrap().is_none());
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_write_changesets() {
    run_test(|ctx| async move {
        let ctx = setup_accounts_history(ctx).await;

        let provider = ctx.factory.provider().unwrap();

        let mut cursor = provider.tx_ref().cursor_dup_read::<AccountChangeSets>().unwrap();

        let entries_4 =
            cursor.walk_range(4..5).unwrap().map(Result::unwrap).map(|t| t.1).collect::<Vec<_>>();

        let entry_a = entries_4.iter().find(|e| e.address == USER_A).unwrap();
        let entry_b = entries_4.iter().find(|e| e.address == USER_B).unwrap();
        let entry_c = entries_4.iter().find(|e| e.address == USER_C).unwrap();

        assert_eq!(entry_a.info.as_ref().unwrap().balance, U256::from(3 * ONE_RU_USDC));
        assert_eq!(entry_b.info.as_ref().unwrap().balance, U256::from(6 * ONE_RU_USDC));
        assert_eq!(entry_c.info.as_ref().unwrap().balance, U256::from(9 * ONE_RU_USDC));

        let entries_5 =
            cursor.walk(Some(5)).unwrap().map(Result::unwrap).map(|t| t.1).collect::<Vec<_>>();

        let entry_a = entries_5.iter().find(|e| e.address == USER_A).unwrap();
        let entry_b = entries_5.iter().find(|e| e.address == USER_B).unwrap();
        let entry_c = entries_5.iter().find(|e| e.address == USER_C).unwrap();

        assert_eq!(entry_a.info.as_ref().unwrap().balance, U256::from(4 * ONE_RU_USDC));
        assert_eq!(entry_b.info.as_ref().unwrap().balance, U256::from(8 * ONE_RU_USDC));
        assert_eq!(entry_c.info.as_ref().unwrap().balance, U256::from(12 * ONE_RU_USDC));
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_write_changesets_with_empty_blocks() {
    run_test(|ctx| async move {
        let ctx = setup_accounts_history(ctx).await;

        let provider = ctx.factory.provider().unwrap();

        let mut cursor = provider.tx_ref().cursor_dup_read::<AccountChangeSets>().unwrap();

        let entries_4 =
            cursor.walk_range(4..5).unwrap().map(Result::unwrap).map(|t| t.1).collect::<Vec<_>>();

        let entry_a = entries_4.iter().find(|e| e.address == USER_A).unwrap();
        let entry_b = entries_4.iter().find(|e| e.address == USER_B).unwrap();
        let entry_c = entries_4.iter().find(|e| e.address == USER_C).unwrap();

        assert_eq!(entry_a.info.as_ref().unwrap().balance, U256::from(3 * ONE_RU_USDC));
        assert_eq!(entry_b.info.as_ref().unwrap().balance, U256::from(6 * ONE_RU_USDC));
        assert_eq!(entry_c.info.as_ref().unwrap().balance, U256::from(9 * ONE_RU_USDC));

        let entries_5 =
            cursor.walk(Some(5)).unwrap().map(Result::unwrap).map(|t| t.1).collect::<Vec<_>>();

        let entry_a = entries_5.iter().find(|e| e.address == USER_A).unwrap();
        let entry_b = entries_5.iter().find(|e| e.address == USER_B).unwrap();
        let entry_c = entries_5.iter().find(|e| e.address == USER_C).unwrap();

        assert_eq!(entry_a.info.as_ref().unwrap().balance, U256::from(4 * ONE_RU_USDC));
        assert_eq!(entry_b.info.as_ref().unwrap().balance, U256::from(8 * ONE_RU_USDC));
        assert_eq!(entry_c.info.as_ref().unwrap().balance, U256::from(12 * ONE_RU_USDC));

        let empty_block = HostBlockSpec::new(ctx.constants());
        ctx.process_blocks(vec![empty_block; 2]).await.unwrap();

        // Even after processing empty blocks, the changesets should not change.
        let mut cursor = provider.tx_ref().cursor_dup_read::<AccountChangeSets>().unwrap();

        let entries_4 =
            cursor.walk_range(4..5).unwrap().map(Result::unwrap).map(|t| t.1).collect::<Vec<_>>();

        let entry_a = entries_4.iter().find(|e| e.address == USER_A).unwrap();
        let entry_b = entries_4.iter().find(|e| e.address == USER_B).unwrap();
        let entry_c = entries_4.iter().find(|e| e.address == USER_C).unwrap();

        assert_eq!(entry_a.info.as_ref().unwrap().balance, U256::from(3 * ONE_RU_USDC));
        assert_eq!(entry_b.info.as_ref().unwrap().balance, U256::from(6 * ONE_RU_USDC));
        assert_eq!(entry_c.info.as_ref().unwrap().balance, U256::from(9 * ONE_RU_USDC));

        let entries_5 =
            cursor.walk(Some(5)).unwrap().map(Result::unwrap).map(|t| t.1).collect::<Vec<_>>();

        let entry_a = entries_5.iter().find(|e| e.address == USER_A).unwrap();
        let entry_b = entries_5.iter().find(|e| e.address == USER_B).unwrap();
        let entry_c = entries_5.iter().find(|e| e.address == USER_C).unwrap();

        assert_eq!(entry_a.info.as_ref().unwrap().balance, U256::from(4 * ONE_RU_USDC));
        assert_eq!(entry_b.info.as_ref().unwrap().balance, U256::from(8 * ONE_RU_USDC));
        assert_eq!(entry_c.info.as_ref().unwrap().balance, U256::from(12 * ONE_RU_USDC));
    })
    .await;
}
