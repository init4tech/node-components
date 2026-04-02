use alloy::{consensus::constants::KECCAK_EMPTY, primitives::Address};
use core::{
    fmt,
    future::{self, Future},
};
use reth::providers::{StateProviderBox, StateProviderFactory};
use signet_block_processor::{AliasError, AliasOracle, AliasOracleFactory};

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
    fn check_alias(&self, address: Address) -> Result<bool, AliasError> {
        let Some(acct) =
            self.0.basic_account(&address).map_err(|e| AliasError::Internal(Box::new(e)))?
        else {
            return Ok(false);
        };
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
            .bytecode_by_hash(&bch)
            .map_err(|e| AliasError::Internal(Box::new(e)))?
            .ok_or_else(|| {
                AliasError::Internal("code not found. This indicates a corrupted database".into())
            })?;

        // If not a 7702 delegation contract, alias it.
        Ok(!code.is_eip7702())
    }
}

impl AliasOracle for RethAliasOracle {
    fn should_alias(
        &self,
        address: Address,
    ) -> impl Future<Output = Result<bool, AliasError>> + Send {
        future::ready(self.check_alias(address))
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

    fn create(&self) -> Result<Self::Oracle, AliasError> {
        // We use `Latest` rather than a pinned host height because pinning
        // would require every node to be an archive node, which is impractical.
        //
        // This is safe because alias status is stable across blocks: an EOA
        // cannot become a non-delegation contract without a birthday attack
        // (c.f. EIP-3607), and EIP-7702 delegations are excluded by
        // `is_eip7702()`. Even in the (computationally infeasible ~2^80)
        // birthday attack scenario, the result is a benign false-positive
        // (over-aliasing), never a dangerous false-negative.
        self.0
            .state_by_block_number_or_tag(alloy::eips::BlockNumberOrTag::Latest)
            .map(RethAliasOracle)
            .map_err(|e| AliasError::Internal(Box::new(e)))
    }
}
