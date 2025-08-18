use crate::{DbExtractionResults, DbSignetEvent, RuChain};
use alloy::primitives::{Address, B256, BlockNumber, U256};
use itertools::Itertools;
use reth::{
    primitives::Account,
    providers::{DatabaseProviderRW, OriginalValuesKnown, ProviderResult, StorageLocation},
};
use reth_db::models::StoredBlockBodyIndices;
use signet_evm::BlockResult;
use signet_node_types::{NodeTypesDbTrait, SignetNodeTypes};
use signet_types::primitives::RecoveredBlock;
use signet_zenith::{Passage, Transactor, Zenith};
use std::{collections::BTreeMap, ops::RangeInclusive};
use tracing::trace;

/// Writer for [`Passage::Enter`] events.
#[auto_impl::auto_impl(&, Arc, Box)]
pub trait RuWriter {
    /// Get the last block number
    fn last_block_number(&self) -> ProviderResult<BlockNumber>;

    /// Insert a journal hash into the DB.
    fn insert_journal_hash(&self, rollup_height: u64, hash: B256) -> ProviderResult<()>;

    /// Remove a journal hash from the DB.
    fn remove_journal_hash(&self, rollup_height: u64) -> ProviderResult<()>;

    /// Get a journal hash from the DB.
    fn get_journal_hash(&self, rollup_height: u64) -> ProviderResult<Option<B256>>;

    /// Get the latest journal hash from the DB.
    fn latest_journal_hash(&self) -> ProviderResult<B256>;

    /// Increase the balance of an account.
    fn mint_eth(&self, address: Address, amount: U256) -> ProviderResult<Account>;

    /// Decrease the balance of an account.
    fn burn_eth(&self, address: Address, amount: U256) -> ProviderResult<Account>;

    /// Store a zenith header in the DB
    fn insert_signet_header(
        &self,
        header: Zenith::BlockHeader,
        host_height: u64,
    ) -> ProviderResult<()>;

    /// Get a Zenith header from the DB.
    fn get_signet_header(&self, host_height: u64) -> ProviderResult<Option<Zenith::BlockHeader>>;

    /// Store a zenith block in the DB.
    fn insert_signet_block(
        &self,
        header: Option<Zenith::BlockHeader>,
        block: &RecoveredBlock,
        journal_hash: B256,
        write_to: StorageLocation,
    ) -> ProviderResult<StoredBlockBodyIndices>;

    /// Append a zenith block body to the DB.
    fn append_signet_block_body(
        &self,
        body: (BlockNumber, &RecoveredBlock),
        write_to: StorageLocation,
    ) -> ProviderResult<()>;

    /// Get zenith headers from the DB.
    fn get_signet_headers(
        &self,
        range: RangeInclusive<BlockNumber>,
    ) -> ProviderResult<Vec<(BlockNumber, Zenith::BlockHeader)>>;

    /// Take zenith headers from the DB.
    fn take_signet_headers_above(
        &self,
        target: BlockNumber,
        remove_from: StorageLocation,
    ) -> ProviderResult<Vec<(BlockNumber, Zenith::BlockHeader)>>;

    /// Remove [`Zenith::BlockHeader`] objects above the specified height from the DB.
    fn remove_signet_headers_above(
        &self,
        target: BlockNumber,
        remove_from: StorageLocation,
    ) -> ProviderResult<()>;

    /// Store an enter event in the DB.
    fn insert_enter(&self, height: u64, index: u64, exit: Passage::Enter) -> ProviderResult<()>;

    /// Get enters from the DB.
    fn get_enters(
        &self,
        range: RangeInclusive<BlockNumber>,
    ) -> ProviderResult<Vec<(BlockNumber, Passage::Enter)>> {
        Ok(self
            .get_signet_events(range)?
            .into_iter()
            .filter_map(|(height, events)| {
                if let DbSignetEvent::Enter(_, enter) = events {
                    Some((height, enter))
                } else {
                    None
                }
            })
            .collect())
    }

    /// Store a transaction event in the DB.
    fn insert_transact(
        &self,
        height: u64,
        index: u64,
        transact: &Transactor::Transact,
    ) -> ProviderResult<()>;

    /// Get [`Transactor::Transact`] from the DB.
    fn get_transacts(
        &self,
        range: RangeInclusive<BlockNumber>,
    ) -> ProviderResult<Vec<(BlockNumber, Transactor::Transact)>> {
        Ok(self
            .get_signet_events(range)?
            .into_iter()
            .filter_map(|(height, events)| {
                if let DbSignetEvent::Transact(_, transact) = events {
                    Some((height, transact))
                } else {
                    None
                }
            })
            .collect())
    }

    /// Insert [`Passage::EnterToken`] into the DB.
    fn insert_enter_token(
        &self,
        height: u64,
        index: u64,
        enter_token: Passage::EnterToken,
    ) -> ProviderResult<()>;

    /// Get [`Passage::EnterToken`] from the DB.
    fn get_enter_tokens(
        &self,
        range: RangeInclusive<BlockNumber>,
    ) -> ProviderResult<Vec<(BlockNumber, Passage::EnterToken)>> {
        Ok(self
            .get_signet_events(range)?
            .into_iter()
            .filter_map(|(height, events)| {
                if let DbSignetEvent::EnterToken(_, enter) = events {
                    Some((height, enter))
                } else {
                    None
                }
            })
            .collect())
    }

    /// Get [`Passage::EnterToken`], [`Passage::Enter`] and
    /// [`Transactor::Transact`] events.
    fn get_signet_events(
        &self,
        range: RangeInclusive<BlockNumber>,
    ) -> ProviderResult<Vec<(BlockNumber, DbSignetEvent)>>;

    /// Take [`Passage::EnterToken`]s from the DB.
    fn take_signet_events_above(
        &self,
        target: BlockNumber,
        remove_from: StorageLocation,
    ) -> ProviderResult<Vec<(BlockNumber, DbSignetEvent)>>;

    /// Remove [`Passage::EnterToken`], [`Passage::Enter`] and
    /// [`Transactor::Transact`] events above the specified height from the DB.
    fn remove_signet_events_above(
        &self,
        target: BlockNumber,
        remove_from: StorageLocation,
    ) -> ProviderResult<()>;

    /// Get extraction results from the DB.
    fn get_extraction_results(
        &self,
        range: RangeInclusive<BlockNumber>,
    ) -> ProviderResult<BTreeMap<BlockNumber, DbExtractionResults>> {
        let mut signet_events = self.get_signet_events(range.clone())?.into_iter().peekable();
        let mut headers = self.get_signet_headers(range.clone())?.into_iter().peekable();

        // For each of these, it is permissible to have no entries. If there is
        // no data. The`DbExtractionResults` struct will contain a `None`
        // header, or an empty vector for the other fields.
        let mut items = BTreeMap::new();
        for working_height in range.clone() {
            let mut enters = vec![];
            let mut transacts = vec![];
            let mut enter_tokens = vec![];

            for (_, event) in
                signet_events.peeking_take_while(|(height, _)| *height == working_height)
            {
                match event {
                    DbSignetEvent::Enter(_, enter) => enters.push(enter),
                    DbSignetEvent::Transact(_, transact) => transacts.push(transact),
                    DbSignetEvent::EnterToken(_, enter_token) => enter_tokens.push(enter_token),
                }
            }

            let header = headers
                .peeking_take_while(|(height, _)| *height == working_height)
                .map(|(_, header)| header)
                .next();

            items.insert(
                working_height,
                DbExtractionResults { header, enters, transacts, enter_tokens },
            );
        }

        Ok(items)
    }

    /// Take extraction results from the DB.
    fn take_extraction_results_above(
        &self,
        target: BlockNumber,
        remove_from: StorageLocation,
    ) -> ProviderResult<BTreeMap<BlockNumber, DbExtractionResults>> {
        let range = target..=(1 + self.last_block_number()?);

        let items = self.get_extraction_results(range)?;
        trace!(count = items.len(), "got extraction results");
        self.remove_extraction_results_above(target, remove_from)?;
        trace!("removed extraction results");
        Ok(items)
    }

    /// Remove extraction results from the DB.
    ///
    /// This will remove the following:
    /// - [`Zenith::BlockHeader`] objects
    /// - [`Passage::Enter`] events
    /// - [`Transactor::Transact`] events
    /// - [`Passage::EnterToken`] events
    fn remove_extraction_results_above(
        &self,
        target: BlockNumber,
        remove_from: StorageLocation,
    ) -> ProviderResult<()> {
        self.remove_signet_headers_above(target, remove_from)?;
        self.remove_signet_events_above(target, remove_from)?;
        Ok(())
    }

    /// Add the output of a host block to the DB.
    #[allow(clippy::too_many_arguments)]
    fn append_host_block(
        &self,
        header: Option<Zenith::BlockHeader>,
        transacts: impl IntoIterator<Item = Transactor::Transact>,
        enters: impl IntoIterator<Item = Passage::Enter>,
        enter_tokens: impl IntoIterator<Item = Passage::EnterToken>,
        block_result: &BlockResult,
        journal_hash: B256,
    ) -> ProviderResult<()>;

    /// Take the block and execution range from the DB, reverting the blocks
    /// and returning the removed information
    fn ru_take_blocks_and_execution_above(
        &self,
        target: BlockNumber,
        remove_from: StorageLocation,
    ) -> ProviderResult<RuChain>;

    /// Remove the block and execution range from the DB.
    fn ru_remove_blocks_and_execution_above(
        &self,
        target: BlockNumber,
        remove_from: StorageLocation,
    ) -> ProviderResult<()>;

    /// Write the state of the rollup to the database.
    ///
    /// This should be identical to [`StateWriter::write_state`], but using a
    /// [`signet_evm::ExecutionOutcome`].
    ///
    /// [`StateWriter::write_state`]: reth::providers::StateWriter::write_state
    fn ru_write_state(
        &self,
        execution_outcome: &signet_evm::ExecutionOutcome,
        is_value_known: OriginalValuesKnown,
        write_receipts_to: StorageLocation,
    ) -> ProviderResult<()>;
}

/// Extend the [`DatabaseProviderRW`] with a guarded commit function.
pub trait DbProviderExt<Db>: Into<DatabaseProviderRW<Db, SignetNodeTypes<Db>>>
where
    Db: NodeTypesDbTrait,
{
    /// Update the database. The function `f` is called with a mutable
    /// reference to the database. If the function returns an error, the
    /// transaction is rolled back.
    fn update(
        self,
        f: impl FnOnce(&mut DatabaseProviderRW<Db, SignetNodeTypes<Db>>) -> ProviderResult<()>,
    ) -> ProviderResult<()>;
}

impl<T, Db> DbProviderExt<Db> for T
where
    Db: NodeTypesDbTrait,
    T: Into<DatabaseProviderRW<Db, SignetNodeTypes<Db>>>,
{
    fn update(
        self,
        f: impl FnOnce(&mut DatabaseProviderRW<Db, SignetNodeTypes<Db>>) -> ProviderResult<()>,
    ) -> ProviderResult<()> {
        let mut this = self.into();
        f(&mut this)?;
        this.commit().map(drop)
    }
}
