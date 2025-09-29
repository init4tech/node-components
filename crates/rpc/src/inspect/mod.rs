pub(crate) mod db;

mod endpoints;

use reth::providers::{ProviderFactory, providers::ProviderNodeTypes};
use reth_db::mdbx;
use signet_node_types::Pnt;
use std::sync::Arc;

/// Instantiate the `inspect` API router.
pub fn inspect<Signet>() -> ajj::Router<ProviderFactory<Signet>>
where
    Signet: Pnt + ProviderNodeTypes<DB = Arc<mdbx::DatabaseEnv>>,
{
    ajj::Router::new().route("db", endpoints::db::<Signet>)
}
