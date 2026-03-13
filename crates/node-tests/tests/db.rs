use alloy::primitives::map::HashSet;
use serial_test::serial;
use signet_cold::mem::MemColdBackend;
use signet_host_reth::decompose_exex_context;
use signet_hot::{
    db::{HotDbRead, UnsafeDbWrite},
    mem::MemKv,
};
use signet_node::SignetNodeBuilder;
use signet_node_config::test_utils::test_config;
use signet_storage::{CancellationToken, HistoryRead, HistoryWrite, HotKv, UnifiedStorage};
use std::sync::{Arc, Mutex};

#[serial]
#[tokio::test]
async fn test_genesis() {
    let cfg = test_config();
    let consts = cfg.constants();
    let (ctx, _) = reth_exex_test_utils::test_exex_context().await.unwrap();

    let chain_spec: Arc<_> = cfg.chain_spec().clone();
    assert_eq!(chain_spec.genesis().config.chain_id, consts.unwrap().ru_chain_id());

    let decomposed = decompose_exex_context(ctx);

    let cancel_token = CancellationToken::new();
    let hot = MemKv::new();
    {
        let hardforks = signet_genesis::genesis_hardforks(cfg.genesis());
        let writer = hot.writer().unwrap();
        writer.load_genesis(cfg.genesis(), &hardforks).unwrap();
        writer.commit().unwrap();
    }

    let storage = Arc::new(UnifiedStorage::spawn(hot, MemColdBackend::new(), cancel_token.clone()));

    let blob_cacher = signet_blobber::BlobFetcher::builder()
        .with_config(cfg.block_extractor())
        .unwrap()
        .with_pool(decomposed.pool)
        .with_client(reqwest::Client::new())
        .build_cache()
        .unwrap()
        .spawn::<alloy::consensus::SimpleCoder>();

    let alias_oracle: Arc<Mutex<HashSet<_>>> = Arc::new(Mutex::new(HashSet::default()));

    let (_, _) = SignetNodeBuilder::new(cfg.clone())
        .with_notifier(decomposed.notifier)
        .with_storage(Arc::clone(&storage))
        .with_alias_oracle(Arc::clone(&alias_oracle))
        .with_blob_cacher(blob_cacher)
        .with_serve_config(decomposed.serve_config)
        .with_rpc_config(decomposed.rpc_config)
        .with_client(reqwest::Client::new())
        .build()
        .await
        .unwrap();

    let reader = storage.reader().unwrap();
    assert!(reader.has_block(0).unwrap());

    let header = reader.get_header(0).unwrap().expect("missing genesis header");
    let zero_hash = alloy::primitives::B256::ZERO;
    assert_eq!(header.parent_hash, zero_hash);
    assert_eq!(header.base_fee_per_gas, Some(0x3b9aca00));

    cancel_token.cancel();
}
