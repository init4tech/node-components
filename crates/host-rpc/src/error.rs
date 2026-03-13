use alloy::transports::{RpcError, TransportErrorKind};

/// Errors from the RPC host notifier.
#[derive(Debug, thiserror::Error)]
pub enum RpcHostError {
    /// The WebSocket subscription was dropped unexpectedly.
    #[error("subscription closed")]
    SubscriptionClosed,

    /// An RPC call failed.
    #[error("rpc error: {0}")]
    Rpc(#[from] RpcError<TransportErrorKind>),

    /// The RPC node returned no block for the requested number.
    #[error("missing block {0}")]
    MissingBlock(u64),

    /// Reorg deeper than the block buffer.
    #[error("reorg depth {depth} exceeds buffer capacity {capacity}")]
    ReorgTooDeep {
        /// The detected reorg depth.
        depth: u64,
        /// The configured buffer capacity.
        capacity: usize,
    },
}
