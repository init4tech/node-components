pub(crate) mod db;

mod endpoints;

use std::sync::Arc;

use crate::RpcCtx;
use reth::providers::providers::ProviderNodeTypes;
use reth_db::mdbx;
use reth_node_api::FullNodeComponents;
use signet_node_types::Pnt;

/// Instantiate the `inspect` API router.
pub fn inspect<Host, Signet>() -> ajj::Router<RpcCtx<Host, Signet>>
where
    Host: FullNodeComponents,
    Signet: Pnt + ProviderNodeTypes<DB = Arc<mdbx::DatabaseEnv>>,
{
    ajj::Router::new().route("db", endpoints::db::<Host, Signet>)
}
