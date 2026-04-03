use alloy::{
    eips::eip7702::constants::EIP7702_DELEGATION_DESIGNATOR,
    primitives::{Address, map::HashSet},
    providers::Provider,
};
use signet_block_processor::{AliasError, AliasOracle, AliasOracleFactory};
use std::sync::{Arc, RwLock};
use tracing::{debug, instrument};

/// EIP-7702 delegation bytecode is exactly 23 bytes: 3-byte designator +
/// 20-byte address.
const EIP7702_DELEGATION_LEN: usize = 23;

/// An RPC-backed [`AliasOracle`] and [`AliasOracleFactory`].
///
/// Checks whether an address has non-delegation bytecode by fetching the
/// code at the address via `eth_getCode`. Addresses with no code or with
/// EIP-7702 delegation code are not aliased; addresses with any other
/// bytecode are.
///
/// Positive results (address is a contract) are cached in a shared set.
/// Contract status is permanent — an address cannot stop being a
/// contract — so cached positives never go stale. All clones (including
/// those produced by [`AliasOracleFactory::create`]) share the same
/// cache.
///
/// Querying at `latest` is safe because alias status is stable across
/// blocks: an EOA cannot become a non-delegation contract without a
/// birthday attack (c.f. EIP-3607), and EIP-7702 delegations are
/// excluded by the delegation designator check. Even in the
/// (computationally infeasible ~2^80) birthday attack scenario, the
/// result is a benign false-positive (over-aliasing), never a dangerous
/// false-negative.
#[derive(Debug, Clone)]
pub struct RpcAliasOracle<P> {
    provider: P,
    /// Shared cache of addresses known to be non-delegation contracts.
    cache: Arc<RwLock<HashSet<Address>>>,
}

impl<P> RpcAliasOracle<P> {
    /// Create a new [`RpcAliasOracle`] from an alloy provider.
    pub fn new(provider: P) -> Self {
        Self { provider, cache: Arc::new(RwLock::new(HashSet::default())) }
    }
}

/// Classify bytecode: returns `true` if the code belongs to a
/// non-delegation contract that should be aliased.
fn should_alias_bytecode(code: &[u8]) -> bool {
    if code.is_empty() {
        return false;
    }
    // EIP-7702 delegation: exactly 23 bytes starting with the designator.
    if code.len() == EIP7702_DELEGATION_LEN && code.starts_with(&EIP7702_DELEGATION_DESIGNATOR) {
        return false;
    }
    true
}

impl<P: Provider + Clone + 'static> AliasOracle for RpcAliasOracle<P> {
    #[instrument(skip(self), fields(%address))]
    async fn should_alias(&self, address: Address) -> Result<bool, AliasError> {
        // NOTE: `std::sync::RwLock` is safe here because guards are always
        // dropped before the `.await` point. Do not hold a guard across the
        // `get_code_at` call — it will deadlock on single-threaded runtimes.

        // Check cache first — if we've seen this address as a contract, skip RPC.
        if self.cache.read().expect("cache poisoned").contains(&address) {
            debug!("cache hit");
            return Ok(true);
        }

        let code = self
            .provider
            .get_code_at(address)
            .await
            .map_err(|e| AliasError::Internal(Box::new(e)))?;
        let alias = should_alias_bytecode(&code);
        debug!(code_len = code.len(), alias, "resolved");

        if alias {
            self.cache.write().expect("cache poisoned").insert(address);
        }

        Ok(alias)
    }
}

impl<P: Provider + Clone + 'static> AliasOracleFactory for RpcAliasOracle<P> {
    type Oracle = Self;

    fn create(&self) -> Result<Self::Oracle, AliasError> {
        Ok(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::eips::eip7702::constants::EIP7702_CLEARED_DELEGATION;

    #[test]
    fn empty_code_is_not_aliased() {
        assert!(!should_alias_bytecode(&[]));
    }

    #[test]
    fn valid_delegation_is_not_aliased() {
        // 3-byte designator + 20-byte address = 23 bytes
        let mut delegation = [0u8; 23];
        delegation[..3].copy_from_slice(&EIP7702_DELEGATION_DESIGNATOR);
        delegation[3..].copy_from_slice(&[0xAB; 20]);
        assert!(!should_alias_bytecode(&delegation));
    }

    #[test]
    fn cleared_delegation_is_not_aliased() {
        assert!(!should_alias_bytecode(&EIP7702_CLEARED_DELEGATION));
    }

    #[test]
    fn contract_bytecode_is_aliased() {
        // Typical contract: starts with PUSH, not 0xef
        assert!(should_alias_bytecode(&[0x60, 0x80, 0x60, 0x40, 0x52]));
    }

    #[test]
    fn short_ef_prefix_is_aliased() {
        // Only 3 bytes starting with the designator — not a valid 23-byte
        // delegation, so it should be treated as a contract.
        assert!(should_alias_bytecode(&EIP7702_DELEGATION_DESIGNATOR));
    }

    #[test]
    fn long_ef_prefix_is_aliased() {
        // 24 bytes starting with the designator — too long to be a
        // delegation, so it should be treated as a contract.
        let mut long = [0u8; 24];
        long[..3].copy_from_slice(&EIP7702_DELEGATION_DESIGNATOR);
        assert!(should_alias_bytecode(&long));
    }
}
