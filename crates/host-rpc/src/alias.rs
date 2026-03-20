use alloy::{
    eips::eip7702::constants::EIP7702_DELEGATION_DESIGNATOR,
    primitives::{Address, map::HashSet},
    providers::Provider,
};
use signet_block_processor::{AliasOracle, AliasOracleFactory};
use std::sync::{Arc, RwLock};

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

impl<P: Provider + Clone + 'static> AliasOracle for RpcAliasOracle<P> {
    async fn should_alias(&self, address: Address) -> eyre::Result<bool> {
        // Check cache first — if we've seen this address as a contract, skip RPC.
        if self.cache.read().expect("cache poisoned").contains(&address) {
            return Ok(true);
        }

        let code = self.provider.get_code_at(address).await?;
        // No code — not a contract.
        if code.is_empty() {
            return Ok(false);
        }
        // EIP-7702 delegation — do not alias.
        if code.starts_with(&EIP7702_DELEGATION_DESIGNATOR) {
            return Ok(false);
        }
        // Non-delegation contract — cache and alias it.
        self.cache.write().expect("cache poisoned").insert(address);
        Ok(true)
    }
}

impl<P: Provider + Clone + 'static> AliasOracleFactory for RpcAliasOracle<P> {
    type Oracle = Self;

    fn create(&self) -> eyre::Result<Self::Oracle> {
        Ok(self.clone())
    }
}
