use alloy::primitives::Address;
use serial_test::serial;
use signet_cold::mem::MemColdBackend;
use signet_hot::{
    db::{HotDbRead, UnsafeDbWrite},
    mem::MemKv,
};
use signet_journal::GENESIS_JOURNAL_HASH;
use signet_node::SignetNodeBuilder;
use signet_node_config::test_utils::test_config;
use signet_node_tests::{HostBlockSpec, TestHostNotifier, run_test};
use signet_rpc::{ServeConfig, StorageRpcConfig};
use signet_storage::{CancellationToken, HistoryRead, HistoryWrite, HotKv, UnifiedStorage};
use std::sync::Arc;
use tokio::sync::mpsc;

#[serial]
#[tokio::test]
async fn test_genesis() {
    let cfg = test_config();
    let consts = cfg.constants().unwrap();
    assert_eq!(cfg.genesis().config.chain_id, consts.ru_chain_id());

    let cancel_token = CancellationToken::new();
    let hot = MemKv::new();
    {
        let hardforks = signet_genesis::genesis_hardforks(cfg.genesis());
        let writer = hot.writer().unwrap();
        writer.load_genesis(cfg.genesis(), &hardforks).unwrap();
        writer.commit().unwrap();
    }

    let storage =
        Arc::new(UnifiedStorage::spawn_erased(hot, MemColdBackend::new(), cancel_token.clone()));

    // Create a dummy notifier (not used, we only check genesis loading)
    let (_sender, receiver) = mpsc::unbounded_channel();
    let notifier = TestHostNotifier::new(receiver);

    // Build a dummy blob cacher
    let blob_cacher = signet_blobber::BlobFetcher::builder()
        .with_source(signet_blobber::MemoryBlobSource::new())
        .build_cache()
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
            ipc: None,
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

    // Genesis is loaded outside the producer's journal-emit path, so the `JournalHashes` table
    // has no entry at height 0 - `previous_journal_hash` falls back to `GENESIS_JOURNAL_HASH`.
    assert_eq!(reader.get_journal_hash(0).unwrap(), None);

    cancel_token.cancel();
}

#[serial]
#[tokio::test]
async fn test_journal_hash_persisted_after_process_block() {
    run_test(|ctx| async move {
        // Sanity: nothing persisted at genesis.
        let reader = ctx.storage.reader().unwrap();
        assert_eq!(reader.get_journal_hash(0).unwrap(), None);
        assert_eq!(reader.get_journal_hash(1).unwrap(), None);
        drop(reader);

        let block = HostBlockSpec::new(ctx.constants()).enter_token(
            Address::repeat_byte(0x77),
            100,
            ctx.constants().host().tokens().usdc(),
        );
        ctx.process_block(block).await.unwrap();

        // After processing block 1, the producer's encoded journal hash must be persisted in
        // hot storage so a restart can seed `previous_journal_hash` from it.
        let hash_1 = {
            let reader = ctx.storage.reader().unwrap();
            let hash = reader
                .get_journal_hash(1)
                .unwrap()
                .expect("journal hash for block 1 was not persisted");
            assert_ne!(
                hash, GENESIS_JOURNAL_HASH,
                "first journal hash must not equal genesis sentinel"
            );
            hash
        };

        // Processing a second block should chain off the first - persistence at block 2 must
        // also succeed, and the two hashes must differ.
        let block = HostBlockSpec::new(ctx.constants()).enter_token(
            Address::repeat_byte(0x77),
            200,
            ctx.constants().host().tokens().usdc(),
        );
        ctx.process_block(block).await.unwrap();

        let reader = ctx.storage.reader().unwrap();
        let hash_2 = reader
            .get_journal_hash(2)
            .unwrap()
            .expect("journal hash for block 2 was not persisted");
        assert_ne!(hash_1, hash_2);
    })
    .await;
}
