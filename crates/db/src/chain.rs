use alloy::primitives::BlockNumber;
use reth::providers::Chain;
use signet_zenith::{Passage, Transactor, Zenith};
use std::collections::BTreeMap;

/// The host extraction contents for a block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DbExtractionResults {
    /// The zenith header for the block.
    pub header: Option<Zenith::BlockHeader>,
    /// The enters for the block.
    pub enters: Vec<Passage::Enter>,
    /// The transacts for the block.
    pub transacts: Vec<Transactor::Transact>,
    /// The enter tokens for the block.
    pub enter_tokens: Vec<Passage::EnterToken>,
}

/// Equivalent of [`Chain`] but also containing zenith headers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuChain {
    /// Inner chain of RU blocks.
    pub inner: Chain,
    /// Zenith headers for each block.
    pub ru_info: BTreeMap<BlockNumber, DbExtractionResults>,
}

impl RuChain {
    /// Get the length of the chain in blocks.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Check if the chain is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}
