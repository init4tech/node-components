#[path = "./common/mod.rs"]
mod test_common;

use alloy::{
    consensus::{BlockBody, BlockHeader, Signed, TxEip1559, TxEnvelope},
    primitives::{Address, B256, U256},
    signers::Signature,
};
use reth::providers::{BlockNumReader, BlockReader};
use signet_constants::test_utils::{DEPLOY_HEIGHT, RU_CHAIN_ID};
use signet_db::RuWriter;
use signet_types::primitives::{RecoveredBlock, SealedBlock, SealedHeader};
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

    let block = RecoveredBlock {
        block: SealedBlock {
            header: SealedHeader::new(alloy::consensus::Header::default()),
            body: BlockBody {
                transactions: std::iter::repeat(
                    TxEnvelope::Eip1559(Signed::new_unhashed(
                        TxEip1559::default(),
                        Signature::test_signature(),
                    ))
                    .into(),
                )
                .take(10)
                .collect(),
                ommers: vec![],
                withdrawals: None,
            },
        },
        senders: std::iter::repeat(Address::repeat_byte(0x33)).take(10).collect(),
    };

    writer
        .insert_signet_block(header, &block, journal_hash, reth::providers::StorageLocation::Both)
        .unwrap();

    // Check basic updates
    assert_eq!(writer.last_block_number().unwrap(), block.number());
    assert_eq!(writer.latest_journal_hash().unwrap(), journal_hash);
    assert_eq!(writer.get_journal_hash(block.number()).unwrap(), Some(journal_hash));
    // This tests resolving `BlockId::Latest`
    assert_eq!(writer.best_block_number().unwrap(), block.number());

    // Check that the block can be loaded back
    let loaded_block = writer
        .recovered_block_range(block.number()..=block.number())
        .unwrap()
        .first()
        .cloned()
        .unwrap();
    assert_eq!(loaded_block.header(), block.block.header.header());
    assert_eq!(loaded_block.body().transactions.len(), block.block.body.transactions.len());

    // Check that the ZenithHeader can be loaded back
    let loaded_header = writer.get_signet_header(block.number()).unwrap();
    assert_eq!(loaded_header, header);
}
