//! End-to-end tests for the `journals` sync strategy: a syncing [`SignetNode`] subscribes to an
//! in-process journal WebSocket source via the real [`signet_journal_client::JournalClient`],
//! applies the journals through its ingestor, and (when caught up to the host tip) hands off to
//! block execution.

use alloy::{
    consensus::Header,
    primitives::{Address, B256, keccak256, map::HashSet},
};
use bytes::Bytes;
use serial_test::serial;
use signet_cold::mem::MemColdBackend;
use signet_hot::{
    db::{HistoryWrite, HotDbRead, UnsafeDbWrite},
    mem::MemKv,
};
use signet_journal::{GENESIS_JOURNAL_HASH, HostJournal, Journal, JournalMeta};
use signet_journal_chain::{Checkpoint, extract_signet_metadata, test_utils::TestServer};
use signet_node::{NodeStatus, SignetNodeBuilder};
use signet_node_config::{JournalConfig, test_utils::test_config_with_journal};
use signet_node_tests::{
    HostBlockSpec, NotificationWithSidecars, TestHostNotifier, convert::to_host_notification,
};
use signet_rpc::{ServeConfig, StorageRpcConfig};
use signet_storage::{CancellationToken, HistoryRead, HotKv, UnifiedStorage};
use std::{
    borrow::Cow,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;
use trevm::journal::{BundleStateIndex, JournalEncode};

/// Upper bound on how long any test waits for the node to make progress. In-process journal
/// application is sub-second; the slowest path is source exhaustion (a ~400ms window), so this
/// comfortably exceeds the real timings while keeping a genuine hang quick to surface.
const TIMEOUT: Duration = Duration::from_secs(5);

/// A cloneable builder for chained `Journal::V1` blobs. Two chains advance in lockstep:
///
/// - The **journal-hash chain** (`previous_journal_hash`), seeded at [`GENESIS_JOURNAL_HASH`] so
///   the first journal validates against the node's genesis checkpoint, then chaining off the
///   keccak256 of each prior journal blob.
/// - The **block-header chain** (`parent_hash`), seeded at the genesis header hash so storage's
///   `append_blocks` continuity check passes, then chaining off each prior header's hash.
///
/// Cloning snapshots the state at a height, which a reorg test uses to grow a divergent fork from
/// a common ancestor.
#[derive(Clone)]
struct JournalChainGen {
    previous_journal_hash: B256,
    parent_hash: B256,
    next_height: u64,
}

impl JournalChainGen {
    fn new() -> Self {
        Self {
            previous_journal_hash: GENESIS_JOURNAL_HASH,
            parent_hash: genesis_header_hash(),
            next_height: 1,
        }
    }

    /// Append a journal at the next height. `salt` perturbs the header (via its timestamp) so two
    /// forks at the same height produce distinct header and journal hashes. Returns the wire blob
    /// and its journal hash (the value persisted into the `JournalHashes` table).
    fn push(&mut self, salt: u64) -> (Bytes, B256) {
        let header = Header {
            number: self.next_height,
            parent_hash: self.parent_hash,
            timestamp: salt,
            ..Header::default()
        };
        self.parent_hash = header.hash_slow();
        self.next_height += 1;
        let meta = JournalMeta::new(0, self.previous_journal_hash, Cow::Owned(header));
        let journal = Journal::V1(HostJournal::new(meta, BundleStateIndex::default()));
        let bytes = Bytes::from(journal.encoded().to_vec());
        let journal_hash = keccak256(&bytes);
        self.previous_journal_hash = journal_hash;
        (bytes, journal_hash)
    }
}

/// Build `count` chained journals (salt 0 throughout). Returns the blobs and their journal hashes.
fn build_signet_journals(count: u64) -> (Vec<Bytes>, Vec<B256>) {
    let mut generator = JournalChainGen::new();
    let mut journals = Vec::with_capacity(count as usize);
    let mut journal_hashes = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let (bytes, hash) = generator.push(0);
        journals.push(bytes);
        journal_hashes.push(hash);
    }
    (journals, journal_hashes)
}

/// Load the test genesis into a throwaway hot store and return its block-0 header hash, so test
/// journals can chain their `parent_hash` onto the same genesis the node will load.
fn genesis_header_hash() -> B256 {
    let cfg = test_config_with_journal(JournalConfig::default());
    let hot = MemKv::new();
    let hardforks = signet_genesis::genesis_hardforks(cfg.genesis());
    let writer = hot.writer().unwrap();
    writer.load_genesis(cfg.genesis(), &hardforks).unwrap();
    writer.commit().unwrap();
    let reader = hot.reader().unwrap();
    reader.get_header(0).unwrap().expect("missing genesis header").hash_slow()
}

/// Build and spawn a journal-syncing node over the given `storage` and `cancel_token`, pointed at
/// `server_url` with the host tip fixed at `host_tip`. Genesis is loaded by the builder's prebuild
/// if absent, so the same storage can be handed to a second node to exercise restart-resume.
async fn build_journal_sync_node(
    storage: Arc<UnifiedStorage<MemKv>>,
    cancel_token: CancellationToken,
    server_url: &str,
    host_tip: u64,
) -> (tokio::task::JoinHandle<eyre::Result<()>>, tokio::sync::watch::Receiver<NodeStatus>) {
    // The notifier is held but never polled until the node transitions to block execution; its
    // shared tip drives the catch-up decision.
    let (_sender, receiver) = mpsc::unbounded_channel();
    let notifier = TestHostNotifier::new(receiver, Arc::new(AtomicU64::new(host_tip)));
    let journal = JournalConfig::journal_sync_for_test(vec![server_url.to_owned()]);
    build_node_with_notifier(storage, cancel_token, journal, notifier).await
}

/// Build and spawn a journal-syncing node with a caller-supplied journal config and notifier.
/// Used directly by the drain test (to install a host-backpressuring notifier whose notification
/// sender it retains) and the exhaustion test (to inject a fail-fast journal config).
async fn build_node_with_notifier(
    storage: Arc<UnifiedStorage<MemKv>>,
    cancel_token: CancellationToken,
    journal: JournalConfig,
    notifier: TestHostNotifier,
) -> (tokio::task::JoinHandle<eyre::Result<()>>, tokio::sync::watch::Receiver<NodeStatus>) {
    let cfg = test_config_with_journal(journal);

    let blob_cacher = signet_blobber::BlobFetcher::builder()
        .with_source(signet_blobber::MemoryBlobSource::new())
        .build_cache()
        .spawn();

    let (node, status) = SignetNodeBuilder::new(cfg)
        .with_notifier(notifier)
        .with_storage(storage)
        .with_alias_oracle(Arc::new(Mutex::new(HashSet::<Address>::default())))
        .with_blob_cacher(blob_cacher)
        .with_serve_config(ServeConfig {
            http: vec![],
            http_cors: None,
            ws: vec![],
            ws_cors: None,
            ipc: None,
        })
        .with_rpc_config(StorageRpcConfig::default())
        .with_cancellation_token(cancel_token)
        .build()
        .await
        .unwrap();

    (tokio::spawn(node.start()), status)
}

/// Stand up a journal-syncing node with fresh in-memory storage, pointed at `server_url` with the
/// host tip fixed at `host_tip`. Returns the storage handle, the (shared storage + node)
/// cancellation token, the node's join handle, and a status receiver.
async fn spawn_journal_sync_node(
    server_url: &str,
    host_tip: u64,
) -> (
    Arc<UnifiedStorage<MemKv>>,
    CancellationToken,
    tokio::task::JoinHandle<eyre::Result<()>>,
    tokio::sync::watch::Receiver<NodeStatus>,
) {
    let cancel_token = CancellationToken::new();
    let storage = Arc::new(UnifiedStorage::spawn_erased(
        MemKv::new(),
        MemColdBackend::new(),
        cancel_token.clone(),
    ));
    let (handle, status) =
        build_journal_sync_node(Arc::clone(&storage), cancel_token.clone(), server_url, host_tip)
            .await;
    (storage, cancel_token, handle, status)
}

/// Wait until the node status reports at least `height`. Races the node's join `handle`: if the
/// node exits early it panics with the node's result (a clear diagnostic) rather than letting the
/// caller fall through to a misleading assertion. Panics on timeout.
async fn wait_for_height(
    status: &mut tokio::sync::watch::Receiver<NodeStatus>,
    handle: &mut tokio::task::JoinHandle<eyre::Result<()>>,
    height: u64,
) {
    let wait = async {
        loop {
            if let NodeStatus::AtHeight(current) = *status.borrow_and_update()
                && current >= height
            {
                return;
            }
            tokio::select! {
                biased;
                result = &mut *handle => panic!("node exited before reaching height {height}: {result:?}"),
                _ = status.changed() => {}
            }
        }
    };
    tokio::time::timeout(TIMEOUT, wait).await.expect("timed out waiting for node to reach height");
}

/// Poll storage until the journal hash at `height` equals `expected`, or panic after a timeout.
/// Used to observe a reorg replacing a height's journal with a divergent one.
async fn wait_for_journal_hash(storage: &UnifiedStorage<MemKv>, height: u64, expected: B256) {
    let wait = async {
        loop {
            // The ingestor writes on a separate task, and the in-memory backend rejects a reader
            // taken while a writer is open (`try_read`/`try_write`). A failed acquisition just
            // means a write is mid-flight, so treat it as "not yet" and retry - a real MVCC backend
            // would never block a reader on a writer.
            let current = storage
                .reader()
                .ok()
                .and_then(|reader| reader.get_journal_hash(height).expect("journal hash read"));
            if current == Some(expected) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    tokio::time::timeout(TIMEOUT, wait)
        .await
        .expect("timed out waiting for journal hash to update");
}

#[serial]
#[tokio::test(flavor = "multi_thread")]
async fn journal_sync_applies_journals_to_storage() {
    let (journals, hashes) = build_signet_journals(5);
    let server = TestServer::spawn_with(
        extract_signet_metadata,
        Checkpoint { height: 0, hash: GENESIS_JOURNAL_HASH },
        &journals,
    )
    .await;

    // Pin the host tip far ahead so the node never decides it has caught up: it stays in journal
    // sync and we can observe every applied journal in storage.
    let (storage, cancel_token, mut handle, mut status) =
        spawn_journal_sync_node(server.url.as_str(), 1_000_000).await;

    wait_for_height(&mut status, &mut handle, 5).await;

    let reader = storage.reader().unwrap();
    assert_eq!(reader.last_block_number().unwrap(), Some(5));
    for (index, expected_hash) in hashes.iter().enumerate() {
        let height = index as u64 + 1;
        assert!(reader.get_header(height).unwrap().is_some(), "missing header at height {height}");
        assert_eq!(
            reader.get_journal_hash(height).unwrap(),
            Some(*expected_hash),
            "journal hash mismatch at height {height}",
        );
    }
    drop(reader);

    // Graceful shutdown: cancelling drives the cooperative wind-down and the node exits cleanly.
    cancel_token.cancel();
    let result = tokio::time::timeout(TIMEOUT, handle)
        .await
        .expect("node did not shut down within 10s")
        .expect("node task panicked");
    result.expect("node should shut down cleanly");

    drop(server.shutdown_sender);
}

/// Serve `journals`, spawn a journal-sync node at `host_tip`, and wait for it to transition to
/// block execution and exit cleanly. The node's host-notifier channel has no live sender, so once
/// it hands off to block execution `next_notification` yields `None` and it shuts down on its own;
/// a self-exit with `Ok(())` (no cancellation) is the signal the transition happened - journal
/// sync alone would keep running. Returns storage so the caller can assert how far it synced.
async fn run_until_transition(journals: &[Bytes], host_tip: u64) -> Arc<UnifiedStorage<MemKv>> {
    let server = TestServer::spawn_with(
        extract_signet_metadata,
        Checkpoint { height: 0, hash: GENESIS_JOURNAL_HASH },
        journals,
    )
    .await;
    let (storage, _cancel_token, handle, _status) =
        spawn_journal_sync_node(server.url.as_str(), host_tip).await;

    let result = tokio::time::timeout(TIMEOUT, handle)
        .await
        .expect("node did not transition and exit before timeout")
        .expect("node task panicked");
    result.expect("node should transition to block execution and exit cleanly");

    drop(server.shutdown_sender);
    storage
}

#[serial]
#[tokio::test(flavor = "multi_thread")]
async fn journal_sync_transitions_at_startup_when_already_at_tip() {
    let constants = test_config_with_journal(JournalConfig::default()).constants().unwrap();
    // Boundary case: a host tip at the genesis-paired height means the startup catch-up check
    // passes before the client is even spawned, so no journal is ever applied (the upstream is
    // never connected), the node transitions straight to block execution, and storage stays at
    // genesis.
    let (journals, _) = build_signet_journals(3);
    let storage = run_until_transition(&journals, constants.host_deploy_height()).await;
    assert_eq!(storage.reader().unwrap().last_block_number().unwrap(), Some(0));
}

#[serial]
#[tokio::test(flavor = "multi_thread")]
async fn journal_sync_transitions_after_catching_up() {
    let constants = test_config_with_journal(JournalConfig::default()).constants().unwrap();
    // A host tip paired with rollup height 5 forces the node to apply journals first: the catch-up
    // check only trips once the applied tip is within the transition margin of 5. It then hands
    // off to block execution somewhere in [5 - margin, 5].
    let (journals, _) = build_signet_journals(5);
    let storage = run_until_transition(&journals, constants.pair_ru(5).host).await;

    let tip = storage
        .reader()
        .unwrap()
        .last_block_number()
        .unwrap()
        .expect("node should have synced at least one block before transitioning");
    assert!((3..=5).contains(&tip), "expected transition after syncing 3-5 journals, got {tip}");
}

#[serial]
#[tokio::test(flavor = "multi_thread")]
async fn journal_sync_fatal_when_all_sources_exhausted() {
    // Point the node at a source that never accepts a journal connection. The fail-fast config
    // uses short client timeouts, so the client fails the source, exhausts its only option, and
    // the node bails well within `TIMEOUT`.
    let cancel_token = CancellationToken::new();
    let storage = Arc::new(UnifiedStorage::spawn_erased(
        MemKv::new(),
        MemColdBackend::new(),
        cancel_token.clone(),
    ));
    let notifier =
        TestHostNotifier::new(mpsc::unbounded_channel().1, Arc::new(AtomicU64::new(1_000_000)));
    let journal =
        JournalConfig::journal_sync_for_test_fail_fast(vec!["ws://127.0.0.1:1".to_owned()]);
    let (handle, _status) =
        build_node_with_notifier(storage, cancel_token, journal, notifier).await;

    let result = tokio::time::timeout(TIMEOUT, handle)
        .await
        .expect("node did not bail before timeout")
        .expect("node task panicked");
    let error = result.expect_err("node should bail when all journal sources are exhausted");
    assert!(format!("{error:#}").contains("exhausted all sources"), "unexpected error: {error:#}");
}

#[serial]
#[tokio::test(flavor = "multi_thread")]
async fn journal_sync_handles_reorg() {
    // Common prefix: heights 1-2. Snapshot the generator there, then grow two divergent forks.
    let mut generator = JournalChainGen::new();
    let prefix: Vec<Bytes> = (0..2).map(|_| generator.push(0).0).collect();
    let mut fork = generator.clone();

    // Chain A (salt 0): heights 3-5, served initially.
    let (a3, a3_hash) = generator.push(0);
    let (a4, _) = generator.push(0);
    let (a5, _) = generator.push(0);
    let chain_a: Vec<Bytes> = prefix.iter().cloned().chain([a3, a4, a5]).collect();

    // Chain B (salt 1): divergent heights 3-5, pushed live to trigger the reorg.
    let (b3, b3_hash) = fork.push(1);
    let (b4, b4_hash) = fork.push(1);
    let (b5, b5_hash) = fork.push(1);

    let server = TestServer::spawn_with(
        extract_signet_metadata,
        Checkpoint { height: 0, hash: GENESIS_JOURNAL_HASH },
        &chain_a,
    )
    .await;

    let (storage, cancel_token, mut handle, mut status) =
        spawn_journal_sync_node(server.url.as_str(), 1_000_000).await;

    wait_for_height(&mut status, &mut handle, 5).await;
    assert_eq!(
        storage.reader().unwrap().get_journal_hash(3).unwrap(),
        Some(a3_hash),
        "expected chain A journal at height 3 before the reorg",
    );

    // Push the divergent fork. The server detects the reorg at height 3, truncates, and rebroadcasts
    // the replacements; the node's chain sees the hash mismatch and the ingestor unwinds + replays.
    for journal in [b3, b4, b5] {
        server.journal_sender.send(journal).await.expect("server chain channel open");
    }

    wait_for_journal_hash(&storage, 5, b5_hash).await;
    let reader = storage.reader().unwrap();
    assert_eq!(reader.last_block_number().unwrap(), Some(5), "tip should be back at 5 post-reorg");
    assert_eq!(reader.get_journal_hash(3).unwrap(), Some(b3_hash), "height 3 not replaced");
    assert_eq!(reader.get_journal_hash(4).unwrap(), Some(b4_hash), "height 4 not replaced");
    drop(reader);

    cancel_token.cancel();
    let result = tokio::time::timeout(TIMEOUT, handle)
        .await
        .expect("node did not shut down before timeout")
        .expect("node task panicked");
    result.expect("node should shut down cleanly");

    drop(server.shutdown_sender);
}

#[serial]
#[tokio::test(flavor = "multi_thread")]
async fn journal_sync_resumes_from_persisted_checkpoint() {
    let (journals, hashes) = build_signet_journals(5);

    // The storage outlives both node runs; its own cancellation token is held until the very end
    // so its background tasks survive the first node's shutdown.
    let storage_cancel = CancellationToken::new();
    let storage = Arc::new(UnifiedStorage::spawn_erased(
        MemKv::new(),
        MemColdBackend::new(),
        storage_cancel.clone(),
    ));

    // Phase 1: sync heights 1-3 against an upstream that only holds those, then shut the node down.
    // Storage retains the `JournalHashes` entries the next node will seed from.
    {
        let server = TestServer::spawn_with(
            extract_signet_metadata,
            Checkpoint { height: 0, hash: GENESIS_JOURNAL_HASH },
            &journals[..3],
        )
        .await;
        let node_cancel = CancellationToken::new();
        let (mut handle, mut status) = build_journal_sync_node(
            Arc::clone(&storage),
            node_cancel.clone(),
            server.url.as_str(),
            1_000_000,
        )
        .await;
        wait_for_height(&mut status, &mut handle, 3).await;
        node_cancel.cancel();
        tokio::time::timeout(TIMEOUT, handle)
            .await
            .expect("first node did not shut down before timeout")
            .expect("first node task panicked")
            .expect("first node should shut down cleanly");
        drop(server.shutdown_sender);
    }

    assert_eq!(storage.reader().unwrap().last_block_number().unwrap(), Some(3));
    assert_eq!(storage.reader().unwrap().get_journal_hash(3).unwrap(), Some(hashes[2]));

    // Phase 2: a fresh node over the same storage must seed its client checkpoint from height 3
    // (not genesis) and resume from height 4. The upstream serves the whole 1-5 chain; a node that
    // wrongly re-bootstrapped from genesis would receive height 1 first and fail to append it onto
    // a storage already at tip 3 - so simply reaching height 5 proves the resume.
    {
        let server = TestServer::spawn_with(
            extract_signet_metadata,
            Checkpoint { height: 0, hash: GENESIS_JOURNAL_HASH },
            &journals,
        )
        .await;
        let node_cancel = CancellationToken::new();
        let (mut handle, mut status) = build_journal_sync_node(
            Arc::clone(&storage),
            node_cancel.clone(),
            server.url.as_str(),
            1_000_000,
        )
        .await;
        wait_for_height(&mut status, &mut handle, 5).await;
        let reader = storage.reader().unwrap();
        assert_eq!(reader.last_block_number().unwrap(), Some(5));
        assert_eq!(reader.get_journal_hash(5).unwrap(), Some(hashes[4]));
        drop(reader);
        node_cancel.cancel();
        tokio::time::timeout(TIMEOUT, handle)
            .await
            .expect("second node did not shut down before timeout")
            .expect("second node task panicked")
            .expect("second node should shut down cleanly");
        drop(server.shutdown_sender);
    }

    storage_cancel.cancel();
}

/// Poll `counter` until it reaches at least `target`, or panic after [`TIMEOUT`].
async fn wait_for_count(counter: &AtomicU64, target: u64) {
    let wait = async {
        while counter.load(Ordering::SeqCst) < target {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    tokio::time::timeout(TIMEOUT, wait).await.expect("timed out waiting for drained count");
}

#[serial]
#[tokio::test(flavor = "multi_thread")]
async fn journal_sync_drains_host_notifications() {
    let (journals, _) = build_signet_journals(5);
    let server = TestServer::spawn_with(
        extract_signet_metadata,
        Checkpoint { height: 0, hash: GENESIS_JOURNAL_HASH },
        &journals,
    )
    .await;

    let cancel_token = CancellationToken::new();
    let storage = Arc::new(UnifiedStorage::spawn_erased(
        MemKv::new(),
        MemColdBackend::new(),
        cancel_token.clone(),
    ));

    // Pin the host tip far ahead so the node never transitions: it stays in journal sync, keeping
    // the drain arm active. The notifier reports `backpressures_host` and tallies every drained
    // notification, emulating a reth ExEx.
    let drained = Arc::new(AtomicU64::new(0));
    let (host_sender, host_receiver) = mpsc::unbounded_channel();
    let notifier = TestHostNotifier::new(host_receiver, Arc::new(AtomicU64::new(1_000_000)))
        .with_backpressure(Arc::clone(&drained));

    let journal = JournalConfig::journal_sync_for_test(vec![server.url.as_str().to_owned()]);
    let (mut handle, mut status) =
        build_node_with_notifier(Arc::clone(&storage), cancel_token.clone(), journal, notifier)
            .await;

    // Feed host notifications the node must drain and discard - it applies journals, not blocks.
    let constants = test_config_with_journal(JournalConfig::default()).constants().unwrap();
    for height in 1..=3u64 {
        let block = HostBlockSpec::new(constants.clone());
        block.set_block_number(height);
        let notification = NotificationWithSidecars::commit_single_block(block);
        host_sender.send(to_host_notification(&notification.notification)).unwrap();
    }

    // Journals still apply while the host notifications are concurrently drained, and every fed
    // notification is consumed rather than left to pile up (which would stall a real host).
    wait_for_height(&mut status, &mut handle, 5).await;
    wait_for_count(&drained, 3).await;
    assert_eq!(storage.reader().unwrap().last_block_number().unwrap(), Some(5));

    cancel_token.cancel();
    let result = tokio::time::timeout(TIMEOUT, handle)
        .await
        .expect("node did not shut down before timeout")
        .expect("node task panicked");
    result.expect("node should shut down cleanly");

    drop(server.shutdown_sender);
}

/// A journals-strategy node that boots already at the host tip transitions straight to block
/// execution without ever spawning the journal-sync ingestor. The journal chain still holds the
/// backpressured sender, so the ingestor's (unused) receiver end must be released at the handoff -
/// otherwise the chain blocks forever on its `blocking_send` once block execution has produced
/// enough journals to fill that channel, stalling ingestion and hanging the next shutdown.
///
/// This drives more host blocks than the backpressured channel can hold and then asserts a clean,
/// timely shutdown - which only completes if the chain was never allowed to block.
#[serial]
#[tokio::test(flavor = "multi_thread")]
async fn journal_sync_startup_transition_does_not_stall_block_execution() {
    let constants = test_config_with_journal(JournalConfig::default()).constants().unwrap();
    let deploy = constants.host_deploy_height();

    // The node transitions at startup (fresh storage, host tip at the genesis-paired deploy
    // height), so it never connects upstream; the server only needs to supply a valid source URL.
    let server = TestServer::spawn_with(
        extract_signet_metadata,
        Checkpoint { height: 0, hash: GENESIS_JOURNAL_HASH },
        &[],
    )
    .await;

    let cancel_token = CancellationToken::new();
    let storage = Arc::new(UnifiedStorage::spawn_erased(
        MemKv::new(),
        MemColdBackend::new(),
        cancel_token.clone(),
    ));

    // A live host sender so block execution has blocks to process after the handoff.
    let (host_sender, host_receiver) = mpsc::unbounded_channel();
    let notifier = TestHostNotifier::new(host_receiver, Arc::new(AtomicU64::new(deploy)));
    let journal = JournalConfig::journal_sync_for_test(vec![server.url.as_str().to_owned()]);
    let (mut handle, mut status) =
        build_node_with_notifier(Arc::clone(&storage), cancel_token.clone(), journal, notifier)
            .await;

    // Comfortably more than JOURNAL_SYNC_BACKPRESSURE_CAPACITY (16): enough emitted journals to
    // fill the backpressured channel and force the chain to block on its `blocking_send` if the
    // receiver was leaked. Each block carries an enter so it advances the rollup by one.
    let block_count = 20u64;
    let usdc = constants.host().tokens().usdc();
    for offset in 1..=block_count {
        let block = HostBlockSpec::new(constants.clone()).enter_token(
            Address::repeat_byte(0x77),
            100,
            usdc,
        );
        block.set_block_number(deploy + offset);
        let notification = NotificationWithSidecars::commit_single_block(block);
        host_sender.send(to_host_notification(&notification.notification)).unwrap();
    }

    // Storage advances even with the bug (the chain stalls behind it), so reaching the tip alone
    // does not prove the fix; it only ensures every journal has been emitted before we shut down.
    wait_for_height(&mut status, &mut handle, block_count).await;
    assert_eq!(storage.reader().unwrap().last_block_number().unwrap(), Some(block_count));

    // The discriminator: a chain blocked on the backpressured send never finishes, so awaiting it
    // during shutdown would hang. A clean, timely exit proves the receiver was released.
    cancel_token.cancel();
    let result = tokio::time::timeout(TIMEOUT, handle)
        .await
        .expect(
            "node did not shut down before timeout: journal chain stalled on backpressured send",
        )
        .expect("node task panicked");
    result.expect("node should shut down cleanly");

    drop(server.shutdown_sender);
}
