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

    /// Walk exhaustion recovery requires a cached finalized block number,
    /// but none has been fetched yet.
    #[error("no cached finalized block number for exhaustion recovery")]
    NoFinalizedBlock,
}
