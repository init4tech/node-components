//! Parity `trace` namespace RPC endpoint implementations.

use crate::{
    config::{EvmBlockContext, StorageRpcCtx},
    eth::helpers::{CfgFiller, await_handler},
    trace::{
        TraceError,
        types::{
            ReplayBlockParams, ReplayTransactionParams, TraceBlockParams, TraceCallManyParams,
            TraceCallParams, TraceFilterParams, TraceGetParams, TraceRawTransactionParams,
            TraceTransactionParams,
        },
    },
};
use ajj::HandlerCtx;
use alloy::{
    consensus::BlockHeader,
    eips::BlockId,
    primitives::{B256, map::HashSet},
    rpc::types::trace::parity::{
        LocalizedTransactionTrace, TraceResults, TraceResultsWithTransactionHash, TraceType,
    },
};
use signet_hot::{HotKv, model::HotKvRead};
use signet_types::{MagicSig, constants::SignetSystemConstants};
use tracing::Instrument;
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
    let mut trevm = evm.fill_cfg(&CfgFiller(ctx_chain_id)).fill_block(header);

    let mut all_traces = Vec::new();
    let mut txns = txs.iter().enumerate().peekable();
    for (idx, tx) in txns
        .by_ref()
        .peeking_take_while(|(_, t)| MagicSig::try_from_signature(t.signature()).is_none())
    {
        let tx_info = alloy::rpc::types::TransactionInfo {
            hash: Some(*tx.tx_hash()),
            index: Some(idx as u64),
            block_hash: Some(block_hash),
            block_number: Some(header.number),
            base_fee: header.base_fee_per_gas(),
        };

        let t = trevm.fill_tx(tx);
        let (traces, next) = crate::debug::tracer::trace_parity_localized(t, tx_info)
            .map_err(|e| TraceError::EvmHalt { reason: e.to_string() })?;
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
    let mut trevm = evm.fill_cfg(&CfgFiller(ctx_chain_id)).fill_block(header);

    let mut results = Vec::with_capacity(txs.len());
    let mut txns = txs.iter().enumerate().peekable();
    for (_idx, tx) in txns
        .by_ref()
        .peeking_take_while(|(_, t)| MagicSig::try_from_signature(t.signature()).is_none())
    {
        let t = trevm.fill_tx(tx);
        let (trace_res, next) = crate::debug::tracer::trace_parity_replay(t, trace_types)
            .map_err(|e| TraceError::EvmHalt { reason: e.to_string() })?;
        trevm = next;

        results.push(TraceResultsWithTransactionHash {
            full_trace: trace_res,
            transaction_hash: *tx.tx_hash(),
        });
    }

    Ok(results)
}

/// `trace_block` — return Parity traces for all transactions in a block.
pub(super) async fn trace_block<H>(
    hctx: HandlerCtx,
    TraceBlockParams(id): TraceBlockParams,
    ctx: StorageRpcCtx<H>,
) -> Result<Option<Vec<LocalizedTransactionTrace>>, TraceError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let _permit = ctx.acquire_tracing_permit().await;
    let id = BlockId::Number(id);
    let span = tracing::debug_span!("trace_block", ?id);

    let fut = async move {
        let cold = ctx.cold();
        let block_num = ctx.resolve_block_id(id).map_err(|e| {
            tracing::warn!(error = %e, ?id, "block resolution failed");
            TraceError::Resolve(e)
        })?;

        let sealed = ctx.resolve_header(BlockId::Number(block_num.into())).map_err(|e| {
            tracing::warn!(error = %e, block_num, "header resolution failed");
            TraceError::Resolve(e)
        })?;

        let Some(sealed) = sealed else {
            return Ok(None);
        };

        let block_hash = sealed.hash();
        let header = sealed.into_inner();

        let txs = cold.get_transactions_in_block(block_num).await.map_err(TraceError::from)?;

        let db =
            ctx.revm_state_at_height(header.number.saturating_sub(1)).map_err(TraceError::from)?;

        let spec_id = ctx.spec_id_for_header(&header);
        let traces = trace_block_localized(
            ctx.chain_id(),
            ctx.constants().clone(),
            spec_id,
            &header,
            block_hash,
            &txs,
            db,
        )?;

        Ok(Some(traces))
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        TraceError::EvmHalt { reason: "task panicked or cancelled".into() }
    )
}

/// `trace_transaction` — return Parity traces for a single transaction.
pub(super) async fn trace_transaction<H>(
    hctx: HandlerCtx,
    TraceTransactionParams(tx_hash): TraceTransactionParams,
    ctx: StorageRpcCtx<H>,
) -> Result<Option<Vec<LocalizedTransactionTrace>>, TraceError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let _permit = ctx.acquire_tracing_permit().await;
    let span = tracing::debug_span!("trace_transaction", %tx_hash);

    let fut = async move {
        let cold = ctx.cold();

        let confirmed = cold.get_tx_by_hash(tx_hash).await.map_err(TraceError::from)?;

        let Some(confirmed) = confirmed else {
            return Ok(None);
        };
        let (_tx, meta) = confirmed.into_parts();
        let block_num = meta.block_number();
        let block_hash = meta.block_hash();

        let block_id = BlockId::Number(block_num.into());
        let sealed = ctx.resolve_header(block_id).map_err(|e| {
            tracing::warn!(error = %e, block_num, "header resolution failed");
            TraceError::Resolve(e)
        })?;
        let header = sealed.ok_or(TraceError::BlockNotFound(block_id))?.into_inner();

        let txs = cold.get_transactions_in_block(block_num).await.map_err(TraceError::from)?;

        let db = ctx.revm_state_at_height(block_num.saturating_sub(1)).map_err(TraceError::from)?;

        let spec_id = ctx.spec_id_for_header(&header);
        let mut evm = signet_evm::signet_evm(db, ctx.constants().clone());
        evm.set_spec_id(spec_id);
        let mut trevm = evm.fill_cfg(&CfgFiller(ctx.chain_id())).fill_block(&header);

        // Replay preceding txs without tracing.
        use itertools::Itertools;
        let mut txns = txs.iter().enumerate().peekable();
        for (_idx, tx) in txns.by_ref().peeking_take_while(|(_, t)| t.tx_hash() != &tx_hash) {
            if MagicSig::try_from_signature(tx.signature()).is_some() {
                return Ok(None);
            }
            trevm = trevm
                .run_tx(tx)
                .map_err(|e| TraceError::EvmHalt { reason: e.into_error().to_string() })?
                .accept_state();
        }

        let Some((index, tx)) = txns.next() else {
            return Ok(None);
        };

        let tx_info = alloy::rpc::types::TransactionInfo {
            hash: Some(*tx.tx_hash()),
            index: Some(index as u64),
            block_hash: Some(block_hash),
            block_number: Some(header.number),
            base_fee: header.base_fee_per_gas(),
        };

        let trevm = trevm.fill_tx(tx);
        let (traces, _) = crate::debug::tracer::trace_parity_localized(trevm, tx_info)
            .map_err(|e| TraceError::EvmHalt { reason: e.to_string() })?;

        Ok(Some(traces))
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        TraceError::EvmHalt { reason: "task panicked or cancelled".into() }
    )
}

/// `trace_replayBlockTransactions` — replay all block txs with trace type selection.
pub(super) async fn replay_block_transactions<H>(
    hctx: HandlerCtx,
    ReplayBlockParams(id, trace_types): ReplayBlockParams,
    ctx: StorageRpcCtx<H>,
) -> Result<Option<Vec<TraceResultsWithTransactionHash>>, TraceError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let _permit = ctx.acquire_tracing_permit().await;
    let id = BlockId::Number(id);
    let span = tracing::debug_span!("trace_replayBlockTransactions", ?id);

    let fut = async move {
        let cold = ctx.cold();
        let block_num = ctx.resolve_block_id(id).map_err(|e| {
            tracing::warn!(error = %e, ?id, "block resolution failed");
            TraceError::Resolve(e)
        })?;

        let sealed =
            ctx.resolve_header(BlockId::Number(block_num.into())).map_err(TraceError::Resolve)?;

        let Some(sealed) = sealed else {
            return Ok(None);
        };

        let block_hash = sealed.hash();
        let header = sealed.into_inner();

        let txs = cold.get_transactions_in_block(block_num).await.map_err(TraceError::from)?;

        let db =
            ctx.revm_state_at_height(header.number.saturating_sub(1)).map_err(TraceError::from)?;

        let spec_id = ctx.spec_id_for_header(&header);
        let results = trace_block_replay(
            ctx.chain_id(),
            ctx.constants().clone(),
            spec_id,
            &header,
            block_hash,
            &txs,
            db,
            &trace_types,
        )?;

        Ok(Some(results))
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        TraceError::EvmHalt { reason: "task panicked or cancelled".into() }
    )
}

/// `trace_replayTransaction` — replay a single tx with trace type selection.
pub(super) async fn replay_transaction<H>(
    hctx: HandlerCtx,
    ReplayTransactionParams(tx_hash, trace_types): ReplayTransactionParams,
    ctx: StorageRpcCtx<H>,
) -> Result<TraceResults, TraceError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let _permit = ctx.acquire_tracing_permit().await;
    let span = tracing::debug_span!("trace_replayTransaction", %tx_hash);

    let fut = async move {
        let cold = ctx.cold();
        let confirmed = cold
            .get_tx_by_hash(tx_hash)
            .await
            .map_err(TraceError::from)?
            .ok_or(TraceError::TransactionNotFound(tx_hash))?;

        let (_tx, meta) = confirmed.into_parts();
        let block_num = meta.block_number();

        let block_id = BlockId::Number(block_num.into());
        let sealed = ctx.resolve_header(block_id).map_err(TraceError::Resolve)?;
        let header = sealed.ok_or(TraceError::BlockNotFound(block_id))?.into_inner();

        let txs = cold.get_transactions_in_block(block_num).await.map_err(TraceError::from)?;

        let db = ctx.revm_state_at_height(block_num.saturating_sub(1)).map_err(TraceError::from)?;

        let spec_id = ctx.spec_id_for_header(&header);
        let mut evm = signet_evm::signet_evm(db, ctx.constants().clone());
        evm.set_spec_id(spec_id);
        let mut trevm = evm.fill_cfg(&CfgFiller(ctx.chain_id())).fill_block(&header);

        // Replay preceding txs.
        use itertools::Itertools;
        let mut txns = txs.iter().enumerate().peekable();
        for (_idx, tx) in txns.by_ref().peeking_take_while(|(_, t)| t.tx_hash() != &tx_hash) {
            if MagicSig::try_from_signature(tx.signature()).is_some() {
                return Err(TraceError::TransactionNotFound(tx_hash));
            }
            trevm = trevm
                .run_tx(tx)
                .map_err(|e| TraceError::EvmHalt { reason: e.into_error().to_string() })?
                .accept_state();
        }

        let (_index, tx) = txns.next().ok_or(TraceError::TransactionNotFound(tx_hash))?;

        let trevm = trevm.fill_tx(tx);
        let (results, _) = crate::debug::tracer::trace_parity_replay(trevm, &trace_types)
            .map_err(|e| TraceError::EvmHalt { reason: e.to_string() })?;

        Ok(results)
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        TraceError::EvmHalt { reason: "task panicked or cancelled".into() }
    )
}

/// `trace_call` — trace a call with Parity output and state overrides.
pub(super) async fn trace_call<H>(
    hctx: HandlerCtx,
    TraceCallParams(request, trace_types, block_id, state_overrides, block_overrides): TraceCallParams,
    ctx: StorageRpcCtx<H>,
) -> Result<TraceResults, TraceError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let _permit = ctx.acquire_tracing_permit().await;
    let id = block_id.unwrap_or(BlockId::latest());
    let span = tracing::debug_span!("trace_call", ?id);

    let fut = async move {
        let EvmBlockContext { header, db, spec_id } =
            ctx.resolve_evm_block(id).map_err(|e| match e {
                crate::eth::EthError::BlockNotFound(id) => TraceError::BlockNotFound(id),
                other => TraceError::EvmHalt { reason: other.to_string() },
            })?;

        let mut evm = signet_evm::signet_evm(db, ctx.constants().clone());
        evm.set_spec_id(spec_id);
        let trevm = evm.fill_cfg(&CfgFiller(ctx.chain_id())).fill_block(&header);

        // Apply state and block overrides (matching reth trace_call).
        let trevm = trevm
            .maybe_apply_state_overrides(state_overrides.as_ref())
            .map_err(|e| TraceError::EvmHalt { reason: e.to_string() })?
            .maybe_apply_block_overrides(block_overrides.as_deref())
            .fill_tx(&request);

        let (results, _) = crate::debug::tracer::trace_parity_replay(trevm, &trace_types)
            .map_err(|e| TraceError::EvmHalt { reason: e.to_string() })?;

        Ok(results)
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        TraceError::EvmHalt { reason: "task panicked or cancelled".into() }
    )
}

/// `trace_callMany` — trace sequential calls with accumulated state.
///
/// Each call sees state changes from prior calls. Per-call trace
/// types. Defaults to `BlockId::pending()` (matching reth).
pub(super) async fn trace_call_many<H>(
    hctx: HandlerCtx,
    TraceCallManyParams(calls, block_id): TraceCallManyParams,
    ctx: StorageRpcCtx<H>,
) -> Result<Vec<TraceResults>, TraceError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let _permit = ctx.acquire_tracing_permit().await;
    let id = block_id.unwrap_or(BlockId::pending());
    let span = tracing::debug_span!("trace_callMany", ?id, count = calls.len());

    let fut = async move {
        let EvmBlockContext { header, db, spec_id } =
            ctx.resolve_evm_block(id).map_err(|e| match e {
                crate::eth::EthError::BlockNotFound(id) => TraceError::BlockNotFound(id),
                other => TraceError::EvmHalt { reason: other.to_string() },
            })?;

        let mut evm = signet_evm::signet_evm(db, ctx.constants().clone());
        evm.set_spec_id(spec_id);
        let mut trevm = evm.fill_cfg(&CfgFiller(ctx.chain_id())).fill_block(&header);

        let mut results = Vec::with_capacity(calls.len());

        for (request, trace_types) in calls {
            let filled = trevm.fill_tx(&request);
            let (trace_res, next) = crate::debug::tracer::trace_parity_replay(filled, &trace_types)
                .map_err(|e| TraceError::EvmHalt { reason: e.to_string() })?;

            results.push(trace_res);
            trevm = next;
        }

        Ok(results)
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        TraceError::EvmHalt { reason: "task panicked or cancelled".into() }
    )
}

/// `trace_rawTransaction` — trace a transaction from raw RLP bytes.
pub(super) async fn trace_raw_transaction<H>(
    hctx: HandlerCtx,
    TraceRawTransactionParams(rlp_bytes, trace_types, block_id): TraceRawTransactionParams,
    ctx: StorageRpcCtx<H>,
) -> Result<TraceResults, TraceError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let _permit = ctx.acquire_tracing_permit().await;
    let id = block_id.unwrap_or(BlockId::latest());
    let span = tracing::debug_span!("trace_rawTransaction", ?id);

    let fut = async move {
        use alloy::consensus::transaction::SignerRecoverable;

        // Decode and recover sender.
        let tx: signet_storage_types::TransactionSigned =
            alloy::rlp::Decodable::decode(&mut rlp_bytes.as_ref())
                .map_err(|e| TraceError::RlpDecode(e.to_string()))?;
        let recovered = tx.try_into_recovered().map_err(|_| TraceError::SenderRecovery)?;

        let EvmBlockContext { header, db, spec_id } =
            ctx.resolve_evm_block(id).map_err(|e| match e {
                crate::eth::EthError::BlockNotFound(id) => TraceError::BlockNotFound(id),
                other => TraceError::EvmHalt { reason: other.to_string() },
            })?;

        let mut evm = signet_evm::signet_evm(db, ctx.constants().clone());
        evm.set_spec_id(spec_id);
        let trevm =
            evm.fill_cfg(&CfgFiller(ctx.chain_id())).fill_block(&header).fill_tx(&recovered);

        let (results, _) = crate::debug::tracer::trace_parity_replay(trevm, &trace_types)
            .map_err(|e| TraceError::EvmHalt { reason: e.to_string() })?;

        Ok(results)
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        TraceError::EvmHalt { reason: "task panicked or cancelled".into() }
    )
}

/// `trace_get` — get a specific trace by tx hash and index.
///
/// Returns `None` if `indices.len() != 1` (Erigon compatibility,
/// matching reth).
pub(super) async fn trace_get<H>(
    hctx: HandlerCtx,
    TraceGetParams(tx_hash, indices): TraceGetParams,
    ctx: StorageRpcCtx<H>,
) -> Result<Option<LocalizedTransactionTrace>, TraceError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    if indices.len() != 1 {
        return Ok(None);
    }

    let traces = trace_transaction(hctx, TraceTransactionParams(tx_hash), ctx).await?;

    Ok(traces.and_then(|t| t.into_iter().nth(indices[0])))
}

/// `trace_filter` — filter traces across a block range.
///
/// Brute-force replay with configurable block range limit (default
/// 100 blocks). Matches reth's approach.
pub(super) async fn trace_filter<H>(
    hctx: HandlerCtx,
    TraceFilterParams(filter): TraceFilterParams,
    ctx: StorageRpcCtx<H>,
) -> Result<Vec<LocalizedTransactionTrace>, TraceError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let _permit = ctx.acquire_tracing_permit().await;
    let span = tracing::debug_span!("trace_filter");

    let fut = async move {
        let latest = ctx.tags().latest();
        let start = filter.from_block.unwrap_or(0);
        let end = filter.to_block.unwrap_or(latest);

        if start > latest || end > latest {
            return Err(TraceError::BlockNotFound(BlockId::latest()));
        }
        if start > end {
            return Err(TraceError::EvmHalt {
                reason: "fromBlock cannot be greater than toBlock".into(),
            });
        }

        let max = ctx.config().max_trace_filter_blocks;
        let distance = end.saturating_sub(start);
        if distance > max {
            return Err(TraceError::BlockRangeExceeded { requested: distance, max });
        }

        let matcher = filter.matcher();
        let mut all_traces = Vec::new();

        for block_num in start..=end {
            let cold = ctx.cold();
            let block_id = BlockId::Number(block_num.into());

            let sealed = ctx.resolve_header(block_id).map_err(TraceError::Resolve)?;

            let Some(sealed) = sealed else {
                continue;
            };

            let block_hash = sealed.hash();
            let header = sealed.into_inner();

            let txs = cold.get_transactions_in_block(block_num).await.map_err(TraceError::from)?;

            let db = ctx
                .revm_state_at_height(header.number.saturating_sub(1))
                .map_err(TraceError::from)?;

            let spec_id = ctx.spec_id_for_header(&header);
            let mut traces = trace_block_localized(
                ctx.chain_id(),
                ctx.constants().clone(),
                spec_id,
                &header,
                block_hash,
                &txs,
                db,
            )?;

            // Apply filter matcher.
            traces.retain(|t| matcher.matches(&t.trace));
            all_traces.extend(traces);
        }

        // Apply pagination: skip `after`, limit `count`.
        if let Some(after) = filter.after {
            let after = after as usize;
            if after >= all_traces.len() {
                return Ok(vec![]);
            }
            all_traces.drain(..after);
        }
        if let Some(count) = filter.count {
            all_traces.truncate(count as usize);
        }

        Ok(all_traces)
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        TraceError::EvmHalt { reason: "task panicked or cancelled".into() }
    )
}
