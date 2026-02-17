#[path = "./common/mod.rs"]
mod test_common;

use alloy::{
    consensus::{BlockHeader, Signed, TxEip1559, TxEnvelope},
    primitives::{Address, B256, U256},
    signers::Signature,
};
use reth::providers::{BlockNumReader, BlockReader};
use signet_constants::test_utils::{DEPLOY_HEIGHT, RU_CHAIN_ID};
use signet_db::RuWriter;
use signet_types::primitives::{SealedBlock, SealedHeader, TransactionSigned};
use signet_zenith::Zenith;

#[test]
fn test_ru_writer() {
    let factory = test_common::create_test_provider_factory();

    let writer = factory.provider_rw().unwrap();

    dbg!(writer.last_block_number().unwrap());
}

#[test]
fn test_insert_signet_block() {
    let factory = test_common::create_test_provider_factory();
    let writer = factory.provider_rw().unwrap();

    let journal_hash = B256::repeat_byte(0x55);
    let header = Some(Zenith::BlockHeader {
        rollupChainId: U256::from(RU_CHAIN_ID),
        hostBlockNumber: U256::from(DEPLOY_HEIGHT),
        gasLimit: U256::from(30_000_000),
        rewardAddress: Address::repeat_byte(0x11),
        blockDataHash: B256::repeat_byte(0x22),
    });

    let transactions: Vec<TransactionSigned> = std::iter::repeat_n(
        TxEnvelope::Eip1559(Signed::new_unhashed(
            TxEip1559::default(),
            Signature::test_signature(),
        ))
        .into(),
        10,
    )
    .collect();
    let senders: Vec<Address> = std::iter::repeat_n(Address::repeat_byte(0x33), 10).collect();
    let sealed =
        SealedBlock::new(SealedHeader::new(alloy::consensus::Header::default()), transactions);
    let block = sealed.recover_unchecked(senders);

    writer.insert_signet_block(header, &block, journal_hash).unwrap();
    writer.commit().unwrap();

    let reader = factory.provider_rw().unwrap();

    // Check basic updates
    assert_eq!(reader.last_block_number().unwrap(), block.number());
    assert_eq!(reader.latest_journal_hash().unwrap(), journal_hash);
    assert_eq!(reader.get_journal_hash(block.number()).unwrap(), Some(journal_hash));
    // This tests resolving `BlockId::Latest`
    assert_eq!(reader.best_block_number().unwrap(), block.number());

    // Check that the block can be loaded back
    let loaded_block = reader
        .recovered_block_range(block.number()..=block.number())
        .unwrap()
        .first()
        .cloned()
        .unwrap();
    assert_eq!(loaded_block.header(), block.header.inner());
    assert_eq!(loaded_block.body().transactions.len(), block.transactions.len());

    // Check that the ZenithHeader can be loaded back
    let loaded_header = reader.get_signet_header(block.number()).unwrap();
    assert_eq!(loaded_header, header);
}

#[test]
fn test_transaction_hash_indexing() {
    use reth::providers::TransactionsProvider;
    use reth_db::{cursor::DbCursorRO, tables, transaction::DbTx};

    let factory = test_common::create_test_provider_factory();
    let writer = factory.provider_rw().unwrap();

    let journal_hash = B256::repeat_byte(0x55);
    let header = Some(Zenith::BlockHeader {
        rollupChainId: U256::from(RU_CHAIN_ID),
        hostBlockNumber: U256::from(DEPLOY_HEIGHT),
        gasLimit: U256::from(30_000_000),
        rewardAddress: Address::repeat_byte(0x11),
        blockDataHash: B256::repeat_byte(0x22),
    });

    // Create transactions with distinct content so they have different hashes
    let transactions: Vec<TransactionSigned> = (0..5u64)
        .map(|i| {
            let tx = TxEip1559 { nonce: i, ..Default::default() };
            TxEnvelope::Eip1559(Signed::new_unhashed(tx, Signature::test_signature())).into()
        })
        .collect();

    // Collect the expected hashes BEFORE inserting
    let expected_hashes: Vec<B256> =
        transactions.iter().map(|tx: &TransactionSigned| *tx.hash()).collect();

    let senders: Vec<Address> = std::iter::repeat_n(Address::repeat_byte(0x33), 5).collect();
    let sealed =
        SealedBlock::new(SealedHeader::new(alloy::consensus::Header::default()), transactions);
    let block = sealed.recover_unchecked(senders);

    writer.insert_signet_block(header, &block, journal_hash).unwrap();
    writer.commit().unwrap();

    let reader = factory.provider_rw().unwrap();

    // Verify each transaction hash is in the index
    for (idx, expected_hash) in expected_hashes.iter().enumerate() {
        // Method 1: Use provider's transaction_by_hash
        let tx_result = reader.transaction_by_hash(*expected_hash).unwrap();
        assert!(
            tx_result.is_some(),
            "transaction_by_hash failed for tx {} with hash {}",
            idx,
            expected_hash
        );

        // Method 2: Query TransactionHashNumbers directly
        let mut cursor = reader.tx_ref().cursor_read::<tables::TransactionHashNumbers>().unwrap();
        let index_result = cursor.seek_exact(*expected_hash).unwrap();
        assert!(
            index_result.is_some(),
            "TransactionHashNumbers entry missing for tx {} with hash {}",
            idx,
            expected_hash
        );

        let (hash, tx_num) = index_result.unwrap();
        assert_eq!(hash, *expected_hash, "Hash mismatch in index for tx {}", idx);
        assert_eq!(tx_num, idx as u64, "Unexpected tx_num for tx {}", idx);
    }

    // Verify hashes match when loading block back from storage
    let loaded_block = reader
        .recovered_block_range(block.number()..=block.number())
        .unwrap()
        .first()
        .cloned()
        .unwrap();

    for (idx, (original_hash, loaded_tx)) in
        expected_hashes.iter().zip(loaded_block.body().transactions.iter()).enumerate()
    {
        let loaded_hash = *loaded_tx.hash();
        assert_eq!(
            *original_hash, loaded_hash,
            "Hash mismatch after load for tx {}: original={}, loaded={}",
            idx, original_hash, loaded_hash
        );
    }
}
