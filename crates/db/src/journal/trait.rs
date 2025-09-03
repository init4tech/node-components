use crate::RuWriter;
use alloy::consensus::{BlockHeader, Header};
use reth::{providers::ProviderResult, revm::db::BundleState};
use signet_evm::{BlockResult, ExecutionOutcome};
use signet_journal::HostJournal;
use signet_types::primitives::{RecoveredBlock, SealedBlock, SealedHeader, TransactionSigned};

/// A database that can be updated with journals.
pub trait JournalDb: RuWriter {
    /// Ingest a journal into the database.
    ///
    /// This will create a [`BlockResult`] from the provided header and update,
    /// and append it to the database using [`RuWriter::append_host_block`].
    ///
    /// This DOES NOT update tables containing historical transactions,
    /// receipts, events, etc. It only updates tables related to headers,
    /// and state.
    ///
    /// This is intended to be used for tx simulation, and other purposes that
    /// need fast state access WITHTOUT needing to retrieve historical data.
    fn ingest(&self, journal: HostJournal<'static>) -> ProviderResult<()> {
        let journal_hash = journal.journal_hash();

        let (meta, bsi) = journal.into_parts();
        let (_, _, header) = meta.into_parts();

        // TODO: remove the clone in future versions. This can be achieved by
        // _NOT_ making a `BlockResult` and instead manually updating relevan
        // tables. However, this means diverging more fro the underlying reth
        // logic that we are currently re-using.
        let bundle_state: BundleState = bsi.into();
        let execution_outcome = ExecutionOutcome::new(bundle_state, vec![], header.number());

        let block: SealedBlock<TransactionSigned, Header> =
            SealedBlock { header: SealedHeader::new(header), body: Default::default() };
        let block_result =
            BlockResult { sealed_block: RecoveredBlock::new(block, vec![]), execution_outcome };

        self.append_host_block(
            None,
            std::iter::empty(),
            std::iter::empty(),
            std::iter::empty(),
            &block_result,
            journal_hash,
        )?;

        Ok(())
    }
}

impl<T> JournalDb for T where T: RuWriter {}
