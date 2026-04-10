use metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram};
use std::{sync::OnceLock, time::Duration};

const BLOCKS_FETCHED: &str = "host_reth.db_backfill.blocks_fetched";
const BATCHES_COMPLETED: &str = "host_reth.db_backfill.batches_completed";
const BATCH_DURATION: &str = "host_reth.db_backfill.batch_duration_ms";
const CURSOR_POSITION: &str = "host_reth.db_backfill.cursor";

static DESCRIBED: OnceLock<()> = OnceLock::new();

fn ensure_described() {
    DESCRIBED.get_or_init(|| {
        describe_counter!(BLOCKS_FETCHED, "Total blocks read from DB during backfill");
        describe_counter!(BATCHES_COMPLETED, "DB backfill batches completed");
        describe_histogram!(BATCH_DURATION, "DB backfill batch duration (ms)");
        describe_gauge!(CURSOR_POSITION, "Current DB backfill cursor position");
    });
}

/// Record a completed backfill batch.
pub(crate) fn record_backfill_batch(blocks: u64, duration: Duration) {
    ensure_described();
    counter!(BLOCKS_FETCHED).increment(blocks);
    counter!(BATCHES_COMPLETED).increment(1);
    histogram!(BATCH_DURATION).record(duration.as_secs_f64() * 1000.0);
}

/// Update the backfill cursor gauge.
pub(crate) fn set_backfill_cursor(position: u64) {
    ensure_described();
    gauge!(CURSOR_POSITION).set(position as f64);
}
