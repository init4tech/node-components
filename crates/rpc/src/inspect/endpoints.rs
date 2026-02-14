use crate::{
    inspect::db::{DbArgs, ListTableViewer},
    utils::{await_handler, response_tri},
};
use ajj::{HandlerCtx, ResponsePayload};
use reth::providers::{ProviderFactory, providers::ProviderNodeTypes};
use reth_db::mdbx;
use signet_node_types::Pnt;
use std::sync::Arc;

/// Handler for the `db` endpoint in the `inspect` module.
pub(super) async fn db<Signet>(
    hctx: HandlerCtx,
    args: DbArgs,
    ctx: ProviderFactory<Signet>,
) -> ResponsePayload<Box<serde_json::value::RawValue>, String>
where
    Signet: Pnt + ProviderNodeTypes<DB = Arc<mdbx::DatabaseEnv>>,
{
    let task = async move {
        let table: reth_db::Tables = response_tri!(args.table(), "invalid table name");

        let viewer = ListTableViewer::new(&ctx, &args);

        response_tri!(table.view(&viewer), "Failed to view table");

        let Some(output) = viewer.take_output() else {
            return ResponsePayload::internal_error_message(
                "No output generated. The task may have panicked or been cancelled. This is a bug, please report it.".into(),
            );
        };

        ResponsePayload(Ok(output))
    };

    await_handler!(@response_option hctx.spawn_blocking(task))
}
