//! Specifier enums for cold storage lookups.
//!
//! These types define how to locate data in cold storage, supporting
//! the standard Ethereum JSON-RPC lookup patterns.

use alloy::{
    primitives::{B256, BlockNumber},
    rpc::types::eth::BlockNumberOrTag,
};

/// Block tag for semantic block lookups.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockTag {
    /// The most recent block.
    Latest,
    /// The most recent finalized block.
    Finalized,
    /// The most recent safe block.
    Safe,
    /// The earliest/genesis block.
    Earliest,
}

/// Specifier for header lookups.
#[derive(Debug, Clone, Copy)]
pub enum HeaderSpecifier {
    /// Lookup by block number.
    Number(BlockNumber),
    /// Lookup by block hash.
    Hash(B256),
    /// Lookup by semantic tag.
    Tag(BlockTag),
}

impl From<BlockNumberOrTag> for HeaderSpecifier {
    fn from(value: BlockNumberOrTag) -> Self {
        match value {
            BlockNumberOrTag::Number(num) => Self::Number(num),
            BlockNumberOrTag::Latest => Self::Tag(BlockTag::Latest),
            BlockNumberOrTag::Finalized => Self::Tag(BlockTag::Finalized),
            BlockNumberOrTag::Safe => Self::Tag(BlockTag::Safe),
            BlockNumberOrTag::Earliest => Self::Tag(BlockTag::Earliest),
            BlockNumberOrTag::Pending => Self::Tag(BlockTag::Latest), // Treat pending as latest
        }
    }
}

impl From<BlockNumber> for HeaderSpecifier {
    fn from(number: BlockNumber) -> Self {
        Self::Number(number)
    }
}

impl From<B256> for HeaderSpecifier {
    fn from(hash: B256) -> Self {
        Self::Hash(hash)
    }
}

impl From<BlockTag> for HeaderSpecifier {
    fn from(tag: BlockTag) -> Self {
        Self::Tag(tag)
    }
}

/// Specifier for transaction lookups.
#[derive(Debug, Clone, Copy)]
pub enum TransactionSpecifier {
    /// Lookup by transaction hash.
    Hash(B256),
    /// Lookup by block number and transaction index within the block.
    BlockAndIndex {
        /// The block number.
        block: BlockNumber,
        /// The transaction index within the block.
        index: u64,
    },
    /// Lookup by block hash and transaction index within the block.
    BlockHashAndIndex {
        /// The block hash.
        block_hash: B256,
        /// The transaction index within the block.
        index: u64,
    },
}

impl From<B256> for TransactionSpecifier {
    fn from(hash: B256) -> Self {
        Self::Hash(hash)
    }
}

/// Specifier for receipt lookups.
#[derive(Debug, Clone, Copy)]
pub enum ReceiptSpecifier {
    /// Lookup by transaction hash.
    TxHash(B256),
    /// Lookup by block number and transaction index within the block.
    BlockAndIndex {
        /// The block number.
        block: BlockNumber,
        /// The transaction index within the block.
        index: u64,
    },
}

impl From<B256> for ReceiptSpecifier {
    fn from(tx_hash: B256) -> Self {
        Self::TxHash(tx_hash)
    }
}

/// Specifier for SignetEvents lookups.
#[derive(Debug, Clone, Copy)]
pub enum SignetEventsSpecifier {
    /// Lookup all events in a single block.
    Block(BlockNumber),
    /// Lookup all events in a range of blocks (inclusive).
    BlockRange {
        /// The start block number (inclusive).
        start: BlockNumber,
        /// The end block number (inclusive).
        end: BlockNumber,
    },
}

impl From<BlockNumber> for SignetEventsSpecifier {
    fn from(block: BlockNumber) -> Self {
        Self::Block(block)
    }
}

/// Specifier for ZenithHeader lookups.
#[derive(Debug, Clone, Copy)]
pub enum ZenithHeaderSpecifier {
    /// Lookup by block number.
    Number(BlockNumber),
    /// Lookup a range of blocks (inclusive).
    Range {
        /// The start block number (inclusive).
        start: BlockNumber,
        /// The end block number (inclusive).
        end: BlockNumber,
    },
}

impl From<BlockNumber> for ZenithHeaderSpecifier {
    fn from(number: BlockNumber) -> Self {
        Self::Number(number)
    }
}
