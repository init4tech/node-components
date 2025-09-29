use reth::providers::DatabaseProviderRW;
use signet_node_types::SignetNodeTypes;

/// A Convenience alias for a [`DatabaseProviderRW`] using [`SignetNodeTypes`].
pub type SignetDbRw<Db> = DatabaseProviderRW<Db, SignetNodeTypes<Db>>;

/// Type alias for EVMs using a [`StateProviderBox`] as the `DB` type for
/// trevm.
///
/// [`StateProviderBox`]: reth::providers::StateProviderBox
pub type RuRevmState = trevm::revm::database::State<
    reth::revm::database::StateProviderDatabase<reth::providers::StateProviderBox>,
>;
