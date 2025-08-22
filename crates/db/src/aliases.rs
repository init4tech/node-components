use reth::providers::DatabaseProviderRW;
use signet_node_types::SignetNodeTypes;

/// A Convenience alias for a [`DatabaseProviderRW`] using [`SignetNodeTypes`].
pub type SignetDbRw<Db> = DatabaseProviderRW<Db, SignetNodeTypes<Db>>;
