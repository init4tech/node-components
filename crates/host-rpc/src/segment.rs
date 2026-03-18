use alloy::consensus::{BlockHeader, ReceiptEnvelope};
use signet_extract::{BlockAndReceipts, Extractable};
use signet_types::primitives::RecoveredBlock;
use std::sync::Arc;

/// A block with its receipts, fetched via RPC.
#[derive(Debug)]
pub struct RpcBlock {
    /// The recovered block (with senders).
    pub(crate) block: RecoveredBlock,
    /// The receipts for this block's transactions.
    pub(crate) receipts: Vec<ReceiptEnvelope>,
}

impl RpcBlock {
    /// The block number.
    pub fn number(&self) -> u64 {
        self.block.number()
    }

    /// The block hash.
    pub const fn hash(&self) -> alloy::primitives::B256 {
        self.block.header.hash()
    }

    /// The parent block hash.
    pub fn parent_hash(&self) -> alloy::primitives::B256 {
        self.block.parent_hash()
    }
}

/// A chain segment fetched via RPC.
///
/// Contains one or more blocks with their receipts, ordered by block number
/// ascending. Blocks are wrapped in [`Arc`] for cheap sharing with the
/// notifier's internal buffer.
#[derive(Debug)]
pub struct RpcChainSegment {
    blocks: Vec<Arc<RpcBlock>>,
}

impl RpcChainSegment {
    /// Create a new segment from a list of blocks.
    pub const fn new(blocks: Vec<Arc<RpcBlock>>) -> Self {
        Self { blocks }
    }
}

impl Extractable for RpcChainSegment {
    type Block = RecoveredBlock;
    type Receipt = ReceiptEnvelope;

    fn blocks_and_receipts(
        &self,
    ) -> impl Iterator<Item = BlockAndReceipts<'_, Self::Block, Self::Receipt>> {
        self.blocks.iter().map(|b| BlockAndReceipts { block: &b.block, receipts: &b.receipts })
    }

    fn first_number(&self) -> u64 {
        self.blocks.first().map(|b| b.number()).unwrap_or(0)
    }

    fn tip_number(&self) -> u64 {
        self.blocks.last().map(|b| b.number()).unwrap_or(0)
    }

    fn len(&self) -> usize {
        self.blocks.len()
    }
}
