//! Filter kinds for subscriptions and polling filters.

use crate::interest::{NewBlockNotification, filters::FilterOutput, subs::SubscriptionBuffer};
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
}
