use alloy::{
    consensus::constants::KECCAK_EMPTY,
    primitives::{Address, map::HashSet},
};
use eyre::OptionExt;
#[cfg(doc)]
use reth::providers::StateProvider;
use reth::providers::{StateProviderBox, StateProviderFactory};
use std::sync::{Arc, Mutex};

/// Simple trait to allow checking if an address should be aliased.
pub trait AliasOracle {
    /// Returns true if the given address is an alias.
    fn should_alias(&self, address: Address) -> eyre::Result<bool>;
}

/// Default implementation of [`AliasOracle`] for any type implementing
/// [`StateProvider`]. This implementation checks if the address has bytecode
/// associated with it, and if so, whether that bytecode matches the pattern
/// for a 7702 delegation contract. If it is a delegation contract, it is not
/// aliased; otherwise, it is aliased.
impl AliasOracle for StateProviderBox {
    fn should_alias(&self, address: Address) -> eyre::Result<bool> {
        // No account at this address.
        let Some(acct) = self.basic_account(&address)? else { return Ok(false) };
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
            .bytecode_by_hash(&bch)?
            .ok_or_eyre("code not found. This indicates a corrupted database")?;

        // If not a 7702 delegation contract, alias it.
        Ok(!code.is_eip7702())
    }
}

impl AliasOracle for HashSet<Address> {
    fn should_alias(&self, address: Address) -> eyre::Result<bool> {
        Ok(self.contains(&address))
    }
}

/// Factory trait to create new [`AliasOracle`] instances.
///
/// The default implementation on `Box<dyn StateProviderFactory>` creates
/// [`AliasOracle`] instances backed by the state provider for a given block
/// height. It will error if that state provider cannot be obtained.
///
/// This trait is primarily intended to allow injecting test implementations
/// of [`AliasOracle`] into the Signet Node for testing purposes. The test
/// implementation on [`HashSet<Address>`] allows specifying a fixed set of
/// addresses to be aliased.
pub trait AliasOracleFactory: Send + Sync + 'static {
    /// The [`AliasOracle`] type.
    type Oracle: AliasOracle;

    /// Create a new [`AliasOracle`].
    fn create(&self) -> eyre::Result<Self::Oracle>;
}

impl AliasOracleFactory for Box<dyn StateProviderFactory> {
    type Oracle = StateProviderBox;

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
        self.state_by_block_number_or_tag(alloy::eips::BlockNumberOrTag::Latest).map_err(Into::into)
    }
}

/// This implementation is primarily for testing purposes.
impl AliasOracleFactory for HashSet<Address> {
    type Oracle = HashSet<Address>;

    fn create(&self) -> eyre::Result<Self::Oracle> {
        Ok(self.clone())
    }
}

impl<T> AliasOracleFactory for Mutex<T>
where
    T: AliasOracleFactory,
{
    type Oracle = T::Oracle;

    fn create(&self) -> eyre::Result<Self::Oracle> {
        let guard =
            self.lock().map_err(|_| eyre::eyre!("failed to lock AliasOracleFactory mutex"))?;
        guard.create()
    }
}

impl<T> AliasOracleFactory for Arc<T>
where
    T: AliasOracleFactory,
{
    type Oracle = T::Oracle;

    fn create(&self) -> eyre::Result<Self::Oracle> {
        self.as_ref().create()
    }
}
