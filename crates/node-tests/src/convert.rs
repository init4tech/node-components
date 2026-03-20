use signet_extract::Extractable;
use signet_node_types::{HostNotification, HostNotificationKind, RevertRange};
use signet_test_utils::chain::Chain;
use std::sync::Arc;

/// Convert a test [`ExExNotification`] into a [`HostNotification`].
///
/// Safe and finalized block numbers are set to `None` since the test
/// harness does not exercise block tag logic.
///
/// [`ExExNotification`]: signet_test_utils::specs::ExExNotification
pub fn to_host_notification(
    notif: &signet_test_utils::specs::ExExNotification,
) -> HostNotification<Chain> {
    let kind = match notif {
        signet_test_utils::specs::ExExNotification::Committed { new } => {
            HostNotificationKind::ChainCommitted { new: Arc::clone(new) }
        }
        signet_test_utils::specs::ExExNotification::Reorged { old, new } => {
            let old = RevertRange::new(old.first_number(), old.tip_number());
            HostNotificationKind::ChainReorged { old, new: Arc::clone(new) }
        }
        signet_test_utils::specs::ExExNotification::Reverted { old } => {
            let old = RevertRange::new(old.first_number(), old.tip_number());
            HostNotificationKind::ChainReverted { old }
        }
    };
    HostNotification { kind, safe_block_number: None, finalized_block_number: None }
}
