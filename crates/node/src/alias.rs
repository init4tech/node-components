use alloy::{consensus::constants::KECCAK_EMPTY, primitives::Address};
use core::fmt;
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

impl AliasOracle for RethAliasOracle {
    fn should_alias(&self, address: Address) -> eyre::Result<bool> {
        // No account at this address.
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
        // NB: This becomes a problem if anyone ever birthday attacks a
        // contract/EOA pair (c.f. EIP-3607). In practice this is unlikely to
        // happen for the foreseeable future, and if it does we can revisit
        // this decision.
        // We considered taking the host height as an argument to this method,
        // but this would require all nodes to be archive nodes in order to
        // sync, which is less than ideal
        self.0
            .state_by_block_number_or_tag(alloy::eips::BlockNumberOrTag::Latest)
            .map(RethAliasOracle)
            .map_err(Into::into)
    }
}
