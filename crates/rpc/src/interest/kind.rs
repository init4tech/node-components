//! Filter kinds for subscriptions and polling filters.

use crate::interest::{
    NewBlockNotification, ReorgNotification, filters::FilterOutput, subs::SubscriptionBuffer,
};
use alloy::rpc::types::{Filter, Header, Log};
use std::collections::VecDeque;

/// The different kinds of filters that can be created.
///
/// Pending tx filters are not supported by Signet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InterestKind {
    /// Log filter with a user-supplied [`Filter`].
    Log(Box<Filter>),
    /// New-block filter.
    Block,
}

impl InterestKind {
    /// True if this is a block filter.
    pub(crate) const fn is_block(&self) -> bool {
        matches!(self, Self::Block)
    }

    /// Fallible cast to a filter.
    pub(crate) const fn as_filter(&self) -> Option<&Filter> {
        match self {
            Self::Log(f) => Some(f),
            _ => None,
        }
    }

    fn apply_block(notif: &NewBlockNotification) -> SubscriptionBuffer {
        let header = Header {
            hash: notif.header.hash_slow(),
            inner: notif.header.clone(),
            total_difficulty: None,
            size: None,
        };
        SubscriptionBuffer::Block(VecDeque::from([header]))
    }

    fn apply_filter(&self, notif: &NewBlockNotification) -> SubscriptionBuffer {
        let filter = self.as_filter().unwrap();
        let block_hash = notif.header.hash_slow();
        let block_number = notif.header.number;
        let block_timestamp = notif.header.timestamp;

        let logs: VecDeque<Log> = notif
            .receipts
            .iter()
            .enumerate()
            .flat_map(|(tx_idx, receipt)| {
                let tx_hash = *notif.transactions[tx_idx].tx_hash();
                receipt.inner.logs.iter().map(move |log| (tx_idx, tx_hash, log))
            })
            .enumerate()
            .filter(|(_, (_, _, log))| filter.matches(log))
            .map(|(log_idx, (tx_idx, tx_hash, log))| Log {
                inner: log.clone(),
                block_hash: Some(block_hash),
                block_number: Some(block_number),
                block_timestamp: Some(block_timestamp),
                transaction_hash: Some(tx_hash),
                transaction_index: Some(tx_idx as u64),
                log_index: Some(log_idx as u64),
                removed: false,
            })
            .collect();

        SubscriptionBuffer::Log(logs)
    }

    /// Apply the filter to a [`NewBlockNotification`], producing a
    /// subscription buffer.
    pub(crate) fn filter_notification_for_sub(
        &self,
        notif: &NewBlockNotification,
    ) -> SubscriptionBuffer {
        if self.is_block() { Self::apply_block(notif) } else { self.apply_filter(notif) }
    }

    /// Return an empty output of the same kind as this filter.
    pub(crate) const fn empty_output(&self) -> FilterOutput {
        match self {
            Self::Log(_) => FilterOutput::Log(VecDeque::new()),
            Self::Block => FilterOutput::Block(VecDeque::new()),
        }
    }

    /// Return an empty subscription buffer of the same kind as this filter.
    pub(crate) const fn empty_sub_buffer(&self) -> SubscriptionBuffer {
        match self {
            Self::Log(_) => SubscriptionBuffer::Log(VecDeque::new()),
            Self::Block => SubscriptionBuffer::Block(VecDeque::new()),
        }
    }

    /// Filter a reorg notification for a subscription, producing a buffer of
    /// removed logs (with `removed: true`) that match this filter.
    ///
    /// Block subscriptions return an empty buffer — the Ethereum JSON-RPC
    /// spec does not push removed headers for `newHeads` subscriptions.
    pub(crate) fn filter_reorg_for_sub(&self, reorg: ReorgNotification) -> SubscriptionBuffer {
        let Some(filter) = self.as_filter() else {
            return self.empty_sub_buffer();
        };

        let logs: VecDeque<Log> = reorg
            .removed_blocks
            .iter()
            .flat_map(|block| {
                let hash = block.hash;
                let number = block.number;
                let timestamp = block.timestamp;
                block.logs.iter().map(move |log| (hash, number, timestamp, log))
            })
            .filter(|(_, _, _, log)| filter.matches(log))
            .map(|(hash, number, timestamp, log)| Log {
                inner: log.clone(),
                block_hash: Some(hash),
                block_number: Some(number),
                block_timestamp: Some(timestamp),
                transaction_hash: None,
                transaction_index: None,
                log_index: None,
                removed: true,
            })
            .collect();

        SubscriptionBuffer::Log(logs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{Address, B256, Bytes, LogData, address, b256};

    fn test_log(addr: Address, topic: B256) -> alloy::primitives::Log {
        alloy::primitives::Log {
            address: addr,
            data: LogData::new_unchecked(vec![topic], Bytes::new()),
        }
    }

    fn test_filter(addr: Address) -> Filter {
        Filter::new().address(addr)
    }

    fn test_removed_block(
        number: u64,
        hash: B256,
        logs: Vec<alloy::primitives::Log>,
    ) -> crate::interest::RemovedBlock {
        crate::interest::RemovedBlock { number, hash, timestamp: 1_000_000 + number, logs }
    }

    #[test]
    fn filter_reorg_for_sub_matches_logs() {
        let addr = address!("0x0000000000000000000000000000000000000001");
        let topic = b256!("0x0000000000000000000000000000000000000000000000000000000000000001");
        let block_hash =
            b256!("0x0000000000000000000000000000000000000000000000000000000000000099");

        let kind = InterestKind::Log(Box::new(test_filter(addr)));
        let reorg = ReorgNotification {
            common_ancestor: 10,
            removed_blocks: vec![test_removed_block(11, block_hash, vec![test_log(addr, topic)])],
        };

        let buf = kind.filter_reorg_for_sub(reorg);
        let SubscriptionBuffer::Log(logs) = buf else { panic!("expected Log buffer") };

        assert_eq!(logs.len(), 1);
        assert!(logs[0].removed);
        assert_eq!(logs[0].inner.address, addr);
        assert_eq!(logs[0].block_hash, Some(block_hash));
        assert_eq!(logs[0].block_number, Some(11));
        assert_eq!(logs[0].block_timestamp, Some(1_000_011));
    }

    #[test]
    fn filter_reorg_for_sub_filters_non_matching() {
        let addr = address!("0x0000000000000000000000000000000000000001");
        let other = address!("0x0000000000000000000000000000000000000002");
        let topic = b256!("0x0000000000000000000000000000000000000000000000000000000000000001");
        let block_hash =
            b256!("0x0000000000000000000000000000000000000000000000000000000000000099");

        let kind = InterestKind::Log(Box::new(test_filter(addr)));
        let reorg = ReorgNotification {
            common_ancestor: 10,
            removed_blocks: vec![test_removed_block(11, block_hash, vec![test_log(other, topic)])],
        };

        let buf = kind.filter_reorg_for_sub(reorg);
        let SubscriptionBuffer::Log(logs) = buf else { panic!("expected Log buffer") };
        assert!(logs.is_empty());
    }

    #[test]
    fn filter_reorg_for_sub_block_returns_empty() {
        let block_hash =
            b256!("0x0000000000000000000000000000000000000000000000000000000000000001");

        let reorg = ReorgNotification {
            common_ancestor: 10,
            removed_blocks: vec![test_removed_block(11, block_hash, vec![])],
        };

        let buf = InterestKind::Block.filter_reorg_for_sub(reorg);
        assert!(buf.is_empty());
    }
}
