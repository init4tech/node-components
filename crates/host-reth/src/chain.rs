use crate::shim::RecoveredBlockShim;
use alloy::{consensus::Block, consensus::BlockHeader};
use reth::primitives::{EthPrimitives, RecoveredBlock};
use reth::providers::Chain;
use signet_extract::{BlockAndReceipts, Extractable};
use signet_types::primitives::TransactionSigned;
use std::sync::Arc;

/// Reth's recovered block type, aliased for readability.
type RethRecovered = RecoveredBlock<Block<TransactionSigned>>;

/// An owning wrapper around reth's [`Chain`] that implements [`Extractable`]
/// with O(1) metadata accessors.
///
/// # Usage
///
/// `RethChain` is typically obtained from [`HostNotification`] events, not
/// constructed directly. To extract blocks and receipts:
///
/// ```ignore
/// # // Requires reth ExEx runtime — shown for API illustration only.
/// use signet_extract::Extractable;
///
/// fn process(chain: &RethChain) {
///     for bar in chain.blocks_and_receipts() {
///         println!("block receipts: {}", bar.receipts.len());
///     }
/// }
/// ```
///
/// [`HostNotification`]: signet_node_types::HostNotification
#[derive(Debug)]
pub struct RethChain {
    inner: Arc<Chain<EthPrimitives>>,
}

impl RethChain {
    /// Wrap a reth chain.
    pub const fn new(chain: Arc<Chain<EthPrimitives>>) -> Self {
        Self { inner: chain }
    }
}

impl Extractable for RethChain {
    type Block = RecoveredBlockShim;
    type Receipt = reth::primitives::Receipt;

    fn blocks_and_receipts(
        &self,
    ) -> impl Iterator<Item = BlockAndReceipts<'_, Self::Block, Self::Receipt>> {
        self.inner.blocks_and_receipts().map(|(block, receipts)| {
            // Compile-time check: RecoveredBlockShim must have the same
            // layout as RethRecovered (guaranteed by #[repr(transparent)]
            // on RecoveredBlockShim in signet-blobber/src/shim.rs).
            const {
                assert!(
                    size_of::<RecoveredBlockShim>() == size_of::<RethRecovered>(),
                    "RecoveredBlockShim layout diverged from RethRecovered"
                );
                assert!(
                    align_of::<RecoveredBlockShim>() == align_of::<RethRecovered>(),
                    "RecoveredBlockShim alignment diverged from RethRecovered"
                );
            }
            // SAFETY: `RecoveredBlockShim` is `#[repr(transparent)]` over
            // `RethRecovered`, so these types have identical memory layouts.
            // The lifetime of the reference is tied to `self.inner` (the
            // `Arc<Chain>`), which outlives the returned iterator.
            let block =
                unsafe { std::mem::transmute::<&RethRecovered, &RecoveredBlockShim>(block) };
            BlockAndReceipts { block, receipts }
        })
    }

    fn first_number(&self) -> u64 {
        self.inner.first().number()
    }

    fn tip_number(&self) -> u64 {
        self.inner.tip().number()
    }

    fn len(&self) -> usize {
        self.inner.len()
    }
}

/// Unifies DB-backfill and live ExEx chain segments.
///
/// During startup, the notifier emits `Backfill` segments read directly
/// from the reth DB. Once backfill is complete, it switches to `Live`
/// segments from the ExEx notification stream. Both variants implement
/// [`Extractable`] so the node processes them identically.
#[derive(Debug)]
pub enum HostChain {
    /// A segment read from the reth DB during backfill.
    Backfill(crate::backfill::DbChainSegment),
    /// A segment from the live ExEx notification stream.
    Live(RethChain),
}

impl Extractable for HostChain {
    type Block = RecoveredBlockShim;
    type Receipt = reth::primitives::Receipt;

    fn blocks_and_receipts(
        &self,
    ) -> impl Iterator<Item = BlockAndReceipts<'_, Self::Block, Self::Receipt>> {
        match self {
            Self::Backfill(segment) => Box::new(segment.blocks_and_receipts())
                as Box<dyn Iterator<Item = BlockAndReceipts<'_, Self::Block, Self::Receipt>>>,
            Self::Live(chain) => Box::new(chain.blocks_and_receipts()),
        }
    }

    fn first_number(&self) -> u64 {
        match self {
            Self::Backfill(segment) => segment.first_number(),
            Self::Live(chain) => chain.first_number(),
        }
    }

    fn tip_number(&self) -> u64 {
        match self {
            Self::Backfill(segment) => segment.tip_number(),
            Self::Live(chain) => chain.tip_number(),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Backfill(segment) => segment.len(),
            Self::Live(chain) => chain.len(),
        }
    }
}
