mod signet;
pub use signet::SignetCtx;

mod full;
pub use full::RpcCtx;

mod fee_hist;

/// Type alias for EVMs using a [`StateProviderBox`] as the `DB` type for
/// trevm.
///
/// [`StateProviderBox`]: reth::providers::StateProviderBox
pub type RuRevmState = trevm::revm::database::State<
    reth::revm::database::StateProviderDatabase<reth::providers::StateProviderBox>,
>;
