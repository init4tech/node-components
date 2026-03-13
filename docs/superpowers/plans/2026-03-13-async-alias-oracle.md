# Async AliasOracle Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `AliasOracle::should_alias` async using RPITIT to support future async implementations.

**Architecture:** Change the `AliasOracle` trait's `should_alias` method to return `impl Future<Output = eyre::Result<bool>> + Send`. Existing sync implementations wrap their bodies in `async move { ... }`. The single call site in `processor.rs` adds `.await`.

**Tech Stack:** Rust RPITIT (stable since 1.75), `core::future::Future`

---

## Chunk 1: Implementation

### Task 1: Update all AliasOracle impls and call site

All changes are made together in a single commit so no intermediate state breaks compilation.

**Files:**
- Modify: `crates/block-processor/src/alias.rs:1-14`
- Modify: `crates/host-reth/src/alias.rs:1-42`
- Modify: `crates/block-processor/src/v1/processor.rs:106-109,188-195`

- [ ] **Step 1: Update the trait definition and HashSet impl**

In `crates/block-processor/src/alias.rs`, add `Future` import and make `should_alias` return an async future:

```rust
use alloy::primitives::{Address, map::HashSet};
use core::future::Future;
use std::sync::{Arc, Mutex};

/// Simple trait to allow checking if an address should be aliased.
pub trait AliasOracle {
    /// Returns true if the given address is an alias.
    fn should_alias(&self, address: Address) -> impl Future<Output = eyre::Result<bool>> + Send;
}

impl AliasOracle for HashSet<Address> {
    fn should_alias(&self, address: Address) -> impl Future<Output = eyre::Result<bool>> + Send {
        let result = Ok(self.contains(&address));
        async move { result }
    }
}
```

Note: the `HashSet` impl computes the result synchronously _before_ the async block, so `&self` is not captured across the await point. This avoids requiring `Self: Sync`.

- [ ] **Step 2: Update RethAliasOracle impl**

In `crates/host-reth/src/alias.rs`, add `use core::future::Future;` to imports, then change the impl:

```rust
impl AliasOracle for RethAliasOracle {
    fn should_alias(&self, address: Address) -> impl Future<Output = eyre::Result<bool>> + Send {
        // Compute synchronously, then wrap in async.
        let result = (|| {
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
        })();
        async move { result }
    }
}
```

- [ ] **Step 3: Update processor call site**

In `crates/block-processor/src/v1/processor.rs`, update the helper method (line 106-109):

```rust
/// Check if the given address should be aliased.
async fn should_alias(&self, address: Address) -> eyre::Result<bool> {
    self.alias_oracle.create()?.should_alias(address).await
}
```

Update the loop in `run_evm` (line 192):

```rust
if !to_alias.contains(&addr) && self.should_alias(addr).await? {
```

- [ ] **Step 4: Format**

Run: `cargo +nightly fmt`

- [ ] **Step 5: Lint all affected crates**

Run: `cargo clippy -p signet-block-processor --all-features --all-targets`
Run: `cargo clippy -p signet-host-reth --all-features --all-targets`
Expected: PASS

- [ ] **Step 6: Run tests**

Run: `cargo t -p signet-block-processor`
Run: `cargo t -p signet-host-reth`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add crates/block-processor/src/alias.rs crates/host-reth/src/alias.rs crates/block-processor/src/v1/processor.rs
git commit -m "refactor: make AliasOracle::should_alias async via RPITIT"
```

### Task 2: Full workspace validation

- [ ] **Step 1: Lint downstream crates**

Run: `cargo clippy -p signet-node --all-features --all-targets`
Run: `cargo clippy -p signet-node-tests --all-features --all-targets`
Expected: PASS (bounds are on `AliasOracleFactory`, not `AliasOracle` directly)

- [ ] **Step 2: Run node-tests**

Run: `cargo t -p signet-node-tests`
Expected: PASS

- [ ] **Step 3: Final format check**

Run: `cargo +nightly fmt`
