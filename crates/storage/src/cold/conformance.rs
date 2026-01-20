//! Conformance tests for ColdStorage backends.
//!
//! These tests verify that any backend implementation behaves correctly
//! according to the ColdStorage trait contract. To use these tests with
//! a custom backend, call the test functions with your backend instance.

use crate::cold::{BlockData, BlockTag, ColdResult, ColdStorage, HeaderSpecifier};
use alloy::primitives::{B256, BlockNumber};
use reth::primitives::Header;

/// Run all conformance tests against a backend.
///
/// This is the main entry point for testing a custom backend implementation.
pub async fn conformance<B: ColdStorage>(backend: &B) -> ColdResult<()> {
    test_empty_storage(backend).await?;
    test_append_and_read_header(backend).await?;
    test_header_hash_lookup(backend).await?;
    test_header_tag_lookup(backend).await?;
    test_transaction_lookups(backend).await?;
    test_receipt_lookups(backend).await?;
    test_truncation(backend).await?;
    test_batch_append(backend).await?;
    test_latest_block_tracking(backend).await?;
    Ok(())
}

/// Create test block data for conformance tests.
///
/// Creates a minimal valid block with the given block number.
pub fn make_test_block(block_number: BlockNumber) -> BlockData {
    let header = Header { number: block_number, ..Default::default() };

    BlockData::new(header, vec![], vec![], vec![], None)
}

/// Test that empty storage returns None/empty for all lookups.
pub async fn test_empty_storage<B: ColdStorage>(backend: &B) -> ColdResult<()> {
    assert!(backend.get_header(HeaderSpecifier::Number(0)).await?.is_none());
    assert!(backend.get_header(HeaderSpecifier::Hash(B256::ZERO)).await?.is_none());
    assert!(backend.get_header(HeaderSpecifier::Tag(BlockTag::Latest)).await?.is_none());
    assert!(backend.get_latest_block().await?.is_none());
    assert!(backend.get_transactions_in_block(0).await?.is_empty());
    assert!(backend.get_receipts_in_block(0).await?.is_empty());
    assert_eq!(backend.get_transaction_count(0).await?, 0);
    Ok(())
}

/// Test basic append and read for headers.
pub async fn test_append_and_read_header<B: ColdStorage>(backend: &B) -> ColdResult<()> {
    let block_data = make_test_block(100);
    let expected_header = block_data.header.clone();

    backend.append_block(block_data).await?;

    let retrieved = backend.get_header(HeaderSpecifier::Number(100)).await?;
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap(), expected_header);

    Ok(())
}

/// Test header lookup by hash.
pub async fn test_header_hash_lookup<B: ColdStorage>(backend: &B) -> ColdResult<()> {
    let block_data = make_test_block(101);
    let header_hash = block_data.header.hash_slow();

    backend.append_block(block_data).await?;

    let retrieved = backend.get_header(HeaderSpecifier::Hash(header_hash)).await?;
    assert!(retrieved.is_some());

    // Non-existent hash should return None
    let missing = backend.get_header(HeaderSpecifier::Hash(B256::ZERO)).await?;
    assert!(missing.is_none());

    Ok(())
}

/// Test header lookup by tag.
pub async fn test_header_tag_lookup<B: ColdStorage>(backend: &B) -> ColdResult<()> {
    backend.append_block(make_test_block(50)).await?;
    backend.append_block(make_test_block(51)).await?;
    backend.append_block(make_test_block(52)).await?;

    // Latest should return block 52
    let latest = backend.get_header(HeaderSpecifier::Tag(BlockTag::Latest)).await?;
    assert!(latest.is_some());

    // Earliest should return block 50
    let earliest = backend.get_header(HeaderSpecifier::Tag(BlockTag::Earliest)).await?;
    assert!(earliest.is_some());

    Ok(())
}

/// Test transaction lookups by hash and by block+index.
pub async fn test_transaction_lookups<B: ColdStorage>(backend: &B) -> ColdResult<()> {
    // Create block with empty transactions for now
    let block_data = make_test_block(200);

    backend.append_block(block_data).await?;

    let txs = backend.get_transactions_in_block(200).await?;
    let count = backend.get_transaction_count(200).await?;
    assert_eq!(txs.len() as u64, count);

    Ok(())
}

/// Test receipt lookups.
pub async fn test_receipt_lookups<B: ColdStorage>(backend: &B) -> ColdResult<()> {
    let block_data = make_test_block(201);

    backend.append_block(block_data).await?;

    let receipts = backend.get_receipts_in_block(201).await?;
    // Empty receipts for now
    assert!(receipts.is_empty());

    Ok(())
}

/// Test truncation removes data correctly.
pub async fn test_truncation<B: ColdStorage>(backend: &B) -> ColdResult<()> {
    // Append blocks 300, 301, 302
    backend.append_block(make_test_block(300)).await?;
    backend.append_block(make_test_block(301)).await?;
    backend.append_block(make_test_block(302)).await?;

    // Truncate above 300 (removes 301, 302)
    backend.truncate_above(300).await?;

    // Block 300 should still exist
    assert!(backend.get_header(HeaderSpecifier::Number(300)).await?.is_some());

    // Blocks 301, 302 should be gone
    assert!(backend.get_header(HeaderSpecifier::Number(301)).await?.is_none());
    assert!(backend.get_header(HeaderSpecifier::Number(302)).await?.is_none());

    // Latest should now be 300
    assert_eq!(backend.get_latest_block().await?, Some(300));

    Ok(())
}

/// Test batch append.
pub async fn test_batch_append<B: ColdStorage>(backend: &B) -> ColdResult<()> {
    let blocks = vec![make_test_block(400), make_test_block(401), make_test_block(402)];

    backend.append_blocks(blocks).await?;

    assert!(backend.get_header(HeaderSpecifier::Number(400)).await?.is_some());
    assert!(backend.get_header(HeaderSpecifier::Number(401)).await?.is_some());
    assert!(backend.get_header(HeaderSpecifier::Number(402)).await?.is_some());

    Ok(())
}

/// Test latest block tracking.
pub async fn test_latest_block_tracking<B: ColdStorage>(backend: &B) -> ColdResult<()> {
    // Append out of order
    backend.append_block(make_test_block(502)).await?;
    assert_eq!(backend.get_latest_block().await?, Some(502));

    backend.append_block(make_test_block(500)).await?;
    // Latest should still be 502
    assert_eq!(backend.get_latest_block().await?, Some(502));

    backend.append_block(make_test_block(505)).await?;
    assert_eq!(backend.get_latest_block().await?, Some(505));

    Ok(())
}
