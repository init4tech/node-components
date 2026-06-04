use alloy::{
    consensus::constants::ETH_TO_WEI,
    primitives::{Address, U256},
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
// also have account histories entries for blocks 1 to 5.
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

        let a_hist = ctx.account_history(USER_A).unwrap();
        let b_hist = ctx.account_history(USER_B).unwrap();
        let c_hist = ctx.account_history(USER_C).unwrap();
        let d_hist = ctx.account_history(USER_D).unwrap();
        let e_hist = ctx.account_history(USER_E).unwrap();
        let f_hist = ctx.account_history(USER_F).unwrap();

        for i in 1..=5 {
            assert!(a_hist.contains(i));
            assert!(b_hist.contains(i));
            assert!(c_hist.contains(i));
        }
        assert!(!a_hist.contains(6));
        assert!(!b_hist.contains(6));
        assert!(!c_hist.contains(6));

        for i in 1..=5 {
            assert!(!d_hist.contains(i));
            assert!(!e_hist.contains(i));
            assert!(!f_hist.contains(i));
        }
        assert!(d_hist.contains(6));
        assert!(e_hist.contains(6));
        assert!(f_hist.contains(6));
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

        let a_hist = ctx.account_history(USER_A).unwrap();
        let b_hist = ctx.account_history(USER_B).unwrap();
        let c_hist = ctx.account_history(USER_C).unwrap();
        let d_hist = ctx.account_history(USER_D).unwrap();
        let e_hist = ctx.account_history(USER_E).unwrap();
        let f_hist = ctx.account_history(USER_F).unwrap();

        for i in 1..=5 {
            assert!(a_hist.contains(i));
            assert!(b_hist.contains(i));
            assert!(c_hist.contains(i));
        }
        assert!(!a_hist.contains(6));
        assert!(!b_hist.contains(6));
        assert!(!c_hist.contains(6));

        for i in 1..=5 {
            assert!(!d_hist.contains(i));
            assert!(!e_hist.contains(i));
            assert!(!f_hist.contains(i));
        }
        assert!(d_hist.contains(6));
        assert!(e_hist.contains(6));
        assert!(f_hist.contains(6));

        let empty_block = HostBlockSpec::new(ctx.constants());
        ctx.process_block(empty_block).await.unwrap();

        // As we did not process a new RU block, the history should not change.
        let a_hist = ctx.account_history(USER_A).unwrap();
        let b_hist = ctx.account_history(USER_B).unwrap();
        let c_hist = ctx.account_history(USER_C).unwrap();
        let d_hist = ctx.account_history(USER_D).unwrap();
        let e_hist = ctx.account_history(USER_E).unwrap();
        let f_hist = ctx.account_history(USER_F).unwrap();

        for i in 1..=5 {
            assert!(a_hist.contains(i));
            assert!(b_hist.contains(i));
            assert!(c_hist.contains(i));
        }
        assert!(!a_hist.contains(6));
        assert!(!b_hist.contains(6));
        assert!(!c_hist.contains(6));

        for i in 1..=5 {
            assert!(!d_hist.contains(i));
            assert!(!e_hist.contains(i));
            assert!(!f_hist.contains(i));
        }
        assert!(d_hist.contains(6));
        assert!(e_hist.contains(6));
        assert!(f_hist.contains(6));
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_write_account_histories_with_reorg_and_empty_blocks() {
    run_test(|ctx| async move {
        let ctx = setup_accounts_history(ctx).await;

        let another_block = HostBlockSpec::new(ctx.constants())
            .enter_token(USER_A, 10 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_B, 2 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_C, 3 * ONE_HOST_USDC, HOST_USDC);
        ctx.process_blocks(vec![another_block.clone()]).await.unwrap();

        let a_hist = ctx.account_history(USER_A).unwrap();
        let b_hist = ctx.account_history(USER_B).unwrap();
        let c_hist = ctx.account_history(USER_C).unwrap();

        for i in 1..=6 {
            assert!(a_hist.contains(i));
            assert!(b_hist.contains(i));
            assert!(c_hist.contains(i));
        }

        // After reorg, the history should not contain the latest entries
        ctx.revert_block(another_block).await.unwrap();

        let a_hist = ctx.account_history(USER_A).unwrap();
        let b_hist = ctx.account_history(USER_B).unwrap();
        let c_hist = ctx.account_history(USER_C).unwrap();

        for i in 1..=5 {
            assert!(a_hist.contains(i));
            assert!(b_hist.contains(i));
            assert!(c_hist.contains(i));
        }
        assert!(!a_hist.contains(6));
        assert!(!b_hist.contains(6));
        assert!(!c_hist.contains(6));

        // Now process an empty block.
        let empty_block = HostBlockSpec::new(ctx.constants());
        ctx.process_block(empty_block).await.unwrap();

        // As we did not process a new RU block, the history should not change.
        let a_hist = ctx.account_history(USER_A).unwrap();
        let b_hist = ctx.account_history(USER_B).unwrap();
        let c_hist = ctx.account_history(USER_C).unwrap();

        for i in 1..=5 {
            assert!(a_hist.contains(i));
            assert!(b_hist.contains(i));
            assert!(c_hist.contains(i));
        }
        assert!(!a_hist.contains(6));
        assert!(!b_hist.contains(6));
        assert!(!c_hist.contains(6));

        // re-process the reorged block. The empty block above consumed
        // RU height 6, so this block lands at RU height 7.
        let another_block = HostBlockSpec::new(ctx.constants())
            .enter_token(USER_A, 10 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_B, 2 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_C, 3 * ONE_HOST_USDC, HOST_USDC);
        ctx.process_blocks(vec![another_block.clone()]).await.unwrap();

        let a_hist = ctx.account_history(USER_A).unwrap();
        let b_hist = ctx.account_history(USER_B).unwrap();
        let c_hist = ctx.account_history(USER_C).unwrap();

        for i in 1..=5 {
            assert!(a_hist.contains(i));
            assert!(b_hist.contains(i));
            assert!(c_hist.contains(i));
        }
        // Block 6 was the empty block (no state changes, no history entry).
        // Block 7 is the re-processed block with enter_tokens.
        assert!(!a_hist.contains(6));
        assert!(!b_hist.contains(6));
        assert!(!c_hist.contains(6));
        assert!(a_hist.contains(7));
        assert!(b_hist.contains(7));
        assert!(c_hist.contains(7));
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_write_account_histories_with_reorg() {
    run_test(|ctx| async move {
        let ctx = setup_accounts_history(ctx).await;

        let another_block = HostBlockSpec::new(ctx.constants())
            .enter_token(USER_A, 10 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_B, 2 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_C, 3 * ONE_HOST_USDC, HOST_USDC);
        ctx.process_blocks(vec![another_block.clone()]).await.unwrap();

        let a_hist = ctx.account_history(USER_A).unwrap();
        let b_hist = ctx.account_history(USER_B).unwrap();
        let c_hist = ctx.account_history(USER_C).unwrap();

        for i in 1..=6 {
            assert!(a_hist.contains(i));
            assert!(b_hist.contains(i));
            assert!(c_hist.contains(i));
        }

        // After reorg, the history should not contain the latest entries
        ctx.revert_block(another_block).await.unwrap();

        let a_hist = ctx.account_history(USER_A).unwrap();
        let b_hist = ctx.account_history(USER_B).unwrap();
        let c_hist = ctx.account_history(USER_C).unwrap();

        for i in 1..=5 {
            assert!(a_hist.contains(i));
            assert!(b_hist.contains(i));
            assert!(c_hist.contains(i));
        }
        assert!(!a_hist.contains(6));
        assert!(!b_hist.contains(6));
        assert!(!c_hist.contains(6));
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_historical_state_provider() {
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

        // Current state
        assert_eq!(ctx.account(USER_A).unwrap().balance, U256::from(15 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_B).unwrap().balance, U256::from(30 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_C).unwrap().balance, U256::from(45 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_D).unwrap().balance, U256::from(40 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_E).unwrap().balance, U256::from(50 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_F).unwrap().balance, U256::from(60 * ONE_RU_USDC));

        // Historical state at block 5
        assert_eq!(ctx.account_at_height(USER_A, 5).unwrap().balance, U256::from(5 * ONE_RU_USDC));
        assert_eq!(ctx.account_at_height(USER_B, 5).unwrap().balance, U256::from(10 * ONE_RU_USDC));
        assert_eq!(ctx.account_at_height(USER_C, 5).unwrap().balance, U256::from(15 * ONE_RU_USDC));
        assert!(ctx.account_at_height(USER_D, 5).is_none());
        assert!(ctx.account_at_height(USER_E, 5).is_none());
        assert!(ctx.account_at_height(USER_F, 5).is_none());
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_historical_state_provider_with_empty_blocks() {
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

        // Current state
        assert_eq!(ctx.account(USER_A).unwrap().balance, U256::from(15 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_B).unwrap().balance, U256::from(30 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_C).unwrap().balance, U256::from(45 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_D).unwrap().balance, U256::from(40 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_E).unwrap().balance, U256::from(50 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_F).unwrap().balance, U256::from(60 * ONE_RU_USDC));

        // Historical state at block 5
        assert_eq!(ctx.account_at_height(USER_A, 5).unwrap().balance, U256::from(5 * ONE_RU_USDC));
        assert_eq!(ctx.account_at_height(USER_B, 5).unwrap().balance, U256::from(10 * ONE_RU_USDC));
        assert_eq!(ctx.account_at_height(USER_C, 5).unwrap().balance, U256::from(15 * ONE_RU_USDC));
        assert!(ctx.account_at_height(USER_D, 5).is_none());
        assert!(ctx.account_at_height(USER_E, 5).is_none());
        assert!(ctx.account_at_height(USER_F, 5).is_none());

        let empty_block = HostBlockSpec::new(ctx.constants());
        ctx.process_blocks(vec![empty_block; 2]).await.unwrap();

        // The historical state previously checked should not change, even after
        // processing empty blocks.
        assert_eq!(ctx.account(USER_A).unwrap().balance, U256::from(15 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_B).unwrap().balance, U256::from(30 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_C).unwrap().balance, U256::from(45 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_D).unwrap().balance, U256::from(40 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_E).unwrap().balance, U256::from(50 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_F).unwrap().balance, U256::from(60 * ONE_RU_USDC));

        assert_eq!(ctx.account_at_height(USER_A, 5).unwrap().balance, U256::from(5 * ONE_RU_USDC));
        assert_eq!(ctx.account_at_height(USER_B, 5).unwrap().balance, U256::from(10 * ONE_RU_USDC));
        assert_eq!(ctx.account_at_height(USER_C, 5).unwrap().balance, U256::from(15 * ONE_RU_USDC));
        assert!(ctx.account_at_height(USER_D, 5).is_none());
        assert!(ctx.account_at_height(USER_E, 5).is_none());
        assert!(ctx.account_at_height(USER_F, 5).is_none());
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

        // Current state
        assert_eq!(ctx.account(USER_A).unwrap().balance, U256::from(15 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_B).unwrap().balance, U256::from(30 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_C).unwrap().balance, U256::from(45 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_D).unwrap().balance, U256::from(40 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_E).unwrap().balance, U256::from(50 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_F).unwrap().balance, U256::from(60 * ONE_RU_USDC));

        // Historical state at block 5
        assert_eq!(ctx.account_at_height(USER_A, 5).unwrap().balance, U256::from(5 * ONE_RU_USDC));
        assert_eq!(ctx.account_at_height(USER_B, 5).unwrap().balance, U256::from(10 * ONE_RU_USDC));
        assert_eq!(ctx.account_at_height(USER_C, 5).unwrap().balance, U256::from(15 * ONE_RU_USDC));
        assert!(ctx.account_at_height(USER_D, 5).is_none());
        assert!(ctx.account_at_height(USER_E, 5).is_none());
        assert!(ctx.account_at_height(USER_F, 5).is_none());

        ctx.revert_block(another_block).await.unwrap();
        // Make the same assertions after reverting, the historical state should not change
        assert_eq!(ctx.account_at_height(USER_A, 5).unwrap().balance, U256::from(5 * ONE_RU_USDC));
        assert_eq!(ctx.account_at_height(USER_B, 5).unwrap().balance, U256::from(10 * ONE_RU_USDC));
        assert_eq!(ctx.account_at_height(USER_C, 5).unwrap().balance, U256::from(15 * ONE_RU_USDC));
        assert!(ctx.account_at_height(USER_D, 5).is_none());
        assert!(ctx.account_at_height(USER_E, 5).is_none());
        assert!(ctx.account_at_height(USER_F, 5).is_none());

        let new_block_6 = HostBlockSpec::new(ctx.constants())
            .enter_token(USER_A, 10 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_B, 20 * ONE_HOST_USDC, HOST_USDC)
            .enter_token(USER_C, 30 * ONE_HOST_USDC, HOST_USDC);

        ctx.process_block(new_block_6).await.unwrap();

        // New current state assertions
        assert_eq!(ctx.account(USER_A).unwrap().balance, U256::from(15 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_B).unwrap().balance, U256::from(30 * ONE_RU_USDC));
        assert_eq!(ctx.account(USER_C).unwrap().balance, U256::from(45 * ONE_RU_USDC));
        assert!(ctx.account(USER_D).is_none());
        assert!(ctx.account(USER_E).is_none());
        assert!(ctx.account(USER_F).is_none());

        // Make the same assertions after the new block 6, the historical state should not change
        assert_eq!(ctx.account_at_height(USER_A, 5).unwrap().balance, U256::from(5 * ONE_RU_USDC));
        assert_eq!(ctx.account_at_height(USER_B, 5).unwrap().balance, U256::from(10 * ONE_RU_USDC));
        assert_eq!(ctx.account_at_height(USER_C, 5).unwrap().balance, U256::from(15 * ONE_RU_USDC));
        assert!(ctx.account_at_height(USER_D, 5).is_none());
        assert!(ctx.account_at_height(USER_E, 5).is_none());
        assert!(ctx.account_at_height(USER_F, 5).is_none());
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_write_changesets() {
    run_test(|ctx| async move {
        let ctx = setup_accounts_history(ctx).await;

        // The changeset at block N records the state before block N.
        // get_account_at_height(addr, N-1) gives the state at end of block N-1,
        // which is the "before" state for block N.
        let acct_a_at_3 = ctx.account_at_height(USER_A, 3).unwrap();
        let acct_b_at_3 = ctx.account_at_height(USER_B, 3).unwrap();
        let acct_c_at_3 = ctx.account_at_height(USER_C, 3).unwrap();

        assert_eq!(acct_a_at_3.balance, U256::from(3 * ONE_RU_USDC));
        assert_eq!(acct_b_at_3.balance, U256::from(6 * ONE_RU_USDC));
        assert_eq!(acct_c_at_3.balance, U256::from(9 * ONE_RU_USDC));

        let acct_a_at_4 = ctx.account_at_height(USER_A, 4).unwrap();
        let acct_b_at_4 = ctx.account_at_height(USER_B, 4).unwrap();
        let acct_c_at_4 = ctx.account_at_height(USER_C, 4).unwrap();

        assert_eq!(acct_a_at_4.balance, U256::from(4 * ONE_RU_USDC));
        assert_eq!(acct_b_at_4.balance, U256::from(8 * ONE_RU_USDC));
        assert_eq!(acct_c_at_4.balance, U256::from(12 * ONE_RU_USDC));
    })
    .await;
}

#[serial]
#[tokio::test]
async fn test_write_changesets_with_empty_blocks() {
    run_test(|ctx| async move {
        let ctx = setup_accounts_history(ctx).await;

        let acct_a_at_3 = ctx.account_at_height(USER_A, 3).unwrap();
        let acct_b_at_3 = ctx.account_at_height(USER_B, 3).unwrap();
        let acct_c_at_3 = ctx.account_at_height(USER_C, 3).unwrap();

        assert_eq!(acct_a_at_3.balance, U256::from(3 * ONE_RU_USDC));
        assert_eq!(acct_b_at_3.balance, U256::from(6 * ONE_RU_USDC));
        assert_eq!(acct_c_at_3.balance, U256::from(9 * ONE_RU_USDC));

        let acct_a_at_4 = ctx.account_at_height(USER_A, 4).unwrap();
        let acct_b_at_4 = ctx.account_at_height(USER_B, 4).unwrap();
        let acct_c_at_4 = ctx.account_at_height(USER_C, 4).unwrap();

        assert_eq!(acct_a_at_4.balance, U256::from(4 * ONE_RU_USDC));
        assert_eq!(acct_b_at_4.balance, U256::from(8 * ONE_RU_USDC));
        assert_eq!(acct_c_at_4.balance, U256::from(12 * ONE_RU_USDC));

        let empty_block = HostBlockSpec::new(ctx.constants());
        ctx.process_blocks(vec![empty_block; 2]).await.unwrap();

        // Even after processing empty blocks, the historical state should not change.
        let acct_a_at_3 = ctx.account_at_height(USER_A, 3).unwrap();
        let acct_b_at_3 = ctx.account_at_height(USER_B, 3).unwrap();
        let acct_c_at_3 = ctx.account_at_height(USER_C, 3).unwrap();

        assert_eq!(acct_a_at_3.balance, U256::from(3 * ONE_RU_USDC));
        assert_eq!(acct_b_at_3.balance, U256::from(6 * ONE_RU_USDC));
        assert_eq!(acct_c_at_3.balance, U256::from(9 * ONE_RU_USDC));

        let acct_a_at_4 = ctx.account_at_height(USER_A, 4).unwrap();
        let acct_b_at_4 = ctx.account_at_height(USER_B, 4).unwrap();
        let acct_c_at_4 = ctx.account_at_height(USER_C, 4).unwrap();

        assert_eq!(acct_a_at_4.balance, U256::from(4 * ONE_RU_USDC));
        assert_eq!(acct_b_at_4.balance, U256::from(8 * ONE_RU_USDC));
        assert_eq!(acct_c_at_4.balance, U256::from(12 * ONE_RU_USDC));
    })
    .await;
}
