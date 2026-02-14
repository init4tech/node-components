//! Subscription management for `eth_subscribe` / `eth_unsubscribe`.

use crate::interest::{InterestKind, NewBlockNotification};
use ajj::HandlerCtx;
use alloy::{primitives::U64, rpc::types::Log};
use dashmap::DashMap;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::broadcast::{self, error::RecvError};
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};
use tracing::{debug, debug_span, enabled, trace};

/// Either type for subscription outputs.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(untagged)]
pub(crate) enum Either {
    /// A log entry.
    Log(Box<Log>),
    /// A block header.
    Block(Box<alloy::rpc::types::Header>),
}

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
    result: &'a Either,
    subscription: U64,
}

/// Buffer for subscription outputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SubscriptionBuffer {
    /// Log buffer.
    Log(VecDeque<Log>),
    /// Block header buffer.
    Block(VecDeque<alloy::rpc::types::Header>),
}

impl SubscriptionBuffer {
    /// True if the buffer is empty.
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get the number of items in the buffer.
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Log(buf) => buf.len(),
            Self::Block(buf) => buf.len(),
        }
    }

    /// Extend this buffer with another buffer.
    ///
    /// # Panics
    ///
    /// Panics if the buffers are of different types.
    pub(crate) fn extend(&mut self, other: Self) {
        match (self, other) {
            (Self::Log(buf), Self::Log(other)) => buf.extend(other),
            (Self::Block(buf), Self::Block(other)) => buf.extend(other),
            _ => panic!("mismatched buffer types"),
        }
    }

    /// Pop the front of the buffer.
    pub(crate) fn pop_front(&mut self) -> Option<Either> {
        match self {
            Self::Log(buf) => buf.pop_front().map(|log| Either::Log(Box::new(log))),
            Self::Block(buf) => buf.pop_front().map(|header| Either::Block(Box::new(header))),
        }
    }
}

impl From<Vec<Log>> for SubscriptionBuffer {
    fn from(logs: Vec<Log>) -> Self {
        Self::Log(logs.into())
    }
}

impl FromIterator<Log> for SubscriptionBuffer {
    fn from_iter<T: IntoIterator<Item = Log>>(iter: T) -> Self {
        Self::Log(iter.into_iter().collect())
    }
}

impl From<Vec<alloy::rpc::types::Header>> for SubscriptionBuffer {
    fn from(headers: Vec<alloy::rpc::types::Header>) -> Self {
        Self::Block(headers.into())
    }
}

impl FromIterator<alloy::rpc::types::Header> for SubscriptionBuffer {
    fn from_iter<T: IntoIterator<Item = alloy::rpc::types::Header>>(iter: T) -> Self {
        Self::Block(iter.into_iter().collect())
    }
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

            // Drain one buffered item per iteration, checking for
            // cancellation between each send.
            if let Some(item) = notif_buffer.pop_front() {
                let notification = SubscriptionNotification {
                    jsonrpc: "2.0",
                    method: "eth_subscription",
                    params: SubscriptionParams { result: &item, subscription: id },
                };

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
                    result = ajj_ctx.notify(&notification) => {
                        if result.is_err() {
                            trace!("channel to client closed");
                            break;
                        }
                    }
                }
                continue;
            }

            // Buffer empty — wait for incoming broadcast notifications.
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
                if let Some(inner) = self.inner.upgrade() {
                    inner.tasks.retain(|_, task| !task.is_cancelled());
                }
            }
        });
    }
}
