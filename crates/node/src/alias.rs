use alloy::{consensus::constants::KECCAK_EMPTY, primitives::Address};
use core::{
    fmt,
    future::{self, Future},
};
use eyre::OptionExt;
use reth::providers::{StateProviderBox, StateProviderFactory};
use signet_block_processor::{AliasOracle, AliasOracleFactory};

/// An [`AliasOracle`] backed by a reth [`StateProviderBox`].
///
/// Checks whether an address has non-delegation bytecode, indicating it
/// should be aliased during transaction processing.
pub struct RethAliasOracle(StateProviderBox);

impl fmt::Debug for RethAliasOracle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RethAliasOracle").finish_non_exhaustive()
    }
}

impl RethAliasOracle {
    /// Synchronously check whether the given address should be aliased.
    fn check_alias(&self, address: Address) -> eyre::Result<bool> {
        let Some(acct) = self.0.basic_account(&address)? else { return Ok(false) };
        // Get the bytecode hash for this account.
        let bch = match acct.bytecode_hash {
            Some(hash) => hash,
            // No bytecode hash; not a contract.
            None => return Ok(false),
        };
        // No code at this address.
        if bch == KECCAK_EMPTY {
            return Ok(false);
        }
        // Fetch the code associated with this bytecode hash.
        let code = self
            .0
            .bytecode_by_hash(&bch)?
            .ok_or_eyre("code not found. This indicates a corrupted database")?;

        // If not a 7702 delegation contract, alias it.
        Ok(!code.is_eip7702())
    }
}

impl AliasOracle for RethAliasOracle {
    fn should_alias(&self, address: Address) -> impl Future<Output = eyre::Result<bool>> + Send {
        let result = self.check_alias(address);
        future::ready(result)
    }
}

/// An [`AliasOracleFactory`] backed by a `Box<dyn StateProviderFactory>`.
///
/// Creates [`RethAliasOracle`] instances from the latest host chain state.
pub struct RethAliasOracleFactory(Box<dyn StateProviderFactory>);

impl fmt::Debug for RethAliasOracleFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RethAliasOracleFactory").finish_non_exhaustive()
    }
}

impl RethAliasOracleFactory {
    /// Create a new [`RethAliasOracleFactory`] from a boxed state provider
    /// factory.
    pub fn new(provider: Box<dyn StateProviderFactory>) -> Self {
        Self(provider)
    }
}

impl AliasOracleFactory for RethAliasOracleFactory {
    type Oracle = RethAliasOracle;

    fn create(&self) -> eyre::Result<Self::Oracle> {
        // ## Why `Latest` instead of a pinned host height
        //
        // We use `Latest` rather than pinning to a specific host block
        // height because pinning would require every node to be an archive
        // node in order to sync historical state, which is impractical.
        //
        // ## Why `Latest` is safe
        //
        // An EOA cannot become a non-delegation contract without a birthday
        // attack (c.f. EIP-3607). CREATE and CREATE2 addresses are
        // deterministic and cannot target an existing EOA. EIP-7702
        // delegations are explicitly excluded by the `is_eip7702()` check
        // in `should_alias`, so delegated EOAs are never aliased. This
        // means the alias status of an address is stable across blocks
        // under normal conditions, making `Latest` equivalent to any
        // pinned height.
        //
        // ## The only risk: birthday attacks
        //
        // A birthday attack could produce a CREATE/CREATE2 collision with
        // an existing EOA, causing `should_alias` to return a false
        // positive. This is computationally infeasible for the foreseeable
        // future (~2^80 work), and if it ever becomes practical we can
        // revisit this decision.
        //
        // ## Over-aliasing vs under-aliasing
        //
        // Even in the birthday attack scenario, the result is
        // over-aliasing (a false positive), which is benign: a transaction
        // sender gets an aliased address when it shouldn't. The dangerous
        // failure mode — under-aliasing — cannot occur here because
        // contract bytecode is never removed once deployed.
        self.0
            .state_by_block_number_or_tag(alloy::eips::BlockNumberOrTag::Latest)
            .map(RethAliasOracle)
            .map_err(Into::into)
    }
}
