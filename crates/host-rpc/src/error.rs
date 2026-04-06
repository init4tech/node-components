use alloy::{
    eips::BlockNumberOrTag,
    primitives::B256,
    transports::{RpcError, TransportErrorKind},
};

/// Errors from the RPC host notifier.
#[derive(Debug, thiserror::Error)]
pub enum RpcHostError {
    /// An RPC call failed.
    #[error("rpc error: {0}")]
    Rpc(#[from] RpcError<TransportErrorKind>),

    /// The RPC node returned no block for the requested hash.
    #[error("missing block with hash {0}")]
    MissingBlockByHash(B256),

    /// The RPC node returned no block for the requested number or tag.
    #[error("missing block {0}")]
    MissingBlock(BlockNumberOrTag),

    /// The first block of a backfill batch does not chain to the last
    /// emitted block (parent-hash mismatch). A reorg occurred during the
    /// gap between exhaustion and backfill.
    #[error("backfill continuity break: parent hash mismatch after exhaustion recovery")]
    BackfillContinuityBreak,
}
