use std::sync::Arc;

use crate::{
    RpcCtx,
    inspect::db::{DbArgs, ListTableViewer},
    utils::{await_handler, response_tri},
};
use ajj::{HandlerCtx, ResponsePayload};
use reth::providers::providers::ProviderNodeTypes;
use reth_db::mdbx;
use reth_node_api::FullNodeComponents;
use signet_node_types::Pnt;

/// Handler for the `db` endpoint in the `inspect` module.
pub(super) async fn db<Host, Signet>(
    hctx: HandlerCtx,
    args: DbArgs,
    ctx: RpcCtx<Host, Signet>,
) -> ResponsePayload<Box<serde_json::value::RawValue>, String>
where
    Host: FullNodeComponents,
    Signet: Pnt + ProviderNodeTypes<DB = Arc<mdbx::DatabaseEnv>>,
{
    let task = async move {
        let table: reth_db::Tables = response_tri!(args.table(), "invalid table name");

        let viewer = ListTableViewer::new(ctx.signet().factory(), &args);

        response_tri!(table.view(&viewer), "Failed to view table");

        let Some(output) = viewer.take_output() else {
            return ResponsePayload::internal_error_message(
                "No output generated. The task may have panicked or been cancelled. This is a bug, please report it.".into(),
            );
        };

        ResponsePayload::Success(output)
    };

    await_handler!(@response_option hctx.spawn_blocking(task))
}
