use serial_test::serial;
use signet_cold::mem::MemColdBackend;
use signet_hot::{
    db::{HotDbRead, UnsafeDbWrite},
    mem::MemKv,
};
use signet_node::SignetNodeBuilder;
use signet_node_config::test_utils::test_config;
use signet_node_tests::TestHostNotifier;
use signet_rpc::{ServeConfig, StorageRpcConfig};
use signet_storage::{CancellationToken, HistoryRead, HistoryWrite, HotKv, UnifiedStorage};
use std::sync::Arc;
use tokio::sync::mpsc;

#[serial]
#[tokio::test]
async fn test_genesis() {
    let cfg = test_config();
    let consts = cfg.constants();

    let chain_spec: Arc<_> = cfg.chain_spec().clone();
    assert_eq!(chain_spec.genesis().config.chain_id, consts.unwrap().ru_chain_id());

    let cancel_token = CancellationToken::new();
    let hot = MemKv::new();
    {
        let hardforks = signet_genesis::genesis_hardforks(cfg.genesis());
        let writer = hot.writer().unwrap();
        writer.load_genesis(cfg.genesis(), &hardforks).unwrap();
        writer.commit().unwrap();
    }

    let storage = Arc::new(UnifiedStorage::spawn(hot, MemColdBackend::new(), cancel_token.clone()));

    // Create a dummy notifier (not used, we only check genesis loading)
    let (_sender, receiver) = mpsc::unbounded_channel();
    let notifier = TestHostNotifier::new(receiver);

    // Build a dummy blob cacher
    let blob_cacher = signet_blobber::BlobFetcher::builder()
        .with_test_pool()
        .with_explorer_url("https://example.com")
        .with_client(reqwest::Client::new())
        .build_cache()
        .unwrap()
        .spawn();

    let (_, _) = SignetNodeBuilder::new(cfg.clone())
        .with_notifier(notifier)
        .with_storage(Arc::clone(&storage))
        .with_alias_oracle(Arc::new(std::sync::Mutex::new(alloy::primitives::map::HashSet::<
            alloy::primitives::Address,
        >::default())))
        .with_blob_cacher(blob_cacher)
        .with_serve_config(ServeConfig {
            http: vec![],
            http_cors: None,
            ws: vec![],
            ws_cors: None,
            ipc: cfg.ipc_endpoint().map(ToOwned::to_owned),
        })
        .with_rpc_config(StorageRpcConfig::default())
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
