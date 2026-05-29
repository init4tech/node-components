//! Journal-based sync: applying journals received from upstream sources to
//! local hot + cold storage, as an alternative to executing host blocks.

mod ingestor;
pub(crate) use ingestor::JournalIngestor;
mod runtime;
pub(crate) use runtime::{
    JOURNAL_SYNC_BACKPRESSURE_CAPACITY, JournalExitKind, RunningJournalSync, SyncOutcome,
    build_journal_client, collapse_sync_failure, is_caught_up_to_host, journal_sync_loop,
    journal_task_result, seed_journal_checkpoints,
};
