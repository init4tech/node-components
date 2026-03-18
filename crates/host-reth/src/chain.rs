use alloy::{consensus::Block, consensus::BlockHeader};
use reth::primitives::{EthPrimitives, RecoveredBlock};
use reth::providers::Chain;
use signet_blobber::RecoveredBlockShim;
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
