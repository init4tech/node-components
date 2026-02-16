//! Cold-storage gas oracle for computing gas price suggestions.
//!
//! Reads recent block headers and transactions from cold storage to
//! compute a suggested tip cap based on recent transaction activity.
//! Behavior mirrors reth's `GasPriceOracle`: a configurable default
//! price when no transactions exist, an `ignore_price` floor, and a
//! `max_price` cap.

use crate::config::{GasOracleCache, StorageRpcConfig};
use alloy::{consensus::Transaction, primitives::U256};
use signet_cold::{ColdStorageError, ColdStorageReadHandle, HeaderSpecifier};

/// Suggest a tip cap based on recent transaction tips.
///
/// Reads the last `gas_oracle_block_count` blocks from cold storage,
/// computes the effective tip per gas for each transaction, filters
/// tips below `ignore_price`, sorts the remainder, and returns the
/// value at `gas_oracle_percentile`, clamped to `max_price`.
///
/// Returns `default_gas_price` (default 1 Gwei) when no qualifying
/// transactions are found.
///
/// Uses the provided `cache` to avoid redundant cold storage reads
/// when the tip has already been computed for the current block.
pub(crate) async fn suggest_tip_cap(
    cold: &ColdStorageReadHandle,
    latest: u64,
    config: &StorageRpcConfig,
    cache: &GasOracleCache,
) -> Result<U256, ColdStorageError> {
    // Check cache first — return early on hit.
    if let Some(cached_tip) = cache.get(latest) {
        return Ok(cached_tip);
    }

    let block_count = config.gas_oracle_block_count.min(latest + 1);
    let start = latest.saturating_sub(block_count - 1);

    let specs: Vec<_> = (start..=latest).map(HeaderSpecifier::Number).collect();
    let headers = cold.get_headers(specs).await?;

    // Collect blocks that have headers, then read their transactions
    // in parallel to avoid sequential cold-storage round-trips.
    let blocks_with_fees: Vec<_> = headers
        .into_iter()
        .enumerate()
        .filter_map(|(offset, h)| {
            h.map(|header| (start + offset as u64, header.base_fee_per_gas.unwrap_or_default()))
        })
        .collect();

    let mut join_set = tokio::task::JoinSet::new();
    for (block_num, base_fee) in &blocks_with_fees {
        let cold = cold.clone();
        let block_num = *block_num;
        let base_fee = *base_fee;
        join_set.spawn(async move {
            cold.get_transactions_in_block(block_num).await.map(|txs| (txs, base_fee))
        });
    }

    let mut all_tips: Vec<u128> = Vec::new();
    while let Some(result) = join_set.join_next().await {
        let (txs, base_fee) = result.expect("tx read task panicked")?;
        for tx in &txs {
            if let Some(tip) = tx.effective_tip_per_gas(base_fee)
                && config.ignore_price.is_none_or(|floor| tip >= floor)
            {
                all_tips.push(tip);
            }
        }
    }

    if all_tips.is_empty() {
        let default_price = config.default_gas_price.map_or(U256::ZERO, U256::from);
        cache.set(latest, default_price);
        return Ok(default_price);
    }

    all_tips.sort_unstable();

    let index = ((config.gas_oracle_percentile / 100.0) * (all_tips.len() - 1) as f64) as usize;
    let index = index.min(all_tips.len() - 1);

    let mut price = U256::from(all_tips[index]);

    if let Some(max) = config.max_price {
        price = price.min(U256::from(max));
    }

    cache.set(latest, price);

    Ok(price)
}
