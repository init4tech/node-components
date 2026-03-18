use reth::transaction_pool::TransactionPool;
use signet_blobber::CacheHandle;
use signet_node_config::SignetNodeConfig;

/// Build a blob [`CacheHandle`] for test use from a config and transaction
/// pool.
///
/// Uses a fresh `reqwest::Client` and spawns with [`SimpleCoder`].
///
/// [`SimpleCoder`]: alloy::consensus::SimpleCoder
pub fn test_blob_cacher<Pool>(cfg: &SignetNodeConfig, pool: Pool) -> CacheHandle
where
    Pool: TransactionPool + 'static,
{
    signet_blobber::BlobFetcher::builder()
        .with_config(cfg.block_extractor())
        .unwrap()
        .with_pool(pool)
        .with_client(reqwest::Client::new())
        .build_cache()
        .unwrap()
        .spawn::<alloy::consensus::SimpleCoder>()
}
