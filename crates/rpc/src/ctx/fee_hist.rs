use reth::{
    core::primitives::SealedBlock,
    primitives::{Block, RecoveredBlock},
    providers::{CanonStateNotification, Chain},
};
use signet_types::MagicSig;
use std::sync::Arc;

/// Removes Signet system transactions from the block.
fn strip_block(block: RecoveredBlock<Block>) -> RecoveredBlock<Block> {
    let (sealed, mut senders) = block.split_sealed();
    let (header, mut body) = sealed.split_sealed_header_body();

    // This is the index of the first transaction that has a system magic
    // signature.
    let sys_index = body
        .transactions
        .partition_point(|tx| MagicSig::try_from_signature(tx.signature()).is_none());

    body.transactions.truncate(sys_index);
    senders.truncate(sys_index);

    let sealed = SealedBlock::from_sealed_parts(header, body);

    RecoveredBlock::new_sealed(sealed, senders)
}

/// Removes Signet system transactions from the chain. This function uses
/// `Arc::make_mut` to clone the contents of the Arc and modify the new
/// instance.
fn strip_chain(chain: &Chain) -> Arc<Chain> {
    // Takes the contents out, replacing with default
    let (blocks, outcome, trie, hashed) = chain.clone().into_inner();

    // Strip each block
    let blocks: Vec<RecoveredBlock<Block>> = blocks.into_blocks().map(strip_block).collect();

    // Replace the original chain with the stripped version
    Arc::new(Chain::new(blocks, outcome, trie, hashed))
}

/// Strips Signet system transactions from the `CanonStateNotification`.
pub(crate) fn strip_signet_system_txns(notif: CanonStateNotification) -> CanonStateNotification {
    match notif {
        CanonStateNotification::Commit { new } => {
            CanonStateNotification::Commit { new: strip_chain(&new) }
        }
        CanonStateNotification::Reorg { mut old, mut new } => {
            old = strip_chain(&old);
            new = strip_chain(&new);

            CanonStateNotification::Reorg { old, new }
        }
    }
}

#[cfg(test)]
mod test {
    use alloy::{
        consensus::{TxEip1559, TxEnvelope},
        primitives::{Address, B256},
        signers::Signature,
    };
    use reth::primitives::{BlockBody, Header, SealedHeader};

    use super::*;

    fn test_magic_sig_tx() -> TxEnvelope {
        let sig = MagicSig::enter(B256::repeat_byte(0x22), 3);

        let sig = sig.into();

        dbg!(MagicSig::try_from_signature(&sig).is_some());

        TxEnvelope::new_unchecked(TxEip1559::default().into(), sig, B256::repeat_byte(0x33))
    }

    fn test_non_magic_sig_tx() -> TxEnvelope {
        let sig = Signature::test_signature();
        TxEnvelope::new_unchecked(TxEip1559::default().into(), sig, B256::repeat_byte(0x44))
    }

    fn test_block_body() -> BlockBody {
        BlockBody {
            transactions: vec![
                test_non_magic_sig_tx().into(),
                test_non_magic_sig_tx().into(),
                test_magic_sig_tx().into(),
                test_magic_sig_tx().into(),
            ],
            ..Default::default()
        }
    }

    fn test_sealed_header(number: u64) -> SealedHeader {
        let header = Header { number, ..Default::default() };
        SealedHeader::new_unhashed(header)
    }

    fn test_sealed_block(block_num: u64) -> SealedBlock<Block> {
        SealedBlock::from_sealed_parts(test_sealed_header(block_num), test_block_body())
    }

    fn test_block(block_num: u64) -> RecoveredBlock<Block> {
        RecoveredBlock::new_sealed(
            test_sealed_block(block_num),
            vec![Address::repeat_byte(0x11); 4],
        )
    }

    fn test_chain(count: u64) -> Arc<Chain> {
        let blocks = (0..count).map(test_block);
        Arc::new(Chain::new(blocks, Default::default(), Default::default(), Default::default()))
    }

    #[test]
    fn test_strip_block() {
        let block = test_block(0);
        assert_eq!(block.body().transactions.len(), 4);
        assert_eq!(block.senders().len(), 4);

        let stripped = strip_block(block);
        assert_eq!(stripped.body().transactions.len(), 2);
        assert_eq!(stripped.senders().len(), 2);

        for tx in stripped.body().transactions.iter() {
            assert!(MagicSig::try_from_signature(tx.signature()).is_none());
        }
    }

    #[test]
    fn test_strip_chain() {
        let original = test_chain(2);
        assert_eq!(original.blocks().len(), 2);

        let chain = strip_chain(&original);

        assert_ne!(&*chain, &*original);

        assert_eq!(chain.blocks().len(), 2);

        for (_num, block) in chain.blocks().iter() {
            assert_eq!(block.body().transactions.len(), 2);
            assert_eq!(block.senders().len(), 2);
            for tx in block.body().transactions.iter() {
                assert!(MagicSig::try_from_signature(tx.signature()).is_none());
            }
        }
    }
}
