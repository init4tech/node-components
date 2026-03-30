use futures_util::Stream;
use std::{
    pin::Pin,
    task::{Context, Poll},
};

/// Stream adapter that coalesces buffered items, yielding only the
/// most recent ready item on each poll.
///
/// When the consumer is slow, items accumulate in the inner stream.
/// `Latest` drains all buffered ready items on each poll and returns
/// only the last one, discarding stale intermediates and recording a
/// metric for each skipped item.
pub(crate) struct Latest<S> {
    inner: S,
}

impl<S> Latest<S> {
    /// Wrap `inner` in a `Latest` combinator.
    pub(crate) const fn new(inner: S) -> Self {
        Self { inner }
    }
}

impl<S: std::fmt::Debug> std::fmt::Debug for Latest<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Latest").field("inner", &self.inner).finish()
    }
}

impl<S> Stream for Latest<S>
where
    S: Stream + Unpin,
{
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let inner = Pin::new(&mut self.inner);
        let first = match inner.poll_next(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(item) => item,
        };

        // Stream is exhausted or had no item.
        let Some(mut latest) = first else {
            return Poll::Ready(None);
        };

        // Drain all remaining ready items, keeping only the most recent.
        let mut skipped: u64 = 0;
        while let Poll::Ready(Some(newer)) = Pin::new(&mut self.inner).poll_next(cx) {
            latest = newer;
            skipped += 1;
        }

        if skipped > 0 {
            crate::metrics::inc_headers_coalesced(skipped);
        }

        Poll::Ready(Some(latest))
    }
}

#[cfg(test)]
mod tests {
    use super::Latest;
    use futures_util::{StreamExt, stream};

    #[tokio::test]
    async fn single_item_yields_immediately() {
        let mut s = Latest::new(stream::iter([42u32]));
        assert_eq!(s.next().await, Some(42));
        assert_eq!(s.next().await, None);
    }

    #[tokio::test]
    async fn multiple_ready_items_yields_last() {
        let mut s = Latest::new(stream::iter([1u32, 2, 3, 4, 5]));
        // All items are immediately ready; Latest should drain and return the last.
        assert_eq!(s.next().await, Some(5));
        assert_eq!(s.next().await, None);
    }

    #[tokio::test]
    async fn empty_stream_yields_none() {
        let mut s = Latest::new(stream::iter(Vec::<u32>::new()));
        assert_eq!(s.next().await, None);
    }

    #[tokio::test]
    async fn fused_after_inner_terminates() {
        let mut s = Latest::new(stream::iter([7u32]));
        assert_eq!(s.next().await, Some(7));
        // Subsequent polls after termination should return None.
        assert_eq!(s.next().await, None);
        assert_eq!(s.next().await, None);
    }
}
