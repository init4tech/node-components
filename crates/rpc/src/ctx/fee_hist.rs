use reth::{
    core::primitives::SealedBlock,
    primitives::{Block, RecoveredBlock},
    providers::{CanonStateNotification, Chain},
};
use signet_types::MagicSig;
use std::sync::Arc;

/// Removes Signet system transactions from the block.
fn strip_block(block: RecoveredBlock<Block>) -> RecoveredBlock<Block> {
    let (sealed, senders) = block.split_sealed();
    let (header, mut body) = sealed.split_sealed_header_body();

    // This is the index of the first transaction that has a system magic
    // signature.
    let sys_index = body
        .transactions
        .partition_point(|tx| MagicSig::try_from_signature(tx.signature()).is_some());

    body.transactions.truncate(sys_index);

    let sealed = SealedBlock::from_sealed_parts(header, body);
    RecoveredBlock::new_sealed(sealed, senders)
}

/// Removes Signet system transactions from the chain. This function uses
/// `Arc::make_mut` to clone the contents of the Arc and modify the new
/// instance.
fn strip_chain(chain: &mut Arc<Chain>) {
    let chain = Arc::make_mut(chain);

    let (blocks, outcome, trie) = std::mem::take(chain).into_inner();
    let blocks = blocks.into_blocks().map(strip_block);

    *chain = Chain::new(blocks, outcome, trie);
}

/// Strips Signet system transactions from the `CanonStateNotification`.
pub(crate) fn strip_signet_system_txns(notif: CanonStateNotification) -> CanonStateNotification {
    // Cloning here ensures that the `make_mut` invocations in
    // `strip_chain` do not ever modify the original `notif` object.
    let _c = notif.clone();

    match notif {
        CanonStateNotification::Commit { mut new } => {
            strip_chain(&mut new);
            CanonStateNotification::Commit { new }
        }
        CanonStateNotification::Reorg { mut old, mut new } => {
            strip_chain(&mut old);
            strip_chain(&mut new);

            CanonStateNotification::Reorg { old, new }
        }
    }
}
