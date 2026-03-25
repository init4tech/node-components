//! Debug namespace RPC endpoint implementations.

use crate::{
    config::StorageRpcCtx,
    debug::{
        DebugError,
        types::{TraceBlockParams, TraceTransactionParams},
    },
    eth::helpers::{CfgFiller, await_handler},
};
use ajj::HandlerCtx;
use alloy::{
    consensus::{
        BlockHeader, Receipt, ReceiptEnvelope, ReceiptWithBloom, TxReceipt,
        transaction::SignerRecoverable,
    },
    eips::{BlockId, eip2718::Encodable2718},
    primitives::{B256, Bytes, Log},
    rpc::types::trace::geth::{GethDebugTracingOptions, GethTrace, TraceResult},
};
use itertools::Itertools;
use signet_hot::{HotKv, model::HotKvRead};
use signet_types::{MagicSig, constants::SignetSystemConstants};
use tracing::Instrument;
use trevm::revm::{
    Database, DatabaseRef,
    database::{DBErrorMarker, State},
    primitives::hardfork::SpecId,
};

/// Shared tracing loop used by block-level debug handlers.
///
/// Sets up the EVM from pre-resolved components, iterates through
/// transactions (stopping at the first magic-signature tx), and traces
/// each one according to the provided [`GethDebugTracingOptions`].
#[allow(clippy::too_many_arguments)]
fn trace_block_inner<Db>(
    ctx_chain_id: u64,
    constants: SignetSystemConstants,
    spec_id: SpecId,
    header: &alloy::consensus::Header,
    block_hash: B256,
    txs: &[signet_storage_types::RecoveredTx],
    db: State<Db>,
    opts: &GethDebugTracingOptions,
) -> Result<Vec<TraceResult>, DebugError>
where
    Db: Database + DatabaseRef,
    <Db as Database>::Error: DBErrorMarker,
    <Db as DatabaseRef>::Error: DBErrorMarker,
{
    let mut evm = signet_evm::signet_evm(db, constants);
    evm.set_spec_id(spec_id);
    let mut trevm = evm.fill_cfg(&CfgFiller(ctx_chain_id)).fill_block(header);

    let mut frames = Vec::with_capacity(txs.len());
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
        let frame;
        (frame, trevm) = crate::debug::tracer::trace(t, opts, tx_info)?;
        frames.push(TraceResult::Success { result: frame, tx_hash: Some(*tx.tx_hash()) });

        tracing::debug!(tx_index = idx, tx_hash = ?tx.tx_hash(), "Traced transaction");
    }

    Ok(frames)
}

/// `debug_traceBlockByNumber` and `debug_traceBlockByHash` handler.
pub(super) async fn trace_block<T, H>(
    hctx: HandlerCtx,
    TraceBlockParams(id, opts): TraceBlockParams<T>,
    ctx: StorageRpcCtx<H>,
) -> Result<Vec<TraceResult>, DebugError>
where
    T: Into<BlockId>,
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let opts = opts.ok_or(DebugError::InvalidTracerConfig)?;

    // Acquire a tracing semaphore permit to limit concurrent debug
    // requests. The permit is held for the entire handler lifetime and
    // is dropped when the async block completes.
    let _permit = ctx.acquire_tracing_permit().await;

    let id = id.into();
    let span = tracing::debug_span!("traceBlock", ?id, tracer = ?opts.tracer.as_ref());

    let fut = async move {
        let cold = ctx.cold();
        let block_num = ctx.resolve_block_id(id).map_err(|e| {
            tracing::warn!(error = %e, ?id, "block resolution failed");
            DebugError::Resolve(e)
        })?;

        let sealed = ctx.resolve_header(BlockId::Number(block_num.into())).map_err(|e| {
            tracing::warn!(error = %e, block_num, "header resolution failed");
            DebugError::Resolve(e)
        })?;

        let Some(sealed) = sealed else {
            return Err(DebugError::BlockNotFound(id));
        };

        let block_hash = sealed.hash();
        let header = sealed.into_inner();

        let txs = cold.get_transactions_in_block(block_num).await.map_err(|e| {
            tracing::warn!(error = %e, block_num, "cold storage read failed");
            DebugError::from(e)
        })?;

        tracing::debug!(number = header.number, "Loaded block");

        // State BEFORE this block.
        let db = ctx.revm_state_at_height(header.number.saturating_sub(1)).map_err(|e| {
            tracing::warn!(error = %e, block_num, "hot storage read failed");
            DebugError::from(e)
        })?;

        let spec_id = ctx.spec_id_for_header(&header);
        trace_block_inner(
            ctx.chain_id(),
            ctx.constants().clone(),
            spec_id,
            &header,
            block_hash,
            &txs,
            db,
            &opts,
        )
    }
    .instrument(span);

    await_handler!(hctx.spawn(fut), DebugError::Internal("task panicked or cancelled".into()))
}

/// `debug_traceTransaction` handler.
pub(super) async fn trace_transaction<H>(
    hctx: HandlerCtx,
    TraceTransactionParams(tx_hash, opts): TraceTransactionParams,
    ctx: StorageRpcCtx<H>,
) -> Result<GethTrace, DebugError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let opts = opts.ok_or(DebugError::InvalidTracerConfig)?;

    // Held for the handler duration; dropped when the async block completes.
    let _permit = ctx.acquire_tracing_permit().await;

    let span = tracing::debug_span!("traceTransaction", %tx_hash, tracer = ?opts.tracer.as_ref());

    let fut = async move {
        let cold = ctx.cold();

        // Look up the transaction and its containing block.
        let confirmed = cold.get_tx_by_hash(tx_hash).await.map_err(|e| {
            tracing::warn!(error = %e, %tx_hash, "cold storage read failed");
            DebugError::from(e)
        })?;

        let confirmed = confirmed.ok_or(DebugError::TransactionNotFound(tx_hash))?;
        let (_tx, meta) = confirmed.into_parts();

        let block_num = meta.block_number();
        let block_hash = meta.block_hash();

        let block_id = BlockId::Number(block_num.into());
        let sealed = ctx.resolve_header(block_id).map_err(|e| {
            tracing::warn!(error = %e, block_num, "header resolution failed");
            DebugError::Resolve(e)
        })?;
        let header = sealed.ok_or(DebugError::BlockNotFound(block_id))?.into_inner();

        let txs = cold.get_transactions_in_block(block_num).await.map_err(|e| {
            tracing::warn!(error = %e, block_num, "cold storage read failed");
            DebugError::from(e)
        })?;

        tracing::debug!(number = block_num, "Loaded containing block");

        // State BEFORE this block.
        let db = ctx.revm_state_at_height(block_num.saturating_sub(1)).map_err(|e| {
            tracing::warn!(error = %e, block_num, "hot storage read failed");
            DebugError::from(e)
        })?;

        let spec_id = ctx.spec_id_for_header(&header);
        let mut evm = signet_evm::signet_evm(db, ctx.constants().clone());
        evm.set_spec_id(spec_id);
        let mut trevm = evm.fill_cfg(&CfgFiller(ctx.chain_id())).fill_block(&header);

        // Replay all transactions up to (but not including) the target
        let mut txns = txs.iter().enumerate().peekable();
        for (_idx, tx) in txns.by_ref().peeking_take_while(|(_, t)| t.tx_hash() != &tx_hash) {
            if MagicSig::try_from_signature(tx.signature()).is_some() {
                return Err(DebugError::TransactionNotFound(tx_hash));
            }

            trevm = trevm
                .run_tx(tx)
                .map_err(|e| DebugError::EvmHalt { reason: e.into_error().to_string() })?
                .accept_state();
        }

        let (index, tx) = txns.next().ok_or(DebugError::TransactionNotFound(tx_hash))?;

        let trevm = trevm.fill_tx(tx);

        let tx_info = alloy::rpc::types::TransactionInfo {
            hash: Some(*tx.tx_hash()),
            index: Some(index as u64),
            block_hash: Some(block_hash),
            block_number: Some(header.number),
            base_fee: header.base_fee_per_gas(),
        };

        let res = crate::debug::tracer::trace(trevm, &opts, tx_info)?.0;

        Ok(res)
    }
    .instrument(span);

    await_handler!(hctx.spawn(fut), DebugError::Internal("task panicked or cancelled".into()))
}

/// `debug_traceBlock` — trace all transactions in a raw RLP-encoded block.
pub(super) async fn trace_block_rlp<H>(
    hctx: HandlerCtx,
    (rlp_bytes, opts): (Bytes, Option<GethDebugTracingOptions>),
    ctx: StorageRpcCtx<H>,
) -> Result<Vec<TraceResult>, DebugError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let opts = opts.ok_or(DebugError::InvalidTracerConfig)?;
    let _permit = ctx.acquire_tracing_permit().await;

    let span = tracing::debug_span!("traceBlock(RLP)", bytes_len = rlp_bytes.len());

    let fut = async move {
        let block: alloy::consensus::Block<signet_storage_types::TransactionSigned> =
            alloy::rlp::Decodable::decode(&mut rlp_bytes.as_ref())
                .map_err(|e| DebugError::RlpDecode(e.to_string()))?;

        let block_hash = block.header.hash_slow();

        let txs = block
            .body
            .transactions
            .into_iter()
            .map(|tx| tx.try_into_recovered().map_err(|_| DebugError::SenderRecovery))
            .collect::<Result<Vec<_>, _>>()?;

        let db = ctx.revm_state_at_height(block.header.number.saturating_sub(1)).map_err(|e| {
            tracing::warn!(error = %e, number = block.header.number, "hot storage read failed");
            DebugError::from(e)
        })?;

        let spec_id = ctx.spec_id_for_header(&block.header);

        trace_block_inner(
            ctx.chain_id(),
            ctx.constants().clone(),
            spec_id,
            &block.header,
            block_hash,
            &txs,
            db,
            &opts,
        )
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        DebugError::EvmHalt { reason: "task panicked or cancelled".into() }
    )
}

/// `debug_getRawBlock` handler.
///
/// Resolves the given [`BlockId`], fetches header and transactions from cold
/// storage, assembles them into an [`alloy::consensus::Block`], and returns
/// the RLP-encoded bytes.
pub(super) async fn get_raw_block<H>(
    hctx: HandlerCtx,
    (id,): (BlockId,),
    ctx: StorageRpcCtx<H>,
) -> Result<Bytes, DebugError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let span = tracing::debug_span!("getRawBlock", ?id);

    let fut = async move {
        let cold = ctx.cold();
        let block_num = ctx.resolve_block_id(id).map_err(|e| {
            tracing::warn!(error = %e, ?id, "block resolution failed");
            DebugError::Resolve(e)
        })?;

        let sealed = ctx.resolve_header(BlockId::Number(block_num.into())).map_err(|e| {
            tracing::warn!(error = %e, block_num, "header resolution failed");
            DebugError::BlockNotFound(id)
        })?;

        let Some(sealed) = sealed else {
            return Err(DebugError::BlockNotFound(id));
        };

        let txs = cold.get_transactions_in_block(block_num).await.map_err(|e| {
            tracing::warn!(error = %e, block_num, "cold storage read failed");
            DebugError::from(e)
        })?;

        let header = sealed.into_inner();
        let tx_bodies: Vec<_> = txs.into_iter().map(|tx| tx.into_inner()).collect();
        let block = alloy::consensus::Block {
            header,
            body: alloy::consensus::BlockBody {
                transactions: tx_bodies,
                ommers: vec![],
                withdrawals: None,
            },
        };

        Ok(Bytes::from(alloy::rlp::encode(&block)))
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        DebugError::EvmHalt { reason: "task panicked or cancelled".into() }
    )
}

/// `debug_getRawReceipts` handler.
///
/// Fetches all receipts for the given [`BlockId`] and returns a list of
/// EIP-2718 encoded consensus receipt envelopes (one per transaction).
pub(super) async fn get_raw_receipts<H>(
    hctx: HandlerCtx,
    (id,): (BlockId,),
    ctx: StorageRpcCtx<H>,
) -> Result<Vec<Bytes>, DebugError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let span = tracing::debug_span!("getRawReceipts", ?id);

    let fut = async move {
        let block_num = ctx.resolve_block_id(id).map_err(|e| {
            tracing::warn!(error = %e, ?id, "block resolution failed");
            DebugError::Resolve(e)
        })?;

        let receipts = ctx.cold().get_receipts_in_block(block_num).await.map_err(|e| {
            tracing::warn!(error = %e, block_num, "cold storage read failed");
            DebugError::from(e)
        })?;

        let encoded = receipts
            .into_iter()
            .map(|cr| {
                let logs_bloom = cr.receipt.bloom();
                let logs: Vec<Log> = cr.receipt.logs.into_iter().map(|l| l.inner).collect();
                let receipt = Receipt {
                    status: cr.receipt.status,
                    cumulative_gas_used: cr.receipt.cumulative_gas_used,
                    logs,
                };
                let rwb = ReceiptWithBloom { receipt, logs_bloom };
                let envelope: ReceiptEnvelope<Log> = match cr.tx_type {
                    alloy::consensus::TxType::Legacy => ReceiptEnvelope::Legacy(rwb),
                    alloy::consensus::TxType::Eip2930 => ReceiptEnvelope::Eip2930(rwb),
                    alloy::consensus::TxType::Eip1559 => ReceiptEnvelope::Eip1559(rwb),
                    alloy::consensus::TxType::Eip4844 => ReceiptEnvelope::Eip4844(rwb),
                    alloy::consensus::TxType::Eip7702 => ReceiptEnvelope::Eip7702(rwb),
                };
                Bytes::from(envelope.encoded_2718())
            })
            .collect();

        Ok(encoded)
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        DebugError::EvmHalt { reason: "task panicked or cancelled".into() }
    )
}

/// `debug_getRawHeader` handler.
///
/// Resolves the given [`BlockId`] and returns the RLP-encoded block header.
pub(super) async fn get_raw_header<H>(
    hctx: HandlerCtx,
    (id,): (BlockId,),
    ctx: StorageRpcCtx<H>,
) -> Result<Bytes, DebugError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let span = tracing::debug_span!("getRawHeader", ?id);

    let fut = async move {
        let block_num = ctx.resolve_block_id(id).map_err(|e| {
            tracing::warn!(error = %e, ?id, "block resolution failed");
            DebugError::Resolve(e)
        })?;

        let sealed = ctx.resolve_header(BlockId::Number(block_num.into())).map_err(|e| {
            tracing::warn!(error = %e, block_num, "header resolution failed");
            DebugError::BlockNotFound(id)
        })?;

        let Some(sealed) = sealed else {
            return Err(DebugError::BlockNotFound(id));
        };

        let header = sealed.into_inner();
        Ok(Bytes::from(alloy::rlp::encode(&header)))
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        DebugError::EvmHalt { reason: "task panicked or cancelled".into() }
    )
}

/// `debug_traceCall` — trace a call without submitting a transaction.
///
/// Resolves EVM state at the target block, prepares the transaction
/// from a [`alloy::rpc::types::TransactionRequest`], then routes through
/// the tracer. State overrides are not supported in this initial
/// implementation.
pub(super) async fn debug_trace_call<H>(
    hctx: HandlerCtx,
    (request, block_id, opts): (
        alloy::rpc::types::TransactionRequest,
        Option<BlockId>,
        Option<GethDebugTracingOptions>,
    ),
    ctx: StorageRpcCtx<H>,
) -> Result<GethTrace, DebugError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let opts = opts.ok_or(DebugError::InvalidTracerConfig)?;
    let _permit = ctx.acquire_tracing_permit().await;

    let id = block_id.unwrap_or(BlockId::latest());
    let span = tracing::debug_span!("traceCall", ?id, tracer = ?opts.tracer.as_ref());

    let fut = async move {
        use crate::config::EvmBlockContext;

        let EvmBlockContext { header, db, spec_id } =
            ctx.resolve_evm_block(id).map_err(|e| match e {
                crate::eth::EthError::BlockNotFound(id) => DebugError::BlockNotFound(id),
                other => DebugError::EvmHalt { reason: other.to_string() },
            })?;

        let mut evm = signet_evm::signet_evm(db, ctx.constants().clone());
        evm.set_spec_id(spec_id);
        let trevm = evm.fill_cfg(&CfgFiller(ctx.chain_id())).fill_block(&header);

        let trevm = trevm.fill_tx(&request);

        let tx_info = alloy::rpc::types::TransactionInfo {
            hash: None,
            index: None,
            block_hash: None,
            block_number: Some(header.number),
            base_fee: header.base_fee_per_gas(),
        };

        let res = crate::debug::tracer::trace(trevm, &opts, tx_info)?.0;

        Ok(res)
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        DebugError::EvmHalt { reason: "task panicked or cancelled".into() }
    )
}

/// `debug_getRawTransaction` handler.
///
/// Fetches the transaction by hash from cold storage and returns the
/// EIP-2718 encoded bytes.
pub(super) async fn get_raw_transaction<H>(
    hctx: HandlerCtx,
    (hash,): (B256,),
    ctx: StorageRpcCtx<H>,
) -> Result<Bytes, DebugError>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let span = tracing::debug_span!("getRawTransaction", %hash);

    let fut = async move {
        let confirmed = ctx
            .cold()
            .get_tx_by_hash(hash)
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, %hash, "cold storage read failed");
                DebugError::from(e)
            })?
            .ok_or(DebugError::TransactionNotFound(hash))?;

        let tx = confirmed.into_inner().into_inner();
        Ok(Bytes::from(tx.encoded_2718()))
    }
    .instrument(span);

    await_handler!(
        hctx.spawn(fut),
        DebugError::EvmHalt { reason: "task panicked or cancelled".into() }
    )
}
