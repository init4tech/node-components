use serial_test::serial;
use signet_cold::mem::MemColdBackend;
use signet_hot::{
    db::{HotDbRead, UnsafeDbWrite},
    mem::MemKv,
};
use signet_node::SignetNodeBuilder;
use signet_node_config::test_utils::test_config;
use signet_storage::{CancellationToken, HistoryRead, HistoryWrite, HotKv, UnifiedStorage};
use std::sync::Arc;

#[serial]
#[tokio::test]
async fn test_genesis() {
    let cfg = test_config();
    let consts = cfg.constants();
    let (ctx, _) = reth_exex_test_utils::test_exex_context().await.unwrap();

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

    let (_, _) = SignetNodeBuilder::new(cfg.clone())
        .with_ctx(ctx)
        .with_storage(Arc::clone(&storage))
        .build()
        .unwrap();

    let reader = storage.reader().unwrap();
    assert!(reader.has_block(0).unwrap());

    let header = reader.get_header(0).unwrap().expect("missing genesis header");
    let zero_hash = alloy::primitives::B256::ZERO;
    assert_eq!(header.parent_hash, zero_hash);
    assert_eq!(header.base_fee_per_gas, Some(0x3b9aca00));

    cancel_token.cancel();
}
