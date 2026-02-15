use crate::{
    ctx::RpcCtx,
    signet::error::SignetError,
    utils::{await_handler, response_tri},
};
use ajj::{HandlerCtx, ResponsePayload};
use alloy::{eips::BlockId, primitives::B256};
use reth::providers::{BlockHashReader, BlockNumReader};
use reth_node_api::FullNodeComponents;
use serde::Serialize;
use signet_bundle::{SignetBundleDriver, SignetCallBundle, SignetCallBundleResponse};
use signet_node_types::Pnt;
use signet_types::SignedOrder;
use std::time::Duration;
use tokio::select;

/// Signet network status information.
///
/// This provides Signet-specific network status, as opposed to the host
/// network status which would be returned by `eth_protocolVersion`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignetNetworkStatus {
    /// The Signet chain ID.
    pub chain_id: u64,
    /// The genesis block hash.
    pub genesis: B256,
    /// The current head block hash.
    pub head: B256,
    /// The current head block number.
    pub head_number: u64,
}

/// Returns the Signet network status including genesis and head block info.
///
/// This endpoint provides Signet-specific network information that reflects
/// the rollup's state rather than the underlying host network.
pub(super) async fn network_status<Host, Signet>(
    hctx: HandlerCtx,
    ctx: RpcCtx<Host, Signet>,
) -> Result<SignetNetworkStatus, String>
where
    Host: FullNodeComponents,
    Signet: Pnt,
{
    let task = async move {
        let provider = ctx.signet().provider();

        // Get the genesis block hash (block 0)
        let genesis = provider
            .block_hash(0)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "genesis block hash not found".to_string())?;

        // Get the current head block number and hash
        let head_number = provider.last_block_number().map_err(|e| e.to_string())?;

        let head = provider
            .block_hash(head_number)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "head block hash not found".to_string())?;

        // Get the chain ID from constants
        let chain_id = ctx.signet().constants().ru_chain_id();

        Ok(SignetNetworkStatus { chain_id, genesis, head, head_number })
    };

    await_handler!(@option hctx.spawn_blocking(task))
}

pub(super) async fn send_order<Host, Signet>(
    hctx: HandlerCtx,
    order: SignedOrder,
    ctx: RpcCtx<Host, Signet>,
) -> Result<(), String>
where
    Host: FullNodeComponents,
    Signet: Pnt,
{
    let task = |hctx: HandlerCtx| async move {
        let Some(tx_cache) = ctx.signet().tx_cache() else {
            return Err(SignetError::TxCacheUrlNotProvided.into_string());
        };

        hctx.spawn(async move { tx_cache.forward_order(order).await.map_err(|e| e.to_string()) });

        Ok(())
    };

    await_handler!(@option hctx.spawn_blocking_with_ctx(task))
}

pub(super) async fn call_bundle<Host, Signet>(
    hctx: HandlerCtx,
    bundle: SignetCallBundle,
    ctx: RpcCtx<Host, Signet>,
) -> ResponsePayload<SignetCallBundleResponse, String>
where
    Host: FullNodeComponents,
    Signet: Pnt,
{
    let timeout = bundle.bundle.timeout.unwrap_or(1000);

    let task = async move {
        let id = bundle.state_block_number();
        let block_cfg = match ctx.signet().block_cfg(id.into()).await {
            Ok(block_cfg) => block_cfg,
            Err(e) => {
                return ResponsePayload::internal_error_with_message_and_obj(
                    "error while loading block cfg".into(),
                    e.to_string(),
                );
            }
        };

        let mut driver = SignetBundleDriver::from(&bundle);

        let trevm = response_tri!(ctx.trevm(id.into(), &block_cfg));

        response_tri!(trevm.drive_bundle(&mut driver).map_err(|e| e.into_error()));

        ResponsePayload::Success(driver.into_response())
    };

    let task = async move {
        select! {
            _ = tokio::time::sleep(Duration::from_millis(timeout)) => {
                ResponsePayload::internal_error_message(
                    "timeout during bundle simulation".into(),
                )
            }
            result = task => {
                result
            }
        }
    };

    await_handler!(@response_option hctx.spawn_blocking(task))
}
