//! Signet RPC methods and related code.

mod endpoints;
use endpoints::*;

pub(crate) mod error;

use crate::ctx::RpcCtx;
use reth_node_api::FullNodeComponents;
use signet_node_types::Pnt;

/// Instantiate a `signet` API router.
pub fn signet<Host, Signet>() -> ajj::Router<RpcCtx<Host, Signet>>
where
    Host: FullNodeComponents,
    Signet: Pnt,
{
    ajj::Router::new().route("sendOrder", send_order).route("callBundle", call_bundle)
}
