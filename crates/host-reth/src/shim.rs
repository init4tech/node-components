//! Shim and utilities for signet-sdk to reth conversions.

use alloy::{
    consensus::{Block, BlockHeader},
    eips::{BlockNumHash, eip1559::BaseFeeParams, eip7840::BlobParams},
    primitives::{Address, B64, B256, BlockNumber, Bloom, Bytes, U256},
};
use reth::primitives::RecoveredBlock;
use signet_extract::HasTxns;
use signet_types::primitives::TransactionSigned;

/// A type alias for Reth's recovered block with a signed transaction.
type RethRecovered = RecoveredBlock<Block<TransactionSigned>>;

/// A shim for Reth's [`RecoveredBlock`].
///
/// This transparent wrapper forwards [`BlockHeader`] and [`HasTxns`]
/// implementations to the inner reth block, bridging the reth and signet type
/// systems.
#[derive(Debug)]
#[repr(transparent)]
pub struct RecoveredBlockShim {
    /// The underlying Reth block.
    pub block: RethRecovered,
}

impl From<RethRecovered> for RecoveredBlockShim {
    fn from(block: RethRecovered) -> Self {
        Self { block }
    }
}

impl HasTxns for RecoveredBlockShim {
    fn transactions(
        &self,
    ) -> impl ExactSizeIterator<Item = &signet_types::primitives::TransactionSigned> {
        self.block.sealed_block().body().transactions.iter()
    }
}

impl BlockHeader for RecoveredBlockShim {
    fn parent_hash(&self) -> B256 {
        self.block.parent_hash()
    }

    fn ommers_hash(&self) -> B256 {
        self.block.ommers_hash()
    }

    fn beneficiary(&self) -> Address {
        self.block.beneficiary()
    }

    fn base_fee_per_gas(&self) -> Option<u64> {
        self.block.base_fee_per_gas()
    }

    fn blob_fee(&self, blob_params: BlobParams) -> Option<u128> {
        self.block.blob_fee(blob_params)
    }

    fn blob_gas_used(&self) -> Option<u64> {
        self.block.blob_gas_used()
    }

    fn difficulty(&self) -> U256 {
        self.block.difficulty()
    }

    fn exceeds_allowed_future_timestamp(&self, present_timestamp: u64) -> bool {
        self.block.exceeds_allowed_future_timestamp(present_timestamp)
    }

    fn excess_blob_gas(&self) -> Option<u64> {
        self.block.excess_blob_gas()
    }

    fn extra_data(&self) -> &Bytes {
        self.block.extra_data()
    }

    fn parent_beacon_block_root(&self) -> Option<B256> {
        self.block.parent_beacon_block_root()
    }

    fn gas_limit(&self) -> u64 {
        self.block.gas_limit()
    }

    fn gas_used(&self) -> u64 {
        self.block.gas_used()
    }

    fn is_empty(&self) -> bool {
        self.block.is_empty()
    }

    fn is_nonce_zero(&self) -> bool {
        self.block.is_nonce_zero()
    }

    fn is_zero_difficulty(&self) -> bool {
        self.block.is_zero_difficulty()
    }

    fn logs_bloom(&self) -> Bloom {
        self.block.logs_bloom()
    }

    fn maybe_next_block_blob_fee(&self, blob_params: Option<BlobParams>) -> Option<u128> {
        self.block.maybe_next_block_blob_fee(blob_params)
    }

    fn maybe_next_block_excess_blob_gas(&self, blob_params: Option<BlobParams>) -> Option<u64> {
        self.block.maybe_next_block_excess_blob_gas(blob_params)
    }

    fn mix_hash(&self) -> Option<B256> {
        self.block.mix_hash()
    }

    fn next_block_base_fee(&self, base_fee_params: BaseFeeParams) -> Option<u64> {
        self.block.next_block_base_fee(base_fee_params)
    }

    fn next_block_blob_fee(&self, blob_params: BlobParams) -> Option<u128> {
        self.block.next_block_blob_fee(blob_params)
    }

    fn next_block_excess_blob_gas(&self, blob_params: BlobParams) -> Option<u64> {
        self.block.next_block_excess_blob_gas(blob_params)
    }

    fn nonce(&self) -> Option<B64> {
        self.block.nonce()
    }

    fn number(&self) -> BlockNumber {
        self.block.number()
    }

    fn parent_num_hash(&self) -> BlockNumHash {
        self.block.parent_num_hash()
    }

    fn receipts_root(&self) -> B256 {
        self.block.receipts_root()
    }

    fn requests_hash(&self) -> Option<B256> {
        self.block.requests_hash()
    }

    fn state_root(&self) -> B256 {
        self.block.state_root()
    }

    fn timestamp(&self) -> u64 {
        self.block.timestamp()
    }

    fn transactions_root(&self) -> B256 {
        self.block.transactions_root()
    }

    fn withdrawals_root(&self) -> Option<B256> {
        self.block.withdrawals_root()
    }
}
