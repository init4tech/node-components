use alloy::{
    primitives::{Address, B256, LogData},
    providers::Provider,
    rpc::types::eth::{Filter, Log},
    sol_types::{SolCall, SolEvent},
};
use serial_test::serial;
use signet_node_tests::{HostBlockSpec, SignetTestContext, rpc_test, run_test, types::Counter};
use std::time::Duration;

const SOME_USER: Address = Address::repeat_byte(0x39);

/// Helper: build and process an increment block using a host system
/// transaction (`simple_transact`). This avoids the transaction pool
/// entirely, which is important for reorg tests where we revert and
/// rebuild blocks.
///
/// Returns the `HostBlockSpec` so it can later be reverted.
fn increment_block(ctx: &SignetTestContext, contract_address: Address) -> HostBlockSpec {
    ctx.start_host_block().simple_transact(
        ctx.addresses[1],
        contract_address,
        Counter::incrementCall::SELECTOR,
        0,
    )
}

/// Process an increment block and return the spec for later revert.
async fn process_increment(ctx: &SignetTestContext, contract_address: Address) -> HostBlockSpec {
    let block = increment_block(ctx, contract_address);
    let for_revert = block.clone();
    ctx.process_block(block).await.unwrap();
    for_revert
}

// ---------------------------------------------------------------------------
// 1. Block tags
// ---------------------------------------------------------------------------

#[serial]
#[tokio::test]
async fn test_block_tags_reorg() {
    run_test(|ctx| async move {
        // Process two blocks via enter events.
        let block1 = HostBlockSpec::new(ctx.constants()).enter_token(
            SOME_USER,
            1000,
            ctx.constants().host().tokens().usdc(),
        );
        let block1_clone = block1.clone();
        ctx.process_block(block1).await.unwrap();

        let block2 = HostBlockSpec::new(ctx.constants()).enter_token(
            SOME_USER,
            2000,
            ctx.constants().host().tokens().usdc(),
        );
        let block2_clone = block2.clone();
        ctx.process_block(block2).await.unwrap();

        assert_eq!(ctx.alloy_provider.get_block_number().await.unwrap(), 2);

        // Revert block 2.
        ctx.revert_block(block2_clone).await.unwrap();
        assert_eq!(ctx.alloy_provider.get_block_number().await.unwrap(), 1);

        // Revert block 1.
        ctx.revert_block(block1_clone).await.unwrap();
        assert_eq!(ctx.alloy_provider.get_block_number().await.unwrap(), 0);

        // Rebuild two new blocks.
        let new_block1 = HostBlockSpec::new(ctx.constants()).enter_token(
            SOME_USER,
            500,
            ctx.constants().host().tokens().usdc(),
        );
        ctx.process_block(new_block1).await.unwrap();
        assert_eq!(ctx.alloy_provider.get_block_number().await.unwrap(), 1);

        let new_block2 = HostBlockSpec::new(ctx.constants()).enter_token(
            SOME_USER,
            600,
            ctx.constants().host().tokens().usdc(),
        );
        ctx.process_block(new_block2).await.unwrap();
        assert_eq!(ctx.alloy_provider.get_block_number().await.unwrap(), 2);

        // Verify the new block 2 is accessible.
        let block = ctx.alloy_provider.get_block_by_number(2.into()).await.unwrap();
        assert!(block.is_some());
    })
    .await;
}

// ---------------------------------------------------------------------------
// 2. Block filter + reorg
// ---------------------------------------------------------------------------

#[serial]
#[tokio::test]
async fn test_block_filter_reorg() {
    rpc_test(|ctx, contract| async move {
        // Install a block filter (starts after block 1, where contract was deployed).
        let filter_id = ctx.alloy_provider.new_block_filter().await.unwrap();

        // Process block 2 (increment via system tx).
        let _block2 = process_increment(&ctx, *contract.address()).await;

        // Poll: should have 1 block hash.
        let hashes: Vec<B256> = ctx.alloy_provider.get_filter_changes(filter_id).await.unwrap();
        assert_eq!(hashes.len(), 1);

        // Process block 3 (increment), keep clone for revert.
        let block3 = process_increment(&ctx, *contract.address()).await;

        // Revert block 3.
        ctx.revert_block(block3).await.unwrap();

        // Poll: reorg watermark resets start to ancestor+1 (= 3), but latest
        // is now 2, so start > latest -> empty.
        let hashes: Vec<B256> = ctx.alloy_provider.get_filter_changes(filter_id).await.unwrap();
        assert!(hashes.is_empty());

        // Process a new block 3.
        let _new_block3 = process_increment(&ctx, *contract.address()).await;

        // Poll: should return the new block 3 hash.
        let hashes: Vec<B256> = ctx.alloy_provider.get_filter_changes(filter_id).await.unwrap();
        assert_eq!(hashes.len(), 1);
        let new_block3_hash =
            ctx.alloy_provider.get_block_by_number(3.into()).await.unwrap().unwrap().hash();
        assert_eq!(hashes[0], new_block3_hash);

        ctx
    })
    .await;
}

// ---------------------------------------------------------------------------
// 3. Log filter + reorg
// ---------------------------------------------------------------------------

#[serial]
#[tokio::test]
async fn test_log_filter_reorg() {
    rpc_test(|ctx, contract| async move {
        // Install a log filter on the Counter address.
        let filter_id = ctx
            .alloy_provider
            .new_filter(&Filter::new().address(*contract.address()))
            .await
            .unwrap();

        // Process block 2 (increment -> count=1).
        let _block2 = process_increment(&ctx, *contract.address()).await;

        // Poll: 1 log.
        let logs: Vec<Log<LogData>> =
            ctx.alloy_provider.get_filter_changes(filter_id).await.unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].inner.topics()[0], Counter::Count::SIGNATURE_HASH);
        assert_eq!(logs[0].inner.topics()[1], B256::with_last_byte(1));

        // Process block 3 (increment -> count=2), clone for revert.
        let block3 = process_increment(&ctx, *contract.address()).await;

        // Revert block 3.
        ctx.revert_block(block3).await.unwrap();

        // Poll: empty (watermark rewinds, but latest < start).
        let logs: Vec<Log<LogData>> =
            ctx.alloy_provider.get_filter_changes(filter_id).await.unwrap();
        assert!(logs.is_empty());

        // Process a new block 3 (increment -> count=2 again).
        let _new_block3 = process_increment(&ctx, *contract.address()).await;

        // Poll: 1 log with count=2.
        let logs: Vec<Log<LogData>> =
            ctx.alloy_provider.get_filter_changes(filter_id).await.unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].inner.topics()[1], B256::with_last_byte(2));

        ctx
    })
    .await;
}

// ---------------------------------------------------------------------------
// 4. Block subscription + reorg
// ---------------------------------------------------------------------------

#[serial]
#[tokio::test]
async fn test_block_subscription_reorg() {
    rpc_test(|ctx, contract| async move {
        let mut sub = ctx.alloy_provider.subscribe_blocks().await.unwrap();

        // Process block 2.
        let block2 = process_increment(&ctx, *contract.address()).await;

        let header =
            tokio::time::timeout(Duration::from_secs(5), sub.recv()).await.unwrap().unwrap();
        assert_eq!(header.number, 2);

        // Revert block 2. Block subs do not emit anything for reorgs.
        ctx.revert_block(block2).await.unwrap();

        // Process a new block 2.
        let _new_block2 = process_increment(&ctx, *contract.address()).await;

        let header =
            tokio::time::timeout(Duration::from_secs(5), sub.recv()).await.unwrap().unwrap();
        assert_eq!(header.number, 2);

        ctx
    })
    .await;
}

// ---------------------------------------------------------------------------
// 5. Log subscription + reorg (removed: true)
// ---------------------------------------------------------------------------

#[serial]
#[tokio::test]
async fn test_log_subscription_reorg() {
    rpc_test(|ctx, contract| async move {
        let mut sub = ctx
            .alloy_provider
            .subscribe_logs(&Filter::new().address(*contract.address()))
            .await
            .unwrap();

        // Process block 2 (increment -> count=1).
        let block2 = process_increment(&ctx, *contract.address()).await;

        // Receive the normal log.
        let log = tokio::time::timeout(Duration::from_secs(5), sub.recv()).await.unwrap().unwrap();
        assert!(!log.removed);
        assert_eq!(log.inner.address, *contract.address());
        assert_eq!(log.inner.topics()[0], Counter::Count::SIGNATURE_HASH);
        assert_eq!(log.inner.topics()[1], B256::with_last_byte(1));

        // Revert block 2.
        ctx.revert_block(block2).await.unwrap();

        // Receive the removed log.
        let removed_log =
            tokio::time::timeout(Duration::from_secs(5), sub.recv()).await.unwrap().unwrap();
        assert!(removed_log.removed);
        assert_eq!(removed_log.inner.address, *contract.address());
        assert_eq!(removed_log.inner.topics()[0], Counter::Count::SIGNATURE_HASH);

        // Process a new block 2 (increment -> count=1 again).
        let _new_block2 = process_increment(&ctx, *contract.address()).await;

        // Receive the new log.
        let new_log =
            tokio::time::timeout(Duration::from_secs(5), sub.recv()).await.unwrap().unwrap();
        assert!(!new_log.removed);
        assert_eq!(new_log.inner.address, *contract.address());
        assert_eq!(new_log.inner.topics()[1], B256::with_last_byte(1));

        ctx
    })
    .await;
}

// ---------------------------------------------------------------------------
// 6. Log subscription filter selectivity during reorg
// ---------------------------------------------------------------------------

#[serial]
#[tokio::test]
async fn test_log_subscription_reorg_filter_selectivity() {
    rpc_test(|ctx, contract| async move {
        // Subscribe to logs on the Counter address (should receive events).
        let mut matching_sub = ctx
            .alloy_provider
            .subscribe_logs(&Filter::new().address(*contract.address()))
            .await
            .unwrap();

        // Subscribe to logs on a non-matching address (should receive nothing).
        let mut non_matching_sub =
            ctx.alloy_provider.subscribe_logs(&Filter::new().address(SOME_USER)).await.unwrap();

        // Process a block with an increment system tx.
        let block2 = process_increment(&ctx, *contract.address()).await;

        // The matching subscription should receive the log.
        let log = tokio::time::timeout(Duration::from_secs(5), matching_sub.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(!log.removed);
        assert_eq!(log.inner.address, *contract.address());

        // The non-matching subscription should receive nothing.
        let extra = tokio::time::timeout(Duration::from_millis(200), non_matching_sub.recv()).await;
        assert!(extra.is_err(), "non-matching sub should not receive the log");

        // Revert: only the matching subscription should get a removed log.
        ctx.revert_block(block2).await.unwrap();

        let removed = tokio::time::timeout(Duration::from_secs(5), matching_sub.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(removed.removed);
        assert_eq!(removed.inner.address, *contract.address());

        // The non-matching subscription should still receive nothing.
        let extra = tokio::time::timeout(Duration::from_millis(200), non_matching_sub.recv()).await;
        assert!(extra.is_err(), "non-matching sub should not receive removed log");

        ctx
    })
    .await;
}

// ---------------------------------------------------------------------------
// 7. No-regression: normal progression with filters and subscriptions
// ---------------------------------------------------------------------------

#[serial]
#[tokio::test]
async fn test_no_regression_filters_and_subscriptions() {
    rpc_test(|ctx, contract| async move {
        // Install filters.
        let block_filter = ctx.alloy_provider.new_block_filter().await.unwrap();
        let log_filter = ctx
            .alloy_provider
            .new_filter(&Filter::new().address(*contract.address()))
            .await
            .unwrap();

        // Subscribe.
        let mut block_sub = ctx.alloy_provider.subscribe_blocks().await.unwrap();
        let mut log_sub = ctx
            .alloy_provider
            .subscribe_logs(&Filter::new().address(*contract.address()))
            .await
            .unwrap();

        // Process 2 increments via system transactions.
        let _b2 = process_increment(&ctx, *contract.address()).await;
        let _b3 = process_increment(&ctx, *contract.address()).await;

        // Poll block filter: 2 hashes.
        let hashes: Vec<B256> = ctx.alloy_provider.get_filter_changes(block_filter).await.unwrap();
        assert_eq!(hashes.len(), 2);

        // Poll log filter: 2 logs with sequential counter values.
        let logs: Vec<Log<LogData>> =
            ctx.alloy_provider.get_filter_changes(log_filter).await.unwrap();
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].inner.topics()[1], B256::with_last_byte(1));
        assert_eq!(logs[1].inner.topics()[1], B256::with_last_byte(2));

        // Receive 2 block headers.
        for expected_num in [2, 3] {
            let header = tokio::time::timeout(Duration::from_secs(5), block_sub.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(header.number, expected_num);
        }

        // Receive 2 log events, all removed=false.
        for expected_count in [1u8, 2] {
            let log = tokio::time::timeout(Duration::from_secs(5), log_sub.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(!log.removed);
            assert_eq!(log.inner.address, *contract.address());
            assert_eq!(log.inner.topics()[1], B256::with_last_byte(expected_count));
        }

        ctx
    })
    .await;
}

// ---------------------------------------------------------------------------
// 8. Multi-block reorg with log filter
// ---------------------------------------------------------------------------

#[serial]
#[tokio::test]
async fn test_multi_block_reorg_log_filter() {
    rpc_test(|ctx, contract| async move {
        let addr = *contract.address();

        // Install a log filter on the Counter address.
        let filter_id = ctx.alloy_provider.new_filter(&Filter::new().address(addr)).await.unwrap();

        // Process blocks 2, 3, 4 (increment each → count 1, 2, 3).
        let block2 = process_increment(&ctx, addr).await;
        let block3 = process_increment(&ctx, addr).await;
        let block4 = process_increment(&ctx, addr).await;

        // Poll: expect 3 logs.
        let logs: Vec<Log<LogData>> =
            ctx.alloy_provider.get_filter_changes(filter_id).await.unwrap();
        assert_eq!(logs.len(), 3);
        for (i, log) in logs.iter().enumerate() {
            assert_eq!(log.inner.topics()[1], B256::with_last_byte(i as u8 + 1));
        }

        // Revert blocks 4, 3, 2 (back to block 1).
        ctx.revert_block(block4).await.unwrap();
        ctx.revert_block(block3).await.unwrap();
        ctx.revert_block(block2).await.unwrap();

        // Poll: empty (watermark rewinds start to 2, but latest=1).
        let logs: Vec<Log<LogData>> =
            ctx.alloy_provider.get_filter_changes(filter_id).await.unwrap();
        assert!(logs.is_empty());

        // Rebuild blocks 2, 3 (increment each → count 1, 2).
        let _new_b2 = process_increment(&ctx, addr).await;
        let _new_b3 = process_increment(&ctx, addr).await;

        // Poll: expect 2 logs with count 1, 2.
        let logs: Vec<Log<LogData>> =
            ctx.alloy_provider.get_filter_changes(filter_id).await.unwrap();
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].inner.topics()[1], B256::with_last_byte(1));
        assert_eq!(logs[1].inner.topics()[1], B256::with_last_byte(2));

        ctx
    })
    .await;
}

// ---------------------------------------------------------------------------
// 9. Multi-block reorg with log subscription
// ---------------------------------------------------------------------------

#[serial]
#[tokio::test]
async fn test_multi_block_reorg_log_subscription() {
    rpc_test(|ctx, contract| async move {
        let addr = *contract.address();
        let mut sub =
            ctx.alloy_provider.subscribe_logs(&Filter::new().address(addr)).await.unwrap();

        // Process blocks 2, 3 (increment each → count 1, 2).
        let block2 = process_increment(&ctx, addr).await;
        let block3 = process_increment(&ctx, addr).await;

        // Receive 2 normal logs.
        for expected in [1u8, 2] {
            let log =
                tokio::time::timeout(Duration::from_secs(5), sub.recv()).await.unwrap().unwrap();
            assert!(!log.removed);
            assert_eq!(log.inner.topics()[1], B256::with_last_byte(expected));
        }

        // Revert blocks 3, 2.
        ctx.revert_block(block3).await.unwrap();
        ctx.revert_block(block2).await.unwrap();

        // Receive 2 removed logs (one per reverted block).
        for _ in 0..2 {
            let log =
                tokio::time::timeout(Duration::from_secs(5), sub.recv()).await.unwrap().unwrap();
            assert!(log.removed);
            assert_eq!(log.inner.address, addr);
        }

        // Rebuild block 2 (increment → count 1).
        let _new_b2 = process_increment(&ctx, addr).await;

        // Receive the new normal log.
        let log = tokio::time::timeout(Duration::from_secs(5), sub.recv()).await.unwrap().unwrap();
        assert!(!log.removed);
        assert_eq!(log.inner.topics()[1], B256::with_last_byte(1));

        ctx
    })
    .await;
}

// ---------------------------------------------------------------------------
// 10. Multiple reorgs between polls (watermark min path)
// ---------------------------------------------------------------------------

#[serial]
#[tokio::test]
async fn test_multiple_reorgs_between_polls() {
    rpc_test(|ctx, contract| async move {
        let addr = *contract.address();

        let filter_id = ctx.alloy_provider.new_filter(&Filter::new().address(addr)).await.unwrap();

        // Process blocks 2, 3, 4, 5 (count 1, 2, 3, 4).
        let _b2 = process_increment(&ctx, addr).await;
        let _b3 = process_increment(&ctx, addr).await;
        let b4 = process_increment(&ctx, addr).await;
        let b5 = process_increment(&ctx, addr).await;

        // Poll to advance the filter cursor past block 5.
        let logs: Vec<Log<LogData>> =
            ctx.alloy_provider.get_filter_changes(filter_id).await.unwrap();
        assert_eq!(logs.len(), 4);

        // --- Reorg 1: revert block 5 only (common_ancestor=4, watermark=4) ---
        ctx.revert_block(b5).await.unwrap();

        // Rebuild block 5 (count=5).
        let new_b5 = process_increment(&ctx, addr).await;

        // --- Reorg 2 (deeper): revert blocks 5 AND 4 ---
        // common_ancestor for block 5 revert = 4, for block 4 revert = 3
        // watermark = min(4, min(4, 3)) = 3
        ctx.revert_block(new_b5).await.unwrap();
        ctx.revert_block(b4).await.unwrap();

        // DO NOT POLL between reorg 1 and reorg 2 — this is the key.
        // The filter now has watermark=3 from the min() of both reorgs.

        // Poll: empty (watermark+1=4, but latest=3 → start > latest).
        let logs: Vec<Log<LogData>> =
            ctx.alloy_provider.get_filter_changes(filter_id).await.unwrap();
        assert!(logs.is_empty());

        // Rebuild blocks 4, 5 (count 3, 4).
        let _new_b4 = process_increment(&ctx, addr).await;
        let _new_b5 = process_increment(&ctx, addr).await;

        // Poll: 2 logs from blocks 4 and 5 (count 3, 4).
        // After reverting blocks 4+5, count was back to 2. Rebuilding
        // increments to 3 then 4. This proves the deeper watermark (3)
        // was kept — if only the shallow watermark (4) had been kept,
        // we'd see 1 log from block 5 only.
        let logs: Vec<Log<LogData>> =
            ctx.alloy_provider.get_filter_changes(filter_id).await.unwrap();
        assert_eq!(logs.len(), 2);
        assert_eq!(logs[0].inner.topics()[1], B256::with_last_byte(3));
        assert_eq!(logs[1].inner.topics()[1], B256::with_last_byte(4));

        ctx
    })
    .await;
}
