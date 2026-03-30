use metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram};
use std::{sync::OnceLock, time::Duration};

// ── Metric name constants ──────────────────────────────────────────

const BLOCKS_FETCHED: &str = "host_rpc.blocks_fetched";
const REORGS: &str = "host_rpc.reorgs";
const WALK_EXHAUSTED: &str = "host_rpc.walk_exhausted";
const BACKFILL_BATCHES: &str = "host_rpc.backfill_batches";
const TAG_REFRESHES: &str = "host_rpc.tag_refreshes";
const STALE_HINTS: &str = "host_rpc.stale_hints";
const RPC_ERRORS: &str = "host_rpc.rpc_errors";
const HEADERS_COALESCED: &str = "host_rpc.headers_coalesced";

const WALK_CHAIN_DURATION: &str = "host_rpc.walk_chain.duration_ms";
const FETCH_BLOCK_DURATION: &str = "host_rpc.fetch_block.duration_ms";
const BACKFILL_BATCH_DURATION: &str = "host_rpc.backfill_batch.duration_ms";
const HANDLE_NEW_HEAD_DURATION: &str = "host_rpc.handle_new_head.duration_ms";
const REORG_DEPTH: &str = "host_rpc.reorg.depth";

const CHAIN_VIEW_LEN: &str = "host_rpc.chain_view.len";
const TIP_NUMBER: &str = "host_rpc.tip.number";

// ── Self-registering descriptions ──────────────────────────────────

static DESCRIBED: OnceLock<()> = OnceLock::new();

fn ensure_described() {
    DESCRIBED.get_or_init(|| {
        describe_counter!(BLOCKS_FETCHED, "Total blocks fetched via RPC");
        describe_counter!(REORGS, "Chain reorg events detected");
        describe_counter!(WALK_EXHAUSTED, "Walk exhausted the chain view buffer");
        describe_counter!(BACKFILL_BATCHES, "Backfill batches completed");
        describe_counter!(TAG_REFRESHES, "Epoch boundary tag refreshes");
        describe_counter!(STALE_HINTS, "Stale subscription hints that fell back to latest");
        describe_counter!(RPC_ERRORS, "RPC transport/provider errors");
        describe_counter!(HEADERS_COALESCED, "Stale subscription headers coalesced");
        describe_histogram!(WALK_CHAIN_DURATION, "Time to walk the chain (ms)");
        describe_histogram!(FETCH_BLOCK_DURATION, "Single block+receipts fetch (ms)");
        describe_histogram!(BACKFILL_BATCH_DURATION, "Full backfill batch (ms)");
        describe_histogram!(HANDLE_NEW_HEAD_DURATION, "Full new-head processing (ms)");
        describe_histogram!(REORG_DEPTH, "Number of blocks reverted in a reorg");
        describe_gauge!(CHAIN_VIEW_LEN, "Current chain view buffer size");
        describe_gauge!(TIP_NUMBER, "Current tip block number");
    });
}

// ── Helper functions ───────────────────────────────────────────────

/// Record the duration of a `walk_chain` call.
pub(crate) fn record_walk_duration(duration: Duration) {
    ensure_described();
    histogram!(WALK_CHAIN_DURATION).record(duration.as_secs_f64() * 1000.0);
}

/// Increment the walk-exhausted counter.
pub(crate) fn inc_walk_exhausted() {
    ensure_described();
    counter!(WALK_EXHAUSTED).increment(1);
}

/// Record a single block fetch duration.
pub(crate) fn record_fetch_block_duration(duration: Duration) {
    ensure_described();
    histogram!(FETCH_BLOCK_DURATION).record(duration.as_secs_f64() * 1000.0);
}

/// Record blocks fetched with a mode label.
pub(crate) fn inc_blocks_fetched(count: u64, mode: &'static str) {
    ensure_described();
    counter!(BLOCKS_FETCHED, "mode" => mode).increment(count);
}

/// Record a reorg event with its depth.
pub(crate) fn inc_reorgs(depth: u64) {
    ensure_described();
    counter!(REORGS).increment(1);
    histogram!(REORG_DEPTH).record(depth as f64);
}

/// Increment the stale-hints counter.
pub(crate) fn inc_stale_hints() {
    ensure_described();
    counter!(STALE_HINTS).increment(1);
}

/// Increment the RPC errors counter.
pub(crate) fn inc_rpc_errors() {
    ensure_described();
    counter!(RPC_ERRORS).increment(1);
}

/// Record a backfill batch completion.
pub(crate) fn record_backfill_batch(duration: Duration) {
    ensure_described();
    counter!(BACKFILL_BATCHES).increment(1);
    histogram!(BACKFILL_BATCH_DURATION).record(duration.as_secs_f64() * 1000.0);
}

/// Record `handle_new_head` duration.
pub(crate) fn record_handle_new_head_duration(duration: Duration) {
    ensure_described();
    histogram!(HANDLE_NEW_HEAD_DURATION).record(duration.as_secs_f64() * 1000.0);
}

/// Increment the tag-refreshes counter.
pub(crate) fn inc_tag_refreshes() {
    ensure_described();
    counter!(TAG_REFRESHES).increment(1);
}

/// Update the chain view length gauge.
pub(crate) fn set_chain_view_len(len: usize) {
    ensure_described();
    gauge!(CHAIN_VIEW_LEN).set(len as f64);
}

/// Update the tip block number gauge.
pub(crate) fn set_tip(number: u64) {
    ensure_described();
    gauge!(TIP_NUMBER).set(number as f64);
}

/// Increment the headers-coalesced counter.
pub(crate) fn inc_headers_coalesced(count: u64) {
    ensure_described();
    counter!(HEADERS_COALESCED).increment(count);
}
