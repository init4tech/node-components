//! ETH namespace RPC endpoint implementations.

use crate::{
    ctx::{EvmBlockContext, StorageRpcCtx},
    eth::helpers::{
        AddrWithBlock, BlockParams, BlockRangeInclusiveIter, CfgFiller, FeeHistoryArgs,
        StorageAtArgs, SubscribeArgs, TxParams, await_handler, build_receipt,
        build_receipt_from_parts, build_rpc_transaction, normalize_gas_stateless, response_tri,
    },
    gas_oracle,
    interest::{FilterOutput, InterestKind},
};
use ajj::{HandlerCtx, ResponsePayload};
use alloy::{
    consensus::{BlockHeader, Transaction, TxReceipt},
    eips::{
        BlockId, BlockNumberOrTag,
        eip1559::BaseFeeParams,
        eip2718::{Decodable2718, Encodable2718},
    },
    primitives::{B256, U64, U256},
    rpc::types::{Block, BlockTransactions, FeeHistory, Filter, FilteredParams, Log},
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
// Gas & Fee Queries
// ---------------------------------------------------------------------------

pub(crate) async fn gas_price<H>(hctx: HandlerCtx, ctx: StorageRpcCtx<H>) -> Result<U256, String>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let task = async move {
        let latest = ctx.tags().latest();
        let cold = ctx.cold();

        let tip = gas_oracle::suggest_tip_cap(&cold, latest, ctx.config())
            .await
            .map_err(|e| e.to_string())?;

        let base_fee = cold
            .get_header_by_number(latest)
            .await
            .map_err(|e| e.to_string())?
            .and_then(|h| h.base_fee_per_gas)
            .unwrap_or_default();

        Ok(tip + U256::from(base_fee))
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

pub(crate) async fn max_priority_fee_per_gas<H>(
    hctx: HandlerCtx,
    ctx: StorageRpcCtx<H>,
) -> Result<U256, String>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let task = async move {
        let latest = ctx.tags().latest();
        gas_oracle::suggest_tip_cap(&ctx.cold(), latest, ctx.config())
            .await
            .map_err(|e| e.to_string())
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

pub(crate) async fn fee_history<H>(
    hctx: HandlerCtx,
    FeeHistoryArgs(block_count, newest, reward_percentiles): FeeHistoryArgs,
    ctx: StorageRpcCtx<H>,
) -> Result<FeeHistory, String>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let task = async move {
        let mut block_count = block_count.to::<u64>();

        if block_count == 0 {
            return Ok(FeeHistory::default());
        }

        let max_fee_history = if reward_percentiles.is_none() {
            ctx.config().max_header_history
        } else {
            ctx.config().max_block_history
        };

        block_count = block_count.min(max_fee_history);

        let mut newest = newest;
        if newest.is_pending() {
            newest = BlockNumberOrTag::Latest;
            block_count = block_count.saturating_sub(1);
        }

        let end_block = ctx.resolve_block_tag(newest);
        let end_block_plus = end_block + 1;

        block_count = block_count.min(end_block_plus);

        // Validate percentiles
        if let Some(percentiles) = &reward_percentiles
            && percentiles.windows(2).any(|w| w[0] > w[1] || w[0] > 100.)
        {
            return Err("invalid reward percentiles".to_string());
        }

        let start_block = end_block_plus - block_count;
        let cold = ctx.cold();

        let specs: Vec<_> = (start_block..=end_block).map(HeaderSpecifier::Number).collect();
        let headers = cold.get_headers(specs).await.map_err(|e| e.to_string())?;

        let mut base_fee_per_gas: Vec<u128> = Vec::with_capacity(headers.len() + 1);
        let mut gas_used_ratio: Vec<f64> = Vec::with_capacity(headers.len());
        let mut rewards: Vec<Vec<u128>> = Vec::new();

        for (offset, maybe_header) in headers.iter().enumerate() {
            let Some(header) = maybe_header else {
                return Err(format!("missing header at block {}", start_block + offset as u64));
            };

            base_fee_per_gas.push(header.base_fee_per_gas.unwrap_or_default() as u128);
            gas_used_ratio.push(if header.gas_limit > 0 {
                header.gas_used as f64 / header.gas_limit as f64
            } else {
                0.0
            });

            if let Some(percentiles) = &reward_percentiles {
                let block_num = start_block + offset as u64;

                let (txs, receipts) = tokio::try_join!(
                    cold.get_transactions_in_block(block_num),
                    cold.get_receipts_in_block(block_num),
                )
                .map_err(|e| e.to_string())?;

                let block_rewards = calculate_reward_percentiles(
                    percentiles,
                    header.gas_used,
                    header.base_fee_per_gas.unwrap_or_default(),
                    &txs,
                    &receipts,
                );
                rewards.push(block_rewards);
            }
        }

        // Next block base fee
        if let Some(last_header) = headers.last().and_then(|h| h.as_ref()) {
            base_fee_per_gas.push(
                last_header.next_block_base_fee(BaseFeeParams::ethereum()).unwrap_or_default()
                    as u128,
            );
        }

        let base_fee_per_blob_gas = vec![0; base_fee_per_gas.len()];
        let blob_gas_used_ratio = vec![0.; gas_used_ratio.len()];

        Ok(FeeHistory {
            base_fee_per_gas,
            gas_used_ratio,
            base_fee_per_blob_gas,
            blob_gas_used_ratio,
            oldest_block: start_block,
            reward: reward_percentiles.map(|_| rewards),
        })
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

/// Calculate reward percentiles for a single block.
///
/// Sorts transactions by effective tip ascending, then walks
/// cumulative gas used to find the tip value at each percentile.
fn calculate_reward_percentiles(
    percentiles: &[f64],
    gas_used: u64,
    base_fee: u64,
    txs: &[signet_storage_types::TransactionSigned],
    receipts: &[signet_storage_types::Receipt],
) -> Vec<u128> {
    if gas_used == 0 || txs.is_empty() {
        return vec![0; percentiles.len()];
    }

    // Pair each tx's effective tip with its gas used (from receipt cumulative deltas)
    let mut tx_gas_and_tip: Vec<(u64, u128)> = txs
        .iter()
        .zip(receipts.iter())
        .enumerate()
        .map(|(i, (tx, receipt))| {
            let prev_cumulative =
                if i > 0 { receipts[i - 1].inner.cumulative_gas_used() } else { 0 };
            let tx_gas = receipt.inner.cumulative_gas_used() - prev_cumulative;
            let tip = tx.effective_tip_per_gas(base_fee).unwrap_or_default();
            (tx_gas, tip)
        })
        .collect();

    // Sort by tip ascending
    tx_gas_and_tip.sort_by_key(|&(_, tip)| tip);

    let mut result = Vec::with_capacity(percentiles.len());
    let mut cumulative_gas: u64 = 0;
    let mut tx_idx = 0;

    for &percentile in percentiles {
        let threshold = (gas_used as f64 * percentile / 100.0) as u64;

        while tx_idx < tx_gas_and_tip.len() {
            cumulative_gas += tx_gas_and_tip[tx_idx].0;
            if cumulative_gas >= threshold {
                break;
            }
            tx_idx += 1;
        }

        result.push(tx_gas_and_tip.get(tx_idx).map(|&(_, tip)| tip).unwrap_or_default());
    }

    result
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
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let id = t.into();
    let full = full.unwrap_or(false);

    let task = async move {
        let cold = ctx.cold();
        let block_num = ctx.resolve_block_id(id).await.map_err(|e| e.to_string())?;

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
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let id = t.into();

    let task = async move {
        let cold = ctx.cold();
        let block_num = ctx.resolve_block_id(id).await.map_err(|e| e.to_string())?;

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
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let task = async move {
        let cold = ctx.cold();
        let block_num = ctx.resolve_block_id(id).await.map_err(|e| e.to_string())?;

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
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let id = t.into();

    let task = async move {
        let cold = ctx.cold();
        let block_num = ctx.resolve_block_id(id).await.map_err(|e| e.to_string())?;

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
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let id = t.into();

    let task = async move {
        let cold = ctx.cold();
        let block_num = ctx.resolve_block_id(id).await.map_err(|e| e.to_string())?;

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
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let id = t.into();

    let task = async move {
        let cold = ctx.cold();
        let block_num = ctx.resolve_block_id(id).await.map_err(|e| e.to_string())?;

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
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let block = block.unwrap_or(BlockId::latest());

    let task = async move {
        let height = ctx.resolve_block_id(block).await.map_err(|e| e.to_string())?;

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
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let block = block.unwrap_or(BlockId::latest());

    let task = async move {
        let height = ctx.resolve_block_id(block).await.map_err(|e| e.to_string())?;

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
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let block = block.unwrap_or(BlockId::latest());

    let task = async move {
        let height = ctx.resolve_block_id(block).await.map_err(|e| e.to_string())?;

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
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let block = block.unwrap_or(BlockId::latest());

    let task = async move {
        let height = ctx.resolve_block_id(block).await.map_err(|e| e.to_string())?;

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
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let max_gas = ctx.config().rpc_gas_cap;
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
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let max_gas = ctx.config().rpc_gas_cap;
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

/// Maximum headers fetched per batch when scanning bloom filters.
const MAX_HEADERS_RANGE: u64 = 1_000;

pub(crate) async fn get_logs<H>(
    hctx: HandlerCtx,
    (filter,): (Filter,),
    ctx: StorageRpcCtx<H>,
) -> Result<Vec<Log>, String>
where
    H: HotKv + Send + Sync + 'static,
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
                let from = from_block.map(|b| ctx.resolve_block_tag(b)).unwrap_or(0);
                let to = to_block
                    .map(|b| ctx.resolve_block_tag(b))
                    .unwrap_or_else(|| ctx.tags().latest());

                if from > to {
                    return Err("fromBlock must not exceed toBlock".to_string());
                }
                let max_blocks = ctx.config().max_blocks_per_filter;
                if to - from > max_blocks {
                    return Err(format!("query exceeds max block range ({max_blocks})"));
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

                        let max_logs = ctx.config().max_logs_per_response;
                        if max_logs > 0 && all_logs.len() > max_logs {
                            return Err(format!(
                                "query exceeds max logs per response ({max_logs})"
                            ));
                        }
                    }
                }

                Ok(all_logs)
            }
        }
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

/// Extract logs from a block's receipts that match the filter's address and topic criteria.
pub(crate) fn collect_matching_logs(
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

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

pub(crate) async fn new_filter<H>(
    hctx: HandlerCtx,
    (filter,): (Filter,),
    ctx: StorageRpcCtx<H>,
) -> Result<U64, String>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let task = async move {
        let latest = ctx.tags().latest();
        Ok(ctx.filter_manager().install_log_filter(latest, filter))
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

pub(crate) async fn new_block_filter<H>(
    hctx: HandlerCtx,
    ctx: StorageRpcCtx<H>,
) -> Result<U64, String>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let task = async move {
        let latest = ctx.tags().latest();
        Ok(ctx.filter_manager().install_block_filter(latest))
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

pub(crate) async fn uninstall_filter<H>(
    (id,): (U64,),
    ctx: StorageRpcCtx<H>,
) -> Result<bool, String>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    Ok(ctx.filter_manager().uninstall(id).is_some())
}

pub(crate) async fn get_filter_changes<H>(
    hctx: HandlerCtx,
    (id,): (U64,),
    ctx: StorageRpcCtx<H>,
) -> Result<FilterOutput, String>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let task = async move {
        let fm = ctx.filter_manager();
        let mut entry = fm.get_mut(id).ok_or_else(|| format!("filter not found: {id}"))?;

        let latest = ctx.tags().latest();
        let start = entry.next_start_block();

        if start > latest {
            entry.mark_polled(latest);
            return Ok(entry.empty_output());
        }

        let cold = ctx.cold();

        if entry.is_block() {
            let specs: Vec<_> = (start..=latest).map(HeaderSpecifier::Number).collect();
            let headers = cold.get_headers(specs).await.map_err(|e| e.to_string())?;
            let hashes: Vec<B256> = headers.into_iter().flatten().map(|h| h.hash_slow()).collect();
            entry.mark_polled(latest);
            Ok(FilterOutput::from(hashes))
        } else {
            let filter = entry.as_filter().cloned().unwrap();
            let address_filter = FilteredParams::address_filter(&filter.address);
            let topics_filter = FilteredParams::topics_filter(&filter.topics);

            let mut all_logs = Vec::new();

            for (chunk_start, chunk_end) in
                BlockRangeInclusiveIter::new(start..=latest, MAX_HEADERS_RANGE)
            {
                let specs: Vec<_> =
                    (chunk_start..=chunk_end).map(HeaderSpecifier::Number).collect();
                let headers = cold.get_headers(specs).await.map_err(|e| e.to_string())?;

                for (offset, maybe_header) in headers.into_iter().enumerate() {
                    let Some(header) = maybe_header else { continue };

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

                    all_logs.extend(collect_matching_logs(
                        &header, block_hash, &txs, &receipts, &filter,
                    ));
                }
            }

            entry.mark_polled(latest);
            Ok(FilterOutput::from(all_logs))
        }
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

// ---------------------------------------------------------------------------
// Subscriptions
// ---------------------------------------------------------------------------

pub(crate) async fn subscribe<H>(
    hctx: HandlerCtx,
    SubscribeArgs(kind, filter): SubscribeArgs,
    ctx: StorageRpcCtx<H>,
) -> Result<U64, String>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let interest = match kind {
        alloy::rpc::types::pubsub::SubscriptionKind::NewHeads => InterestKind::Block,
        alloy::rpc::types::pubsub::SubscriptionKind::Logs => {
            let f = filter.unwrap_or_default();
            InterestKind::Log(f)
        }
        other => {
            return Err(format!("unsupported subscription kind: {other:?}"));
        }
    };

    ctx.sub_manager()
        .subscribe(&hctx, interest)
        .ok_or_else(|| "notifications not enabled on this transport".to_string())
}

pub(crate) async fn unsubscribe<H>((id,): (U64,), ctx: StorageRpcCtx<H>) -> Result<bool, String>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    Ok(ctx.sub_manager().unsubscribe(id))
}
