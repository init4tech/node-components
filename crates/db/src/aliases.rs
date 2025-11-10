use reth::{
    providers::{DatabaseProviderRW, StateProviderBox},
    revm::database::StateProviderDatabase,
};
use signet_node_types::SignetNodeTypes;
use trevm::revm::database::State;

/// A Convenience alias for a [`DatabaseProviderRW`] using [`SignetNodeTypes`].
pub type SignetDbRw<Db> = DatabaseProviderRW<Db, SignetNodeTypes<Db>>;

/// Type alias for EVMs using a [`StateProviderBox`] as the `DB` type for
/// trevm.
///
/// [`StateProviderBox`]: reth::providers::StateProviderBox
pub type RuRevmState = State<StateProviderDatabase<StateProviderBox>>;
