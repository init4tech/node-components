//! Parameter types for debug namespace RPC endpoints.

use alloy::{primitives::B256, rpc::types::trace::geth::GethDebugTracingOptions};

/// Params for `debug_traceBlockByNumber` and `debug_traceBlockByHash`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct TraceBlockParams<T>(
    pub(crate) T,
    #[serde(default)] pub(crate) Option<GethDebugTracingOptions>,
);

/// Params for `debug_traceTransaction`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct TraceTransactionParams(
    pub(crate) B256,
    #[serde(default)] pub(crate) Option<GethDebugTracingOptions>,
);
