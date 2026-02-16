//! Subscription management for `eth_subscribe` / `eth_unsubscribe`.

use crate::interest::{
    InterestKind, NewBlockNotification,
    buffer::{EventBuffer, EventItem},
};
use ajj::HandlerCtx;
use alloy::primitives::U64;
use dashmap::DashMap;
use std::{
    future::pending,
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::broadcast::{self, error::RecvError};
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};
use tracing::{Instrument, debug, debug_span, enabled, trace};

/// Buffer for subscription outputs: log entries or block headers.
pub(crate) type SubscriptionBuffer = EventBuffer<alloy::rpc::types::Header>;

/// JSON-RPC subscription notification envelope.
#[derive(serde::Serialize)]
struct SubscriptionNotification<'a> {
    jsonrpc: &'static str,
    method: &'static str,
    params: SubscriptionParams<'a>,
}

/// Params field of a subscription notification.
#[derive(serde::Serialize)]
struct SubscriptionParams<'a> {
    result: &'a EventItem<alloy::rpc::types::Header>,
    subscription: U64,
}

/// Tracks ongoing subscription tasks.
///
/// Performs the following functions:
/// - assigns unique subscription IDs
/// - spawns tasks to manage each subscription
/// - allows cancelling subscriptions by ID
///
/// Calling [`Self::new`] spawns a task that periodically cleans stale filters.
/// This task runs on a separate thread to avoid [`DashMap::retain`] deadlock.
/// See [`DashMap`] documentation for more information.
#[derive(Clone)]
pub(crate) struct SubscriptionManager {
    inner: Arc<SubscriptionManagerInner>,
}

impl SubscriptionManager {
    /// Instantiate a new subscription manager, start a task to clean up
    /// subscriptions cancelled by user disconnection.
    pub(crate) fn new(
        notif_sender: broadcast::Sender<NewBlockNotification>,
        clean_interval: Duration,
    ) -> Self {
        let inner = Arc::new(SubscriptionManagerInner::new(notif_sender));
        let task = SubCleanerTask::new(Arc::downgrade(&inner), clean_interval);
        task.spawn();
        Self { inner }
    }
}

impl core::ops::Deref for SubscriptionManager {
    type Target = SubscriptionManagerInner;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl core::fmt::Debug for SubscriptionManager {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SubscriptionManager").finish_non_exhaustive()
    }
}

/// Inner logic for [`SubscriptionManager`].
#[derive(Debug)]
pub(crate) struct SubscriptionManagerInner {
    next_id: AtomicU64,
    tasks: DashMap<U64, CancellationToken>,
    notif_sender: broadcast::Sender<NewBlockNotification>,
}

impl SubscriptionManagerInner {
    /// Create a new subscription manager.
    fn new(notif_sender: broadcast::Sender<NewBlockNotification>) -> Self {
        Self { next_id: AtomicU64::new(1), tasks: DashMap::new(), notif_sender }
    }

    /// Assign a new subscription ID.
    fn next_id(&self) -> U64 {
        U64::from(self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    /// Cancel a subscription task.
    pub(crate) fn unsubscribe(&self, id: U64) -> bool {
        if let Some(task) = self.tasks.remove(&id) {
            task.1.cancel();
            true
        } else {
            false
        }
    }

    /// Subscribe to notifications. Returns `None` if notifications are
    /// disabled.
    pub(crate) fn subscribe(&self, ajj_ctx: &HandlerCtx, filter: InterestKind) -> Option<U64> {
        if !ajj_ctx.notifications_enabled() {
            return None;
        }

        let id = self.next_id();
        let token = CancellationToken::new();
        self.tasks.insert(id, token.clone());
        let task = SubscriptionTask {
            id,
            filter,
            token: token.clone(),
            notifs: self.notif_sender.subscribe(),
        };
        task.spawn(ajj_ctx);

        debug!(%id, "registered new subscription");

        Some(id)
    }
}

/// Task to manage a single subscription.
#[derive(Debug)]
struct SubscriptionTask {
    id: U64,
    filter: InterestKind,
    token: CancellationToken,
    notifs: broadcast::Receiver<NewBlockNotification>,
}

impl SubscriptionTask {
    /// Create the task future.
    async fn task_future(self, ajj_ctx: HandlerCtx, ajj_cancel: WaitForCancellationFutureOwned) {
        let SubscriptionTask { id, filter, token, mut notifs } = self;

        if !ajj_ctx.notifications_enabled() {
            return;
        }

        let mut notif_buffer = filter.empty_sub_buffer();
        tokio::pin!(ajj_cancel);

        loop {
            let span = debug_span!(parent: None, "SubscriptionTask::task_future", %id, filter = tracing::field::Empty);
            if enabled!(tracing::Level::TRACE) {
                span.record("filter", format!("{filter:?}"));
            }

            // NB: reserve half the capacity to avoid blocking other
            // usage. This is a heuristic and can be adjusted as needed.
            let guard = span.enter();
            let permit_fut = async {
                if !notif_buffer.is_empty() {
                    ajj_ctx
                        .permit_many((ajj_ctx.notification_capacity() / 2).min(notif_buffer.len()))
                        .await
                } else {
                    pending().await
                }
            }
            .in_current_span();
            drop(guard);

            // NB: biased select ensures we check cancellation before
            // processing new notifications.
            let _guard = span.enter();
            tokio::select! {
                biased;
                _ = &mut ajj_cancel => {
                    trace!("subscription cancelled by client disconnect");
                    token.cancel();
                    break;
                }
                _ = token.cancelled() => {
                    trace!("subscription cancelled by user");
                    break;
                }
                permits = permit_fut => {
                    let Some(permits) = permits else {
                        trace!("channel to client closed");
                        break
                    };

                    for permit in permits {
                        let Some(item) = notif_buffer.pop_front() else { break };
                        let notification = SubscriptionNotification {
                            jsonrpc: "2.0",
                            method: "eth_subscription",
                            params: SubscriptionParams { result: &item, subscription: id },
                        };
                        let _ = permit.send(&notification);
                    }
                }
                notif_res = notifs.recv() => {
                    let notif = match notif_res {
                        Ok(notif) => notif,
                        Err(RecvError::Lagged(skipped)) => {
                            trace!(skipped, "missed notifications");
                            continue;
                        }
                        Err(e) => {
                            trace!(?e, "notification stream closed");
                            break;
                        }
                    };

                    let output = filter.filter_notification_for_sub(&notif);
                    trace!(count = output.len(), "Filter applied to notification");
                    if !output.is_empty() {
                        notif_buffer.extend(output);
                    }
                }
            }
        }
    }

    /// Spawn on the ajj [`HandlerCtx`].
    fn spawn(self, ctx: &HandlerCtx) {
        ctx.spawn_graceful_with_ctx(|ctx, ajj_cancel| self.task_future(ctx, ajj_cancel));
    }
}

/// Task to clean up cancelled subscriptions.
///
/// This task runs on a separate thread to avoid [`DashMap::retain`] deadlocks.
#[derive(Debug)]
struct SubCleanerTask {
    inner: Weak<SubscriptionManagerInner>,
    interval: Duration,
}

impl SubCleanerTask {
    /// Create a new subscription cleaner task.
    const fn new(inner: Weak<SubscriptionManagerInner>, interval: Duration) -> Self {
        Self { inner, interval }
    }

    /// Run the task. This task runs on a separate thread, which ensures that
    /// [`DashMap::retain`]'s deadlock condition is not met. See [`DashMap`]
    /// documentation for more information.
    fn spawn(self) {
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(self.interval);
                match self.inner.upgrade() {
                    Some(inner) => inner.tasks.retain(|_, task| !task.is_cancelled()),
                    None => break,
                }
            }
        });
    }
}
