pub(crate) mod db;

mod endpoints;

use crate::RpcCtx;
use reth_node_api::FullNodeComponents;
use signet_node_types::Pnt;

/// Instantiate the `inspect` API router.
pub fn inspect<Host, Signet>() -> ajj::Router<RpcCtx<Host, Signet>>
where
    Host: FullNodeComponents,
    Signet: Pnt,
{
    ajj::Router::new().route("db", endpoints::db::<Host, Signet>)
}
