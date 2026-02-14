//! Cold-storage gas oracle for computing gas price suggestions.
//!
//! Reads recent block headers and transactions from cold storage to
//! compute a suggested tip cap based on recent transaction activity.

use alloy::{consensus::Transaction, primitives::U256};
use signet_cold::{ColdStorageError, ColdStorageReadHandle, HeaderSpecifier};

use crate::config::StorageRpcConfig;

/// Suggest a tip cap based on recent transaction tips.
///
/// Reads the last `gas_oracle_block_count` blocks from cold storage,
/// computes the effective tip per gas for each transaction, sorts all
/// tips, and returns the value at `gas_oracle_percentile`.
///
/// Returns `U256::ZERO` if no transactions are found in the range.
pub(crate) async fn suggest_tip_cap(
    cold: &ColdStorageReadHandle,
    latest: u64,
    config: &StorageRpcConfig,
) -> Result<U256, ColdStorageError> {
    let block_count = config.gas_oracle_block_count.min(latest + 1);
    let start = latest.saturating_sub(block_count - 1);

    let specs: Vec<_> = (start..=latest).map(HeaderSpecifier::Number).collect();
    let headers = cold.get_headers(specs).await?;

    let mut all_tips: Vec<u128> = Vec::new();

    for (offset, maybe_header) in headers.into_iter().enumerate() {
        let Some(header) = maybe_header else { continue };
        let base_fee = header.base_fee_per_gas.unwrap_or_default();
        let block_num = start + offset as u64;

        let txs = cold.get_transactions_in_block(block_num).await?;

        for tx in &txs {
            if let Some(tip) = tx.effective_tip_per_gas(base_fee) {
                all_tips.push(tip);
            }
        }
    }

    if all_tips.is_empty() {
        return Ok(U256::ZERO);
    }

    all_tips.sort_unstable();

    let index = ((config.gas_oracle_percentile / 100.0) * (all_tips.len() - 1) as f64) as usize;
    let index = index.min(all_tips.len() - 1);

    Ok(U256::from(all_tips[index]))
}
