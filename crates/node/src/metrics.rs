use std::sync::LazyLock;

use metrics::{Counter, counter, describe_counter};
use reth_exex::ExExNotification;

const NOTIFICATION_RECEIVED: &str = "signet.node.notification_received";
const NOTIFICATION_RECEIVED_HELP: &str = "Number of notifications received";

const REORGS_RECEIVED: &str = "signet.node.reorgs_received";
const REORGS_RECEIVED_HELP: &str = "Number of reorgs received";

const NOTIFICATIONS_PROCESSED: &str = "signet.node.notifications_processed";
const NOTIFICATIONS_PROCESSED_HELP: &str = "Number of notifications processed";

const REORGS_PROCESSED: &str = "signet.node.reorgs_processed";
const REORGS_PROCESSED_HELP: &str = "Number of reorgs processed";

static DESCRIBE: LazyLock<()> = LazyLock::new(|| {
    describe_counter!(NOTIFICATION_RECEIVED, NOTIFICATION_RECEIVED_HELP);
    describe_counter!(REORGS_RECEIVED, REORGS_RECEIVED_HELP);
    describe_counter!(NOTIFICATIONS_PROCESSED, NOTIFICATIONS_PROCESSED_HELP);
    describe_counter!(REORGS_PROCESSED, REORGS_PROCESSED_HELP);
});

fn reorgs_processed() -> Counter {
    LazyLock::force(&DESCRIBE);
    counter!(REORGS_PROCESSED)
}

fn inc_reorgs_processed() {
    reorgs_processed().increment(1);
}

fn notifications_processed() -> Counter {
    LazyLock::force(&DESCRIBE);
    counter!(NOTIFICATIONS_PROCESSED)
}

fn inc_notifications_processed() {
    notifications_processed().increment(1);
}

pub(crate) fn record_notification_received(notification: &ExExNotification) {
    inc_notifications_processed();
    if notification.reverted_chain().is_some() {
        inc_reorgs_processed();
    }
}

pub(crate) fn record_notification_processed(notification: &ExExNotification) {
    inc_notifications_processed();
    if notification.reverted_chain().is_some() {
        inc_reorgs_processed();
    }
}
