//! Shim and utilities for signet-sdk to reth conversions.

use alloy::consensus::Block;
use signet_extract::HasTxns;
use signet_types::primitives::TransactionSigned;

/// A type alias for Reth's recovered block with a signed transaction.
type RethRecovered = reth::primitives::RecoveredBlock<Block<TransactionSigned>>;

/// A shim for Reth's [`reth::primitives::RecoveredBlock`].
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

impl alloy::consensus::BlockHeader for RecoveredBlockShim {
    fn parent_hash(&self) -> alloy::primitives::B256 {
        self.block.parent_hash()
    }

    fn ommers_hash(&self) -> alloy::primitives::B256 {
        self.block.ommers_hash()
    }

    fn beneficiary(&self) -> alloy::primitives::Address {
        self.block.beneficiary()
    }

    fn base_fee_per_gas(&self) -> Option<u64> {
        self.block.base_fee_per_gas()
    }

    fn blob_fee(&self, blob_params: alloy::eips::eip7840::BlobParams) -> Option<u128> {
        self.block.blob_fee(blob_params)
    }

    fn blob_gas_used(&self) -> Option<u64> {
        self.block.blob_gas_used()
    }

    fn difficulty(&self) -> alloy::primitives::U256 {
        self.block.difficulty()
    }

    fn exceeds_allowed_future_timestamp(&self, present_timestamp: u64) -> bool {
        self.block.exceeds_allowed_future_timestamp(present_timestamp)
    }

    fn excess_blob_gas(&self) -> Option<u64> {
        self.block.excess_blob_gas()
    }

    fn extra_data(&self) -> &alloy::primitives::Bytes {
        self.block.extra_data()
    }

    fn parent_beacon_block_root(&self) -> Option<alloy::primitives::B256> {
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

    fn logs_bloom(&self) -> alloy::primitives::Bloom {
        self.block.logs_bloom()
    }

    fn maybe_next_block_blob_fee(
        &self,
        blob_params: Option<alloy::eips::eip7840::BlobParams>,
    ) -> Option<u128> {
        self.block.maybe_next_block_blob_fee(blob_params)
    }

    fn maybe_next_block_excess_blob_gas(
        &self,
        blob_params: Option<alloy::eips::eip7840::BlobParams>,
    ) -> Option<u64> {
        self.block.maybe_next_block_excess_blob_gas(blob_params)
    }

    fn mix_hash(&self) -> Option<alloy::primitives::B256> {
        self.block.mix_hash()
    }

    fn next_block_base_fee(&self, base_fee_params: reth_chainspec::BaseFeeParams) -> Option<u64> {
        self.block.next_block_base_fee(base_fee_params)
    }

    fn next_block_blob_fee(&self, blob_params: alloy::eips::eip7840::BlobParams) -> Option<u128> {
        self.block.next_block_blob_fee(blob_params)
    }

    fn next_block_excess_blob_gas(
        &self,
        blob_params: alloy::eips::eip7840::BlobParams,
    ) -> Option<u64> {
        self.block.next_block_excess_blob_gas(blob_params)
    }

    fn nonce(&self) -> Option<alloy::primitives::B64> {
        self.block.nonce()
    }

    fn number(&self) -> alloy::primitives::BlockNumber {
        self.block.number()
    }

    fn parent_num_hash(&self) -> alloy::eips::BlockNumHash {
        self.block.parent_num_hash()
    }

    fn receipts_root(&self) -> alloy::primitives::B256 {
        self.block.receipts_root()
    }

    fn requests_hash(&self) -> Option<alloy::primitives::B256> {
        self.block.requests_hash()
    }

    fn state_root(&self) -> alloy::primitives::B256 {
        self.block.state_root()
    }

    fn timestamp(&self) -> u64 {
        self.block.timestamp()
    }

    fn transactions_root(&self) -> alloy::primitives::B256 {
        self.block.transactions_root()
    }

    fn withdrawals_root(&self) -> Option<alloy::primitives::B256> {
        self.block.withdrawals_root()
    }
}
