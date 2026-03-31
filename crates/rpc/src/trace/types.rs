//! Parameter types for the `trace` namespace.

use alloy::{
    eips::BlockId,
    primitives::{B256, Bytes, map::HashSet},
    rpc::types::{
        BlockNumberOrTag, BlockOverrides, TransactionRequest,
        state::StateOverride,
        trace::{filter::TraceFilter, parity::TraceType},
    },
};

/// Params for `trace_block`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct TraceBlockParams(pub(crate) BlockNumberOrTag);

/// Params for `trace_transaction`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct TraceTransactionParams(pub(crate) B256);

/// Params for `trace_replayBlockTransactions`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct ReplayBlockParams(pub(crate) BlockNumberOrTag, pub(crate) HashSet<TraceType>);

/// Params for `trace_replayTransaction`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct ReplayTransactionParams(pub(crate) B256, pub(crate) HashSet<TraceType>);

/// Params for `trace_call`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct TraceCallParams(
    pub(crate) TransactionRequest,
    pub(crate) HashSet<TraceType>,
    #[serde(default)] pub(crate) Option<BlockId>,
    #[serde(default)] pub(crate) Option<StateOverride>,
    #[serde(default)] pub(crate) Option<Box<BlockOverrides>>,
);

/// Params for `trace_callMany`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct TraceCallManyParams(
    pub(crate) Vec<(TransactionRequest, HashSet<TraceType>)>,
    #[serde(default)] pub(crate) Option<BlockId>,
);

/// Params for `trace_rawTransaction`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct TraceRawTransactionParams(
    pub(crate) Bytes,
    pub(crate) HashSet<TraceType>,
    #[serde(default)] pub(crate) Option<BlockId>,
);

/// Params for `trace_get`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct TraceGetParams(pub(crate) B256, pub(crate) Vec<usize>);

/// Params for `trace_filter`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct TraceFilterParams(pub(crate) TraceFilter);
