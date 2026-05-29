//! Setup, running state, and run-loop helpers for the `journals` sync strategy.
//!
//! Everything in here is consumed by [`crate::SignetNode::run_journal_sync`] and is
//! deliberately decoupled from the node struct so the orchestrator method stays a thin shell.

use crate::journal_sync::JournalIngestor;
use bytes::Bytes;
use eyre::{Context, OptionExt, Report, eyre};
use signet_journal::GENESIS_JOURNAL_HASH;
#[cfg(doc)]
use signet_journal_chain::JournalChainEvent;
use signet_journal_chain::{Checkpoint, JournalChainError, SAFETY_MARGIN, extract_signet_metadata};
use signet_journal_client::{JournalClient, JournalClientConfig};
use signet_node_config::JournalConfig;
use signet_node_types::HostNotifier;
use signet_storage::{HistoryRead, HotDbRead, HotKv, HotKvRead, UnifiedStorage};
use signet_types::constants::SignetSystemConstants;
use tokio::{
    sync::{mpsc, watch},
    task::{JoinError, JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};
use trevm::revm::database::DBErrorMarker;
use url::Url;

/// Capacity of the bounded channel carrying [`JournalChainEvent`]s from the journal chain to the
/// ingestor on a syncing node. Small: it exists only to let the chain run a few journals ahead of
/// the ingestor; once full, the chain's `blocking_send` backpressures all the way to the upstream
/// source.
pub(crate) const JOURNAL_SYNC_BACKPRESSURE_CAPACITY: usize = 16;

/// Host-block margin allowed between the journal-synced rollup tip and the host tip when deciding
/// the node has caught up. A persistent one-block lag is normal in steady state (the host has
/// usually ticked once between the time we apply a journal and the time we query the tip), so a
/// strict equality check would never converge. `DbBackfill` closes any actual gap when
/// `run_block_sync` calls `set_head`.
pub(crate) const JOURNAL_SYNC_TRANSITION_MARGIN: u64 = 2;

/// Whether the journal-chain ingestion task was expected to have exited at the point of awaiting
/// its join handle.
#[derive(Debug, Clone, Copy)]
pub(crate) enum JournalExitKind {
    /// Awaited during shutdown after closing the input channel; a clean exit is success.
    Expected,
    /// Awaited while still expecting to feed journals; any exit, clean or otherwise, is fatal.
    Unexpected,
}

/// Translate a chain task's join result into an `eyre::Result`, accounting for whether a clean
/// exit was anticipated.
pub(crate) fn journal_task_result(
    result: Result<Result<(), JournalChainError>, JoinError>,
    kind: JournalExitKind,
) -> eyre::Result<()> {
    match result {
        Ok(Ok(())) => match kind {
            JournalExitKind::Expected => Ok(()),
            JournalExitKind::Unexpected => {
                Err(eyre!("journal chain ingestion task exited unexpectedly"))
            }
        },
        Ok(Err(error)) => Err(Report::new(error).wrap_err("journal chain ingestion task failed")),
        Err(error) => {
            Err(Report::new(error).wrap_err("journal chain ingestion task panicked or was aborted"))
        }
    }
}

/// Owns the running journal client and ingestor tasks plus the signals the run-loop reacts to.
/// Cancelling the embedded token stops both tasks cooperatively without affecting the journal
/// chain task (which the node owns separately and keeps alive across the handoff to block
/// execution).
pub(crate) struct RunningJournalSync {
    pub(crate) sync_tasks: JoinSet<eyre::Result<()>>,
    pub(crate) sync_token: CancellationToken,
    pub(crate) applied_rollup_height: watch::Receiver<u64>,
}

impl RunningJournalSync {
    /// Spawn the client and ingestor tasks. The client subscribes from upstream sources and
    /// forwards journal bytes into `journal_sender` (the journal chain's input); the ingestor
    /// applies the journal chain's events to storage. Both are cancelled via `sync_token`, and
    /// `applied_rollup_height` carries the ingestor's progress to the run-loop. The journal chain
    /// task itself is owned separately by the node.
    pub(crate) fn start<H>(
        client: JournalClient,
        journal_sender: mpsc::Sender<Bytes>,
        ingestor: JournalIngestor<H>,
        sync_token: CancellationToken,
        applied_rollup_height: watch::Receiver<u64>,
    ) -> Self
    where
        H: HotKv + Clone + Send + Sync + 'static,
        <H::RoTx as HotKvRead>::Error: DBErrorMarker,
    {
        let client_token = sync_token.clone();
        let mut sync_tasks: JoinSet<eyre::Result<()>> = JoinSet::new();
        sync_tasks.spawn(async move {
            tokio::select! {
                biased;
                () = client_token.cancelled() => Ok(()),
                result = client.subscribe(journal_sender) => match result {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        Err(Report::new(error).wrap_err("journal client exhausted all sources"))
                    }
                },
            }
        });
        sync_tasks
            .spawn(async move { ingestor.run().await.wrap_err("journal ingestor task failed") });
        Self { sync_tasks, sync_token, applied_rollup_height }
    }

    /// Cancel the sync token (idempotent) and drain both tasks cooperatively, logging late errors
    /// without overriding the caller's primary result.
    pub(crate) async fn cancel_and_drain(mut self) {
        self.sync_token.cancel();
        while let Some(joined) = self.sync_tasks.join_next().await {
            match joined {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    let error = format!("{error:#}");
                    error!(error, "journal sync task errored during shutdown");
                }
                Err(join_error) => {
                    error!(error = ?join_error, "journal sync task panicked during shutdown");
                }
            }
        }
    }
}

/// First terminal event observed by [`journal_sync_loop`].
pub(crate) enum SyncOutcome {
    /// The journal-synced tip reached the host tip and the node should hand off to block
    /// execution.
    CaughtUp,
    /// The node's cancellation token fired; perform a graceful shutdown.
    Shutdown,
    /// A task ended in an unexpected or fatal way; propagate the failure.
    Failure(SyncFailure),
}

/// How a journal-sync task or the chain task ended unexpectedly.
pub(crate) enum SyncFailure {
    /// The chain task exited while the sync was still running (the node still held its journal
    /// sender, so a clean exit here is itself unexpected).
    ChainExited(Result<Result<(), JournalChainError>, JoinError>),
    /// A sync task returned an error.
    SyncTaskFailed(Report),
    /// A sync task panicked or was aborted.
    SyncTaskPanicked(JoinError),
    /// A sync task returned `Ok(())` without cancellation - impossible under normal operation
    /// (both tasks only reach `Ok(())` via the cooperative teardown path that the caller
    /// initiates after the loop exits).
    SyncTaskExitedPrematurely,
}

/// React to progress published by the ingestor, the chain task's exit, sync task failures, and
/// cancellation; return the first terminal event.
///
/// Takes `&mut notifier` so it can drain and discard host notifications for backends that
/// [backpressure the host](HostNotifier::backpressures_host) when notifications go unconsumed
/// (a reth ExEx). Journal sync derives state from the upstream feed, not these notifications, so
/// they are discarded; draining only keeps the host's pipeline from stalling. `FinishedHeight` is
/// deliberately never signalled, so the host retains the blocks the post-catch-up handoff
/// backfills via `set_head`.
pub(crate) async fn journal_sync_loop<N>(
    notifier: &mut N,
    constants: &SignetSystemConstants,
    cancellation_token: &CancellationToken,
    journal_task: &mut JoinHandle<Result<(), JournalChainError>>,
    sync: &mut RunningJournalSync,
) -> SyncOutcome
where
    N: HostNotifier,
{
    let mut drain_host = notifier.backpressures_host();

    loop {
        let mut progressed = false;
        tokio::select! {
            biased;
            () = cancellation_token.cancelled() => return SyncOutcome::Shutdown,
            chain_result = &mut *journal_task => {
                return SyncOutcome::Failure(SyncFailure::ChainExited(chain_result));
            }
            joined = sync.sync_tasks.join_next() => match joined {
                Some(Ok(Ok(()))) => {
                    return SyncOutcome::Failure(SyncFailure::SyncTaskExitedPrematurely);
                }
                Some(Ok(Err(error))) => {
                    return SyncOutcome::Failure(SyncFailure::SyncTaskFailed(error));
                }
                Some(Err(join_error)) => {
                    return SyncOutcome::Failure(SyncFailure::SyncTaskPanicked(join_error));
                }
                // `join_next` only returns `None` for an empty set; both sync tasks are in flight
                // here, so the variants above always cover the completion.
                None => unreachable!("sync task set started non-empty"),
            },
            // Ordered above the progress wakeup so draining stays prompt under a host backfill
            // flood. Starving the wakeup is harmless: the catch-up evaluation below reads progress
            // from the watch's change flag rather than from which arm won, so a perpetually-ready
            // drain can never indefinitely defer a transition.
            drained = notifier.next_notification(), if drain_host => match drained {
                // Discard: consumed only to keep the host's notification pipeline draining.
                Some(Ok(_)) => {}
                Some(Err(error)) => {
                    warn!(%error, "host notification drain failed; disabling drain");
                    drain_host = false;
                }
                None => {
                    debug!("host notification stream closed; disabling drain");
                    drain_host = false;
                }
            },
            // A wakeup so the loop re-evaluates catch-up when the ingestor advances even if no
            // other arm is active (e.g. a pull-based follower that never drains). `Err` means the
            // ingestor dropped its sender; the next iteration's `sync_tasks` arm reports the exit.
            changed = sync.applied_rollup_height.changed() => { progressed = changed.is_ok(); }
        }
        // The `changed` arm only sets `progressed` when it wins the biased select; a busy drain arm
        // can keep winning while the ingestor advances concurrently. `has_changed` catches that
        // unconsumed advance so the catch-up check is never starved. Only rollup progress can newly
        // satisfy the condition (the host tip only rises), so non-progress wakeups skip the
        // host-tip query. Re-checked outside the `select!` so it never borrows `notifier` while the
        // drain arm holds `&mut *notifier`.
        if !progressed {
            progressed = sync.applied_rollup_height.has_changed().unwrap_or(false);
        }
        if progressed {
            let rollup_tip = *sync.applied_rollup_height.borrow_and_update();
            if is_caught_up_to_host(&*notifier, rollup_tip, constants).await {
                return SyncOutcome::CaughtUp;
            }
        }
    }
}

/// Whether a journal-synced rollup tip of `rollup_tip` is within [`JOURNAL_SYNC_TRANSITION_MARGIN`]
/// host blocks of the current host tip.
pub(crate) async fn is_caught_up_to_host<N>(
    notifier: &N,
    rollup_tip: u64,
    constants: &SignetSystemConstants,
) -> bool
where
    N: HostNotifier,
{
    match notifier.host_tip().await {
        Ok(host_tip) => rollup_caught_up(rollup_tip, host_tip, constants),
        Err(error) => {
            let error = format!("{:#}", Report::new(error).wrap_err("failed to query host tip"));
            warn!(error, "host tip check failed, treating as not caught up");
            false
        }
    }
}

/// Whether a journal-synced rollup tip at `rollup_tip` is within [`JOURNAL_SYNC_TRANSITION_MARGIN`]
/// host blocks of `host_tip`. `rollup_tip == 0` (genesis) pairs to the host deploy height since
/// there is no rollup block to pair.
const fn rollup_caught_up(
    rollup_tip: u64,
    host_tip: u64,
    constants: &SignetSystemConstants,
) -> bool {
    let paired_host = if rollup_tip == 0 {
        constants.host_deploy_height()
    } else {
        constants.pair_ru(rollup_tip).host
    };
    paired_host.saturating_add(JOURNAL_SYNC_TRANSITION_MARGIN) >= host_tip
}

/// Translate a [`SyncFailure`] into an `eyre::Result`, awaiting the chain task for cleanup where
/// it hasn't already completed.
pub(crate) async fn collapse_sync_failure(
    failure: SyncFailure,
    journal_task: JoinHandle<Result<(), JournalChainError>>,
) -> eyre::Result<()> {
    // For `ChainExited`, the chain task already completed in the select; don't await its handle
    // again (which would panic). For every other variant, drain it cooperatively.
    if let SyncFailure::ChainExited(chain_result) = failure {
        return journal_task_result(chain_result, JournalExitKind::Unexpected);
    }
    // The sync failure below is the primary error; the chain task is only drained here. Log its
    // failure rather than dropping it, so a concurrent chain error is not silently lost.
    if let Err(error) = journal_task_result(journal_task.await, JournalExitKind::Expected) {
        let error = format!("{error:#}");
        error!(error, "journal chain task also errored while collapsing a sync failure");
    }
    match failure {
        SyncFailure::ChainExited(_) => unreachable!(),
        SyncFailure::SyncTaskFailed(error) => Err(error),
        SyncFailure::SyncTaskPanicked(join_error) => {
            Err(Report::new(join_error).wrap_err("journal sync task panicked or was aborted"))
        }
        SyncFailure::SyncTaskExitedPrematurely => {
            Err(eyre!("journal sync task exited unexpectedly before cancellation"))
        }
    }
}

/// The primary and fallback checkpoints used to seed a [`JournalClient`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct JournalCheckpoints {
    /// Last known-good point; the client subscribes from `primary.height + 1`.
    pub(crate) primary: Checkpoint,
    /// A point `SAFETY_MARGIN` behind the primary (saturating to genesis), used as the reconnect
    /// anchor when a reorg spanned downtime.
    pub(crate) fallback: Checkpoint,
}

/// Seed the [`JournalClient`] checkpoints from the node's `JournalHashes` table.
///
/// On a fresh database both checkpoints are the genesis journal hash. Otherwise the primary is
/// the storage tip and the fallback is `SAFETY_MARGIN` blocks behind it (saturating to genesis),
/// so the client can resubscribe from a known-good point after a reorg that spanned downtime
/// instead of re-bootstrapping from genesis.
pub(crate) fn seed_journal_checkpoints<H>(
    storage: &UnifiedStorage<H>,
) -> eyre::Result<JournalCheckpoints>
where
    H: HotKv,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    let reader = storage.reader()?;
    let tip = reader.last_block_number()?.unwrap_or(0);

    if tip == 0 {
        let genesis = Checkpoint { height: 0, hash: GENESIS_JOURNAL_HASH };
        return Ok(JournalCheckpoints { primary: genesis, fallback: genesis });
    }

    let primary_hash = reader.get_journal_hash(tip)?.ok_or_eyre(
        "journal sync requires the JournalHashes table to be populated at the storage tip; \
         either it was disabled previously or storage is corrupt",
    )?;

    let fallback_height = tip.saturating_sub(SAFETY_MARGIN);
    let fallback_hash = if fallback_height == 0 {
        GENESIS_JOURNAL_HASH
    } else {
        reader
            .get_journal_hash(fallback_height)?
            .ok_or_eyre("journal sync requires a JournalHashes entry at the fallback height")?
    };

    Ok(JournalCheckpoints {
        primary: Checkpoint { height: tip, hash: primary_hash },
        fallback: Checkpoint { height: fallback_height, hash: fallback_hash },
    })
}

/// Build a [`JournalClient`] from the journal configuration and seeded checkpoints. Source URLs
/// are parsed here; the client further validates their scheme and shape (see
/// [`JournalClient::new`]).
pub(crate) fn build_journal_client(
    config: &JournalConfig,
    checkpoints: JournalCheckpoints,
) -> eyre::Result<JournalClient> {
    let sources = config
        .sources()
        .iter()
        .map(|source| {
            Url::parse(source).wrap_err_with(|| format!("invalid journal source URL: {source}"))
        })
        .collect::<eyre::Result<Vec<_>>>()?;

    let mut client_config = JournalClientConfig::new(sources);
    if let Some(timeout) = config.client_source_stall_timeout() {
        client_config.source_stall_timeout = timeout;
    }
    if let Some(backoff) = config.client_source_backoff() {
        client_config.source_backoff = backoff;
    }

    JournalClient::new(
        extract_signet_metadata,
        checkpoints.primary,
        checkpoints.fallback,
        client_config,
    )
    .wrap_err("failed to construct journal client")
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        consensus::Header,
        primitives::{B256, Sealable},
    };
    use signet_cold::mem::MemColdBackend;
    use signet_hot::{db::UnsafeDbWrite, mem::MemKv};
    use signet_node_config::{JournalConfig, test_utils::test_config_with_journal};

    /// A chain task that has already completed cleanly, for `collapse_sync_failure` cases that
    /// drain (rather than inspect) the chain handle.
    fn finished_chain_task() -> JoinHandle<Result<(), JournalChainError>> {
        tokio::spawn(async { Ok(()) })
    }

    #[test]
    fn rollup_caught_up_respects_transition_margin() {
        let constants = test_config_with_journal(JournalConfig::default()).constants().unwrap();

        // At genesis (rollup tip 0) the paired host height is the deploy height.
        let deploy = constants.host_deploy_height();
        assert!(rollup_caught_up(0, deploy, &constants));
        // Caught up while the host sits within the margin ahead...
        assert!(rollup_caught_up(0, deploy + JOURNAL_SYNC_TRANSITION_MARGIN, &constants));
        // ...but not once it is one block past the margin.
        assert!(!rollup_caught_up(0, deploy + JOURNAL_SYNC_TRANSITION_MARGIN + 1, &constants));

        // At a non-zero rollup tip the paired host height comes from the pairing.
        let rollup_tip = 5;
        let paired = constants.pair_ru(rollup_tip).host;
        assert!(rollup_caught_up(rollup_tip, paired, &constants));
        assert!(rollup_caught_up(rollup_tip, paired + JOURNAL_SYNC_TRANSITION_MARGIN, &constants));
        assert!(!rollup_caught_up(
            rollup_tip,
            paired + JOURNAL_SYNC_TRANSITION_MARGIN + 1,
            &constants
        ));
    }

    #[tokio::test]
    async fn collapse_sync_failure_propagates_task_error() {
        let failure = SyncFailure::SyncTaskFailed(eyre!("ingestor blew up"));
        let error = collapse_sync_failure(failure, finished_chain_task()).await.unwrap_err();
        assert!(format!("{error:#}").contains("ingestor blew up"), "unexpected error: {error:#}");
    }

    #[tokio::test]
    async fn collapse_sync_failure_reports_premature_exit() {
        let error =
            collapse_sync_failure(SyncFailure::SyncTaskExitedPrematurely, finished_chain_task())
                .await
                .unwrap_err();
        assert!(
            format!("{error:#}").contains("exited unexpectedly"),
            "unexpected error: {error:#}"
        );
    }

    #[tokio::test]
    async fn collapse_sync_failure_reports_panic() {
        // Produce a real `JoinError` by awaiting a task that panicked.
        let panicked = tokio::spawn(async { panic!("boom") }).await.unwrap_err();
        let error =
            collapse_sync_failure(SyncFailure::SyncTaskPanicked(panicked), finished_chain_task())
                .await
                .unwrap_err();
        assert!(
            format!("{error:#}").contains("panicked or was aborted"),
            "unexpected error: {error:#}"
        );
    }

    #[tokio::test]
    async fn collapse_sync_failure_chain_exit_is_unexpected() {
        // A `ChainExited(Ok(Ok(())))` is a clean chain exit while we still expected to feed it -
        // treated as fatal. The carried result is inspected directly, so a second never-resolving
        // handle is passed to prove it is not awaited.
        let never = tokio::spawn(async {
            std::future::pending::<()>().await;
            Ok(())
        });
        let error =
            collapse_sync_failure(SyncFailure::ChainExited(Ok(Ok(()))), never).await.unwrap_err();
        assert!(
            format!("{error:#}").contains("exited unexpectedly"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn build_journal_client_rejects_bad_url() {
        let config = JournalConfig::journal_sync_for_test(vec!["not a url".to_owned()]);
        let checkpoints = JournalCheckpoints {
            primary: Checkpoint { height: 0, hash: GENESIS_JOURNAL_HASH },
            fallback: Checkpoint { height: 0, hash: GENESIS_JOURNAL_HASH },
        };
        let error = build_journal_client(&config, checkpoints).unwrap_err();
        assert!(
            format!("{error:#}").contains("invalid journal source URL"),
            "unexpected error: {error:#}"
        );
    }

    /// Build an in-memory `UnifiedStorage` whose hot store reports `tip` as its last block number
    /// (via a lone header at that height, or an empty store when `tip` is `None`) and holds a
    /// `JournalHashes` entry for each `(height, hash)`. Returns the storage and a token to cancel
    /// its cold task on teardown.
    fn storage_with(
        tip: Option<u64>,
        journal_hashes: &[(u64, B256)],
    ) -> (UnifiedStorage<MemKv>, CancellationToken) {
        let hot = MemKv::new();
        let writer = hot.writer().unwrap();
        if let Some(tip) = tip {
            let header = Header { number: tip, ..Header::default() }.seal_slow();
            writer.put_header(&header).unwrap();
        }
        for (height, hash) in journal_hashes {
            writer.put_journal_hash(*height, hash).unwrap();
        }
        writer.commit().unwrap();

        let cancel = CancellationToken::new();
        let storage = UnifiedStorage::spawn_erased(hot, MemColdBackend::new(), cancel.clone());
        (storage, cancel)
    }

    #[tokio::test]
    async fn seed_checkpoints_fresh_db_uses_genesis() {
        let (storage, cancel) = storage_with(None, &[]);
        let checkpoints = seed_journal_checkpoints(&storage).unwrap();
        assert_eq!(checkpoints.primary.height, 0);
        assert_eq!(checkpoints.primary.hash, GENESIS_JOURNAL_HASH);
        assert_eq!(checkpoints.fallback.height, 0);
        assert_eq!(checkpoints.fallback.hash, GENESIS_JOURNAL_HASH);
        cancel.cancel();
    }

    #[tokio::test]
    async fn seed_checkpoints_fallback_saturates_to_genesis() {
        // Tip below SAFETY_MARGIN: the primary is a real recorded hash but the fallback height
        // saturates to genesis.
        let tip_hash = B256::repeat_byte(0x11);
        let (storage, cancel) = storage_with(Some(3), &[(3, tip_hash)]);
        let checkpoints = seed_journal_checkpoints(&storage).unwrap();
        assert_eq!(checkpoints.primary.height, 3);
        assert_eq!(checkpoints.primary.hash, tip_hash);
        assert_eq!(checkpoints.fallback.height, 0);
        assert_eq!(checkpoints.fallback.hash, GENESIS_JOURNAL_HASH);
        cancel.cancel();
    }

    #[tokio::test]
    async fn seed_checkpoints_uses_non_genesis_fallback() {
        // Tip above SAFETY_MARGIN: the fallback anchors `SAFETY_MARGIN` blocks back at its own
        // recorded hash, not genesis.
        let tip = SAFETY_MARGIN + 50;
        let fallback_height = tip - SAFETY_MARGIN;
        let tip_hash = B256::repeat_byte(0x22);
        let fallback_hash = B256::repeat_byte(0x33);
        let (storage, cancel) =
            storage_with(Some(tip), &[(tip, tip_hash), (fallback_height, fallback_hash)]);
        let checkpoints = seed_journal_checkpoints(&storage).unwrap();
        assert_eq!(checkpoints.primary.height, tip);
        assert_eq!(checkpoints.primary.hash, tip_hash);
        assert_eq!(checkpoints.fallback.height, fallback_height);
        assert_eq!(checkpoints.fallback.hash, fallback_hash);
        cancel.cancel();
    }

    #[tokio::test]
    async fn seed_checkpoints_errors_when_tip_hash_missing() {
        // A non-genesis tip with no recorded journal hash (persistence was off): fatal.
        let (storage, cancel) = storage_with(Some(5), &[]);
        let error = seed_journal_checkpoints(&storage).unwrap_err();
        assert!(format!("{error:#}").contains("storage tip"), "unexpected error: {error:#}");
        cancel.cancel();
    }

    #[tokio::test]
    async fn seed_checkpoints_errors_when_fallback_hash_missing() {
        // Primary present but the fallback height has no recorded hash: fatal.
        let tip = SAFETY_MARGIN + 50;
        let (storage, cancel) = storage_with(Some(tip), &[(tip, B256::repeat_byte(0x44))]);
        let error = seed_journal_checkpoints(&storage).unwrap_err();
        assert!(format!("{error:#}").contains("fallback height"), "unexpected error: {error:#}");
        cancel.cancel();
    }
}
