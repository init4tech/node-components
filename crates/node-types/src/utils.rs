use reth::{primitives::EthPrimitives, providers::providers::ProviderNodeTypes};
use reth_chainspec::ChainSpec;
use reth_db::mdbx;
use std::sync::Arc;

/// Convenience trait for specifying the [`ProviderNodeTypes`] implementation
/// required for Signet functionality. This is used to condense many trait
/// bounds.
pub trait Pnt:
    ProviderNodeTypes<ChainSpec = ChainSpec, Primitives = EthPrimitives, DB = Arc<mdbx::DatabaseEnv>>
{
}

impl<T> Pnt for T where
    T: ProviderNodeTypes<
            ChainSpec = ChainSpec,
            Primitives = EthPrimitives,
            DB = Arc<mdbx::DatabaseEnv>,
        >
{
}

/// Convenience trait to aggregate the DB requirements
pub trait NodeTypesDbTrait:
    reth_db::database::Database + reth_db::database_metrics::DatabaseMetrics + Clone + Unpin + 'static
{
}

impl<T> NodeTypesDbTrait for T where
    T: reth_db::database::Database
        + reth_db::database_metrics::DatabaseMetrics
        + Clone
        + Unpin
        + 'static
{
}
