//! ETH namespace RPC endpoint implementations.

use crate::{
    ctx::{EvmBlockContext, StorageRpcCtx},
    eth::helpers::{
        AddrWithBlock, BlockParams, BlockRangeInclusiveIter, CfgFiller, StorageAtArgs, TxParams,
        await_handler, build_receipt, build_receipt_from_parts, build_rpc_transaction,
        normalize_gas_stateless, response_tri,
    },
    resolve::{resolve_block_id, resolve_block_number_or_tag},
};
use ajj::{HandlerCtx, ResponsePayload};
use alloy::{
    consensus::{BlockHeader, TxReceipt},
    eips::{
        BlockId,
        eip2718::{Decodable2718, Encodable2718},
    },
    primitives::{B256, U64, U256},
    rpc::types::{Block, BlockTransactions, Filter, FilteredParams, Log},
};
use signet_cold::{HeaderSpecifier, ReceiptSpecifier};
use signet_hot::model::HotKvRead;
use signet_hot::{HistoryRead, HotKv, db::HotDbRead};
use std::borrow::Cow;
use tracing::{Instrument, debug, trace_span};
use trevm::{EstimationResult, revm::database::DBErrorMarker};

use super::error::CallErrorData;

// ---------------------------------------------------------------------------
// Not Supported
// ---------------------------------------------------------------------------

pub(crate) async fn not_supported() -> ResponsePayload<(), ()> {
    ResponsePayload::internal_error_message(Cow::Borrowed("Method not supported."))
}

// ---------------------------------------------------------------------------
// Simple Queries
// ---------------------------------------------------------------------------

pub(crate) async fn block_number<H: HotKv>(ctx: StorageRpcCtx<H>) -> Result<U64, String> {
    Ok(U64::from(ctx.tags().latest()))
}

pub(crate) async fn chain_id<H: HotKv>(ctx: StorageRpcCtx<H>) -> Result<U64, ()> {
    Ok(U64::from(ctx.chain_id()))
}

// ---------------------------------------------------------------------------
// Block Queries
// ---------------------------------------------------------------------------

pub(crate) async fn block<T, H>(
    hctx: HandlerCtx,
    BlockParams(t, full): BlockParams<T>,
    ctx: StorageRpcCtx<H>,
) -> Result<Option<Block>, String>
where
    T: Into<BlockId>,
    H: HotKv + Send + Sync + 'static,
    H::RoTx: Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let id = t.into();
    let full = full.unwrap_or(false);

    let task = async move {
        let cold = ctx.cold();
        let block_num = resolve_block_id(id, ctx.tags(), &cold).await.map_err(|e| e.to_string())?;

        let (header, txs) = tokio::try_join!(
            cold.get_header_by_number(block_num),
            cold.get_transactions_in_block(block_num),
        )
        .map_err(|e| e.to_string())?;

        let Some(header) = header else {
            return Ok(None);
        };

        let block_hash = header.hash_slow();

        let transactions = if full {
            let base_fee = header.base_fee_per_gas();
            let rpc_txs: Vec<_> = txs
                .into_iter()
                .enumerate()
                .map(|(i, tx)| {
                    let meta = signet_storage_types::ConfirmationMeta::new(
                        block_num, block_hash, i as u64,
                    );
                    build_rpc_transaction(tx, &meta, base_fee).map_err(|e| e.to_string())
                })
                .collect::<Result<_, _>>()?;
            BlockTransactions::Full(rpc_txs)
        } else {
            let hashes: Vec<B256> = txs.iter().map(|tx| *tx.tx_hash()).collect();
            BlockTransactions::Hashes(hashes)
        };

        Ok(Some(Block {
            header: alloy::rpc::types::Header::new(header),
            transactions,
            uncles: vec![],
            withdrawals: None,
        }))
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

pub(crate) async fn block_tx_count<T, H>(
    hctx: HandlerCtx,
    (t,): (T,),
    ctx: StorageRpcCtx<H>,
) -> Result<Option<U64>, String>
where
    T: Into<BlockId>,
    H: HotKv + Send + Sync + 'static,
    H::RoTx: Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let id = t.into();

    let task = async move {
        let cold = ctx.cold();
        let block_num = resolve_block_id(id, ctx.tags(), &cold).await.map_err(|e| e.to_string())?;

        cold.get_transaction_count(block_num)
            .await
            .map(|c| Some(U64::from(c)))
            .map_err(|e| e.to_string())
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

pub(crate) async fn block_receipts<H>(
    hctx: HandlerCtx,
    (id,): (BlockId,),
    ctx: StorageRpcCtx<H>,
) -> Result<Option<Vec<alloy::rpc::types::TransactionReceipt>>, String>
where
    H: HotKv + Send + Sync + 'static,
    H::RoTx: Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let task = async move {
        let cold = ctx.cold();
        let block_num = resolve_block_id(id, ctx.tags(), &cold).await.map_err(|e| e.to_string())?;

        let (header, txs, receipts) = tokio::try_join!(
            cold.get_header_by_number(block_num),
            cold.get_transactions_in_block(block_num),
            cold.get_receipts_in_block(block_num),
        )
        .map_err(|e| e.to_string())?;

        let Some(header) = header else {
            return Ok(None);
        };

        let block_hash = header.hash_slow();
        let mut log_index: u64 = 0;

        let rpc_receipts = txs
            .into_iter()
            .zip(receipts.iter())
            .enumerate()
            .map(|(idx, (tx, receipt))| {
                let prev_cumulative = idx
                    .checked_sub(1)
                    .and_then(|i| receipts.get(i))
                    .map(|r| r.inner.cumulative_gas_used())
                    .unwrap_or_default();

                let gas_used = receipt.inner.cumulative_gas_used() - prev_cumulative;
                let offset = log_index;
                log_index += receipt.inner.logs().len() as u64;

                build_receipt_from_parts(
                    tx,
                    &header,
                    block_hash,
                    idx as u64,
                    receipt.clone(),
                    gas_used,
                    offset,
                )
                .map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Some(rpc_receipts))
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

pub(crate) async fn header_by<T, H>(
    hctx: HandlerCtx,
    (t,): (T,),
    ctx: StorageRpcCtx<H>,
) -> Result<Option<alloy::rpc::types::Header>, String>
where
    T: Into<BlockId>,
    H: HotKv + Send + Sync + 'static,
    H::RoTx: Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let id = t.into();

    let task = async move {
        let cold = ctx.cold();
        let block_num = resolve_block_id(id, ctx.tags(), &cold).await.map_err(|e| e.to_string())?;

        cold.get_header_by_number(block_num)
            .await
            .map(|h| h.map(alloy::rpc::types::Header::new))
            .map_err(|e| e.to_string())
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

// ---------------------------------------------------------------------------
// Transaction Queries
// ---------------------------------------------------------------------------

pub(crate) async fn transaction_by_hash<H>(
    hctx: HandlerCtx,
    (hash,): (B256,),
    ctx: StorageRpcCtx<H>,
) -> Result<Option<alloy::rpc::types::Transaction>, String>
where
    H: HotKv + Send + Sync + 'static,
    H::RoTx: Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let task = async move {
        let cold = ctx.cold();
        let Some(confirmed) = cold.get_tx_by_hash(hash).await.map_err(|e| e.to_string())? else {
            return Ok(None);
        };

        let (tx, meta) = confirmed.into_parts();

        // Fetch header for base_fee
        let header =
            cold.get_header_by_number(meta.block_number()).await.map_err(|e| e.to_string())?;
        let base_fee = header.and_then(|h| h.base_fee_per_gas());

        build_rpc_transaction(tx, &meta, base_fee).map(Some).map_err(|e| e.to_string())
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

pub(crate) async fn raw_transaction_by_hash<H>(
    hctx: HandlerCtx,
    (hash,): (B256,),
    ctx: StorageRpcCtx<H>,
) -> Result<Option<alloy::primitives::Bytes>, String>
where
    H: HotKv + Send + Sync + 'static,
    H::RoTx: Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let task = async move {
        ctx.cold()
            .get_tx_by_hash(hash)
            .await
            .map(|opt| opt.map(|c| c.into_inner().encoded_2718().into()))
            .map_err(|e| e.to_string())
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

pub(crate) async fn tx_by_block_and_index<T, H>(
    hctx: HandlerCtx,
    (t, index): (T, U64),
    ctx: StorageRpcCtx<H>,
) -> Result<Option<alloy::rpc::types::Transaction>, String>
where
    T: Into<BlockId>,
    H: HotKv + Send + Sync + 'static,
    H::RoTx: Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let id = t.into();

    let task = async move {
        let cold = ctx.cold();
        let block_num = resolve_block_id(id, ctx.tags(), &cold).await.map_err(|e| e.to_string())?;

        let Some(confirmed) = cold
            .get_tx_by_block_and_index(block_num, index.to::<u64>())
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };

        let (tx, meta) = confirmed.into_parts();
        let header =
            cold.get_header_by_number(meta.block_number()).await.map_err(|e| e.to_string())?;
        let base_fee = header.and_then(|h| h.base_fee_per_gas());

        build_rpc_transaction(tx, &meta, base_fee).map(Some).map_err(|e| e.to_string())
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

pub(crate) async fn raw_tx_by_block_and_index<T, H>(
    hctx: HandlerCtx,
    (t, index): (T, U64),
    ctx: StorageRpcCtx<H>,
) -> Result<Option<alloy::primitives::Bytes>, String>
where
    T: Into<BlockId>,
    H: HotKv + Send + Sync + 'static,
    H::RoTx: Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let id = t.into();

    let task = async move {
        let cold = ctx.cold();
        let block_num = resolve_block_id(id, ctx.tags(), &cold).await.map_err(|e| e.to_string())?;

        cold.get_tx_by_block_and_index(block_num, index.to::<u64>())
            .await
            .map(|opt| opt.map(|c| c.into_inner().encoded_2718().into()))
            .map_err(|e| e.to_string())
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

pub(crate) async fn transaction_receipt<H>(
    hctx: HandlerCtx,
    (hash,): (B256,),
    ctx: StorageRpcCtx<H>,
) -> Result<Option<alloy::rpc::types::TransactionReceipt>, String>
where
    H: HotKv + Send + Sync + 'static,
    H::RoTx: Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let task = async move {
        let Some(receipt_ctx) = ctx
            .cold()
            .get_receipt_with_context(ReceiptSpecifier::TxHash(hash))
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };

        build_receipt(receipt_ctx).map(Some).map_err(|e| e.to_string())
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

// ---------------------------------------------------------------------------
// Account State (Hot Storage)
// ---------------------------------------------------------------------------

pub(crate) async fn balance<H>(
    hctx: HandlerCtx,
    AddrWithBlock(address, block): AddrWithBlock,
    ctx: StorageRpcCtx<H>,
) -> Result<U256, String>
where
    H: HotKv + Send + Sync + 'static,
    H::RoTx: Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let block = block.unwrap_or(BlockId::latest());

    let task = async move {
        let cold = ctx.cold();
        let height = resolve_block_id(block, ctx.tags(), &cold).await.map_err(|e| e.to_string())?;

        let reader = ctx.hot_reader().map_err(|e| e.to_string())?;
        let acct =
            reader.get_account_at_height(&address, Some(height)).map_err(|e| e.to_string())?;

        Ok(acct.map(|a| a.balance).unwrap_or_default())
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

pub(crate) async fn storage_at<H>(
    hctx: HandlerCtx,
    StorageAtArgs(address, key, block): StorageAtArgs,
    ctx: StorageRpcCtx<H>,
) -> Result<B256, String>
where
    H: HotKv + Send + Sync + 'static,
    H::RoTx: Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let block = block.unwrap_or(BlockId::latest());

    let task = async move {
        let cold = ctx.cold();
        let height = resolve_block_id(block, ctx.tags(), &cold).await.map_err(|e| e.to_string())?;

        let reader = ctx.hot_reader().map_err(|e| e.to_string())?;
        let val = reader
            .get_storage_at_height(&address, &key, Some(height))
            .map_err(|e| e.to_string())?;

        Ok(val.unwrap_or_default().to_be_bytes().into())
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

pub(crate) async fn addr_tx_count<H>(
    hctx: HandlerCtx,
    AddrWithBlock(address, block): AddrWithBlock,
    ctx: StorageRpcCtx<H>,
) -> Result<U64, String>
where
    H: HotKv + Send + Sync + 'static,
    H::RoTx: Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let block = block.unwrap_or(BlockId::latest());

    let task = async move {
        let cold = ctx.cold();
        let height = resolve_block_id(block, ctx.tags(), &cold).await.map_err(|e| e.to_string())?;

        let reader = ctx.hot_reader().map_err(|e| e.to_string())?;
        let acct =
            reader.get_account_at_height(&address, Some(height)).map_err(|e| e.to_string())?;

        Ok(U64::from(acct.map(|a| a.nonce).unwrap_or_default()))
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

pub(crate) async fn code_at<H>(
    hctx: HandlerCtx,
    AddrWithBlock(address, block): AddrWithBlock,
    ctx: StorageRpcCtx<H>,
) -> Result<alloy::primitives::Bytes, String>
where
    H: HotKv + Send + Sync + 'static,
    H::RoTx: Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let block = block.unwrap_or(BlockId::latest());

    let task = async move {
        let cold = ctx.cold();
        let height = resolve_block_id(block, ctx.tags(), &cold).await.map_err(|e| e.to_string())?;

        let reader = ctx.hot_reader().map_err(|e| e.to_string())?;
        let acct =
            reader.get_account_at_height(&address, Some(height)).map_err(|e| e.to_string())?;

        let Some(acct) = acct else {
            return Ok(alloy::primitives::Bytes::new());
        };

        let Some(code_hash) = acct.bytecode_hash else {
            return Ok(alloy::primitives::Bytes::new());
        };

        let code = reader.get_bytecode(&code_hash).map_err(|e| e.to_string())?;

        Ok(code.map(|c| c.original_bytes()).unwrap_or_default())
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

// ---------------------------------------------------------------------------
// EVM Execution
// ---------------------------------------------------------------------------

pub(crate) async fn call<H>(
    hctx: HandlerCtx,
    TxParams(mut request, block, state_overrides, block_overrides): TxParams,
    ctx: StorageRpcCtx<H>,
) -> ResponsePayload<alloy::primitives::Bytes, CallErrorData>
where
    H: HotKv + Send + Sync + 'static,
    H::RoTx: Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let max_gas = ctx.rpc_gas_cap();
    normalize_gas_stateless(&mut request, max_gas);

    let id = block.unwrap_or(BlockId::latest());
    let span = trace_span!("eth_call", block_id = %id);

    let task = async move {
        let EvmBlockContext { header, db } = response_tri!(ctx.resolve_evm_block(id).await);

        let trevm = signet_evm::signet_evm(db, ctx.constants().clone())
            .fill_cfg(&CfgFiller(ctx.chain_id()))
            .fill_block(&header);

        let trevm = response_tri!(trevm.maybe_apply_state_overrides(state_overrides.as_ref()))
            .maybe_apply_block_overrides(block_overrides.as_deref())
            .fill_tx(&request);

        let mut trevm = trevm;
        let new_gas = response_tri!(trevm.cap_tx_gas());
        if Some(new_gas) != request.gas {
            debug!(req_gas = ?request.gas, new_gas, "capping gas for call");
        }

        let result = response_tri!(trevm.call().map_err(signet_evm::EvmErrored::into_error));

        match result.0 {
            trevm::revm::context::result::ExecutionResult::Success { output, .. } => {
                ResponsePayload::Success(output.data().clone())
            }
            trevm::revm::context::result::ExecutionResult::Revert { output, .. } => {
                ResponsePayload::internal_error_with_message_and_obj(
                    "execution reverted".into(),
                    output.clone().into(),
                )
            }
            trevm::revm::context::result::ExecutionResult::Halt { reason, .. } => {
                ResponsePayload::internal_error_with_message_and_obj(
                    "execution halted".into(),
                    format!("{reason:?}").into(),
                )
            }
        }
    }
    .instrument(span);

    await_handler!(@response_option hctx.spawn_blocking(task))
}

pub(crate) async fn estimate_gas<H>(
    hctx: HandlerCtx,
    TxParams(mut request, block, state_overrides, block_overrides): TxParams,
    ctx: StorageRpcCtx<H>,
) -> ResponsePayload<U64, CallErrorData>
where
    H: HotKv + Send + Sync + 'static,
    H::RoTx: Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let max_gas = ctx.rpc_gas_cap();
    normalize_gas_stateless(&mut request, max_gas);

    let id = block.unwrap_or(BlockId::pending());
    let span = trace_span!("eth_estimateGas", block_id = %id);

    let task = async move {
        let EvmBlockContext { header, db } = response_tri!(ctx.resolve_evm_block(id).await);

        let trevm = signet_evm::signet_evm(db, ctx.constants().clone())
            .fill_cfg(&CfgFiller(ctx.chain_id()))
            .fill_block(&header);

        let trevm = response_tri!(trevm.maybe_apply_state_overrides(state_overrides.as_ref()))
            .maybe_apply_block_overrides(block_overrides.as_deref())
            .fill_tx(&request);

        let (estimate, _) =
            response_tri!(trevm.estimate_gas().map_err(signet_evm::EvmErrored::into_error));

        match estimate {
            EstimationResult::Success { limit, .. } => ResponsePayload::Success(U64::from(limit)),
            EstimationResult::Revert { reason, .. } => {
                ResponsePayload::internal_error_with_message_and_obj(
                    "execution reverted".into(),
                    reason.clone().into(),
                )
            }
            EstimationResult::Halt { reason, .. } => {
                ResponsePayload::internal_error_with_message_and_obj(
                    "execution halted".into(),
                    format!("{reason:?}").into(),
                )
            }
        }
    }
    .instrument(span);

    await_handler!(@response_option hctx.spawn_blocking(task))
}

// ---------------------------------------------------------------------------
// Transaction Submission
// ---------------------------------------------------------------------------

pub(crate) async fn send_raw_transaction<H>(
    hctx: HandlerCtx,
    (tx,): (alloy::primitives::Bytes,),
    ctx: StorageRpcCtx<H>,
) -> Result<B256, String>
where
    H: HotKv + Send + Sync + 'static,
    H::RoTx: Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let Some(tx_cache) = ctx.tx_cache().cloned() else {
        return Err("tx-cache URL not provided".to_string());
    };

    let task = |hctx: HandlerCtx| async move {
        let envelope = alloy::consensus::TxEnvelope::decode_2718(&mut tx.as_ref())
            .map_err(|e| e.to_string())?;

        let hash = *envelope.tx_hash();
        hctx.spawn(async move {
            if let Err(e) = tx_cache.forward_raw_transaction(envelope).await {
                tracing::warn!(%hash, err = %e, "failed to forward raw transaction");
            }
        });

        Ok(hash)
    };

    await_handler!(@option hctx.spawn_blocking_with_ctx(task))
}

// ---------------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------------

/// Maximum number of blocks per `eth_getLogs` range query.
const MAX_BLOCKS_PER_FILTER: u64 = 10_000;

/// Maximum headers fetched per batch when scanning bloom filters.
const MAX_HEADERS_RANGE: u64 = 1_000;

pub(crate) async fn get_logs<H>(
    hctx: HandlerCtx,
    (filter,): (Filter,),
    ctx: StorageRpcCtx<H>,
) -> Result<Vec<Log>, String>
where
    H: HotKv + Send + Sync + 'static,
    H::RoTx: Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let task = async move {
        let cold = ctx.cold();

        // Build bloom filters for efficient block-level filtering.
        let address_filter = FilteredParams::address_filter(&filter.address);
        let topics_filter = FilteredParams::topics_filter(&filter.topics);

        match filter.block_option {
            alloy::rpc::types::FilterBlockOption::AtBlockHash(hash) => {
                let header = cold
                    .get_header_by_hash(hash)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("block not found: {hash}"))?;

                if !FilteredParams::matches_address(header.logs_bloom, &address_filter)
                    || !FilteredParams::matches_topics(header.logs_bloom, &topics_filter)
                {
                    return Ok(vec![]);
                }

                let block_num = header.number;
                let (txs, receipts) = tokio::try_join!(
                    cold.get_transactions_in_block(block_num),
                    cold.get_receipts_in_block(block_num),
                )
                .map_err(|e| e.to_string())?;

                Ok(collect_matching_logs(&header, hash, &txs, &receipts, &filter))
            }

            alloy::rpc::types::FilterBlockOption::Range { from_block, to_block } => {
                let from = from_block
                    .map(|b| resolve_block_number_or_tag(b, ctx.tags()))
                    .transpose()
                    .map_err(|e| e.to_string())?
                    .unwrap_or(0);
                let to = to_block
                    .map(|b| resolve_block_number_or_tag(b, ctx.tags()))
                    .transpose()
                    .map_err(|e| e.to_string())?
                    .unwrap_or_else(|| ctx.tags().latest());

                if from > to {
                    return Err("fromBlock must not exceed toBlock".to_string());
                }
                if to - from > MAX_BLOCKS_PER_FILTER {
                    return Err(format!("query exceeds max block range ({MAX_BLOCKS_PER_FILTER})"));
                }

                let mut all_logs = Vec::new();

                for (chunk_start, chunk_end) in
                    BlockRangeInclusiveIter::new(from..=to, MAX_HEADERS_RANGE)
                {
                    let specs: Vec<_> =
                        (chunk_start..=chunk_end).map(HeaderSpecifier::Number).collect();

                    let headers = cold.get_headers(specs).await.map_err(|e| e.to_string())?;

                    for (offset, maybe_header) in headers.into_iter().enumerate() {
                        let Some(header) = maybe_header else {
                            continue;
                        };

                        if !FilteredParams::matches_address(header.logs_bloom, &address_filter)
                            || !FilteredParams::matches_topics(header.logs_bloom, &topics_filter)
                        {
                            continue;
                        }

                        let block_num = chunk_start + offset as u64;
                        let block_hash = header.hash_slow();

                        let (txs, receipts) = tokio::try_join!(
                            cold.get_transactions_in_block(block_num),
                            cold.get_receipts_in_block(block_num),
                        )
                        .map_err(|e| e.to_string())?;

                        let logs =
                            collect_matching_logs(&header, block_hash, &txs, &receipts, &filter);
                        all_logs.extend(logs);
                    }
                }

                Ok(all_logs)
            }
        }
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

/// Extract logs from a block's receipts that match the filter's address and topic criteria.
fn collect_matching_logs(
    header: &alloy::consensus::Header,
    block_hash: B256,
    txs: &[signet_storage_types::TransactionSigned],
    receipts: &[signet_storage_types::Receipt],
    filter: &Filter,
) -> Vec<Log> {
    let mut logs = Vec::new();
    let mut log_index: u64 = 0;

    for (tx_idx, (tx, receipt)) in txs.iter().zip(receipts.iter()).enumerate() {
        let tx_hash = *tx.tx_hash();

        for log in &receipt.inner.logs {
            if filter.matches_address(log.address) && filter.matches_topics(log.topics()) {
                logs.push(Log {
                    inner: log.clone(),
                    block_hash: Some(block_hash),
                    block_number: Some(header.number),
                    block_timestamp: Some(header.timestamp),
                    transaction_hash: Some(tx_hash),
                    transaction_index: Some(tx_idx as u64),
                    log_index: Some(log_index),
                    removed: false,
                });
            }
            log_index += 1;
        }
    }

    logs
}
