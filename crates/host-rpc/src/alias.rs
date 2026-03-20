use alloy::{
    eips::eip7702::constants::EIP7702_DELEGATION_DESIGNATOR, primitives::Address,
    providers::Provider,
};
use signet_block_processor::{AliasOracle, AliasOracleFactory};

/// An [`AliasOracle`] backed by an alloy RPC [`Provider`].
///
/// Checks whether an address has non-delegation bytecode by fetching the
/// code at the address via `eth_getCode`. Addresses with no code or with
/// EIP-7702 delegation code are not aliased; addresses with any other
/// bytecode are.
#[derive(Debug, Clone)]
pub struct RpcAliasOracle<P> {
    provider: P,
}

impl<P: Provider + Clone + 'static> AliasOracle for RpcAliasOracle<P> {
    async fn should_alias(&self, address: Address) -> eyre::Result<bool> {
        let code = self.provider.get_code_at(address).await?;
        // No code — not a contract.
        if code.is_empty() {
            return Ok(false);
        }
        // EIP-7702 delegation — do not alias.
        if code.starts_with(&EIP7702_DELEGATION_DESIGNATOR) {
            return Ok(false);
        }
        // Non-delegation contract — alias it.
        Ok(true)
    }
}

/// An [`AliasOracleFactory`] backed by an alloy RPC [`Provider`].
///
/// Creates [`RpcAliasOracle`] instances that query the host chain at
/// `latest`. This is safe because alias status is stable across blocks:
/// an EOA cannot become a non-delegation contract without a birthday
/// attack (c.f. EIP-3607), and EIP-7702 delegations are excluded by the
/// delegation designator check. Even in the (computationally infeasible
/// ~2^80) birthday attack scenario, the result is a benign
/// false-positive (over-aliasing), never a dangerous false-negative.
#[derive(Debug, Clone)]
pub struct RpcAliasOracleFactory<P> {
    provider: P,
}

impl<P> RpcAliasOracleFactory<P> {
    /// Create a new [`RpcAliasOracleFactory`] from an alloy provider.
    pub const fn new(provider: P) -> Self {
        Self { provider }
    }
}

impl<P: Provider + Clone + 'static> AliasOracleFactory for RpcAliasOracleFactory<P> {
    type Oracle = RpcAliasOracle<P>;

    fn create(&self) -> eyre::Result<Self::Oracle> {
        Ok(RpcAliasOracle { provider: self.provider.clone() })
    }
}
