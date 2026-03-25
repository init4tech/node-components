//! Parity `trace` namespace RPC endpoint implementations.

use crate::{
    eth::helpers::CfgFiller,
    trace::TraceError,
};
use alloy::{
    consensus::BlockHeader,
    primitives::{map::HashSet, B256},
    rpc::types::trace::parity::{
        LocalizedTransactionTrace, TraceResults,
        TraceResultsWithTransactionHash, TraceType,
    },
};
use signet_types::{MagicSig, constants::SignetSystemConstants};
use trevm::revm::{
    Database, DatabaseRef,
    database::{DBErrorMarker, State},
    primitives::hardfork::SpecId,
};

/// Shared localized tracing loop for Parity `trace_block` and
/// `trace_filter`.
///
/// Replays all transactions in a block (stopping at the first
/// magic-signature tx) and returns localized Parity traces.
#[allow(clippy::too_many_arguments)]
fn trace_block_localized<Db>(
    ctx_chain_id: u64,
    constants: SignetSystemConstants,
    spec_id: SpecId,
    header: &alloy::consensus::Header,
    block_hash: B256,
    txs: &[signet_storage_types::RecoveredTx],
    db: State<Db>,
) -> Result<Vec<LocalizedTransactionTrace>, TraceError>
where
    Db: Database + DatabaseRef,
    <Db as Database>::Error: DBErrorMarker,
    <Db as DatabaseRef>::Error: DBErrorMarker,
{
    use itertools::Itertools;

    let mut evm = signet_evm::signet_evm(db, constants);
    evm.set_spec_id(spec_id);
    let mut trevm = evm
        .fill_cfg(&CfgFiller(ctx_chain_id))
        .fill_block(header);

    let mut all_traces = Vec::new();
    let mut txns = txs.iter().enumerate().peekable();
    for (idx, tx) in txns
        .by_ref()
        .peeking_take_while(|(_, t)| {
            MagicSig::try_from_signature(t.signature()).is_none()
        })
    {
        let tx_info = alloy::rpc::types::TransactionInfo {
            hash: Some(*tx.tx_hash()),
            index: Some(idx as u64),
            block_hash: Some(block_hash),
            block_number: Some(header.number),
            base_fee: header.base_fee_per_gas(),
        };

        let t = trevm.fill_tx(tx);
        let (traces, next) = crate::debug::tracer::trace_parity_localized(
            t, tx_info,
        )
        .map_err(|e| TraceError::EvmHalt {
            reason: e.to_string(),
        })?;
        trevm = next;
        all_traces.extend(traces);
    }

    Ok(all_traces)
}

/// Shared replay tracing loop for Parity `trace_replayBlockTransactions`.
///
/// Replays all transactions and returns per-tx `TraceResults` with
/// the caller's `TraceType` selection.
#[allow(clippy::too_many_arguments)]
fn trace_block_replay<Db>(
    ctx_chain_id: u64,
    constants: SignetSystemConstants,
    spec_id: SpecId,
    header: &alloy::consensus::Header,
    _block_hash: B256,
    txs: &[signet_storage_types::RecoveredTx],
    db: State<Db>,
    trace_types: &HashSet<TraceType>,
) -> Result<Vec<TraceResultsWithTransactionHash>, TraceError>
where
    Db: Database + DatabaseRef,
    <Db as Database>::Error: DBErrorMarker,
    <Db as DatabaseRef>::Error: std::fmt::Debug + DBErrorMarker,
{
    use itertools::Itertools;

    let mut evm = signet_evm::signet_evm(db, constants);
    evm.set_spec_id(spec_id);
    let mut trevm = evm
        .fill_cfg(&CfgFiller(ctx_chain_id))
        .fill_block(header);

    let mut results = Vec::with_capacity(txs.len());
    let mut txns = txs.iter().enumerate().peekable();
    for (_idx, tx) in txns
        .by_ref()
        .peeking_take_while(|(_, t)| {
            MagicSig::try_from_signature(t.signature()).is_none()
        })
    {
        let t = trevm.fill_tx(tx);
        let (trace_res, next) = crate::debug::tracer::trace_parity_replay(
            t, trace_types,
        )
        .map_err(|e| TraceError::EvmHalt {
            reason: e.to_string(),
        })?;
        trevm = next;

        results.push(TraceResultsWithTransactionHash {
            full_trace: trace_res,
            transaction_hash: *tx.tx_hash(),
        });
    }

    Ok(results)
}
