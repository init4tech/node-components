use crate::{
    DataCompat, DbZenithHeader, RuChain, ZenithHeaders,
    tables::{DbSignetEvent, JournalHashes, SignetEvents},
    traits::RuWriter,
};
use alloy::{
    consensus::{BlockHeader, TxReceipt},
    primitives::{Address, B256, BlockNumber, U256, map::HashSet},
};
use reth::{
    primitives::{Account, StaticFileSegment},
    providers::{
        AccountReader, BlockBodyIndicesProvider, BlockNumReader, BlockReader, BlockWriter, Chain,
        DBProvider, DatabaseProviderRW, HistoryWriter, OriginalValuesKnown, ProviderError,
        ProviderResult, StageCheckpointWriter, StateWriter, StaticFileProviderFactory,
        StaticFileWriter, StorageLocation,
    },
};
use reth_db::{
    PlainAccountState,
    cursor::{DbCursorRO, DbCursorRW},
    models::{BlockNumberAddress, StoredBlockBodyIndices},
    tables,
    transaction::{DbTx, DbTxMut},
};
use reth_prune_types::{MINIMUM_PRUNING_DISTANCE, PruneMode};
use signet_evm::BlockResult;
use signet_node_types::{NodeTypesDbTrait, SignetNodeTypes};
use signet_types::primitives::RecoveredBlock;
use signet_zenith::{
    Passage::{self, Enter, EnterToken},
    Transactor::Transact,
    Zenith,
};
use std::ops::RangeInclusive;
use tracing::{debug, instrument, trace, warn};

impl<Db> RuWriter for DatabaseProviderRW<Db, SignetNodeTypes<Db>>
where
    Db: NodeTypesDbTrait,
{
    fn last_block_number(&self) -> ProviderResult<BlockNumber> {
        BlockNumReader::last_block_number(&self.0)
    }

    fn insert_journal_hash(&self, ru_height: u64, hash: B256) -> ProviderResult<()> {
        self.tx_ref().put::<JournalHashes>(ru_height, hash)?;
        Ok(())
    }

    fn remove_journal_hash(&self, ru_height: u64) -> ProviderResult<()> {
        self.tx_ref().delete::<JournalHashes>(ru_height, None)?;
        Ok(())
    }

    fn get_journal_hash(&self, ru_height: u64) -> ProviderResult<Option<B256>> {
        self.tx_ref().get::<JournalHashes>(ru_height).map_err(Into::into)
    }

    fn latest_journal_hash(&self) -> ProviderResult<B256> {
        let latest_height = self.last_block_number()?;
        Ok(self
            .get_journal_hash(latest_height)?
            .expect("DB in corrupt state. Missing Journal Hash for latest height"))
    }

    /// Insert an enter into the DB
    /// This is a signet-specific function that inserts an enter event into the
    /// [`SignetEvents`] table.
    fn insert_enter(&self, ru_height: u64, index: u64, enter: Enter) -> ProviderResult<()> {
        self.tx_ref()
            .put::<SignetEvents>(ru_height, DbSignetEvent::Enter(index, enter))
            .map_err(Into::into)
    }

    /// Insert an enter token event into the DB
    /// This is a signet-specific function that inserts an enter token event
    /// into the [`SignetEvents`] table.
    fn insert_enter_token(
        &self,
        ru_height: u64,
        index: u64,
        enter_token: EnterToken,
    ) -> ProviderResult<()> {
        self.tx_ref()
            .put::<SignetEvents>(ru_height, DbSignetEvent::EnterToken(index, enter_token))?;
        Ok(())
    }

    /// Insert a Transact into the DB
    /// This is a signet-specific function that inserts a transact event into the
    /// [`SignetEvents`] table.
    fn insert_transact(
        &self,
        ru_height: u64,
        index: u64,
        transact: &Transact,
    ) -> ProviderResult<()> {
        // this is unfortunate, but probably fine because the large part is the
        // shared Bytes object.
        let t = transact.clone();
        self.tx_ref()
            .put::<SignetEvents>(ru_height, DbSignetEvent::Transact(index, t))
            .map_err(Into::into)
    }

    /// Increase the balance of an account.
    fn mint_eth(&self, address: Address, amount: U256) -> ProviderResult<Account> {
        let mut account = self.basic_account(&address)?.unwrap_or_default();
        account.balance = account.balance.saturating_add(amount);
        self.tx_ref().put::<PlainAccountState>(address, account)?;
        trace!(%address, balance = %account.balance, "minting ETH");
        Ok(account)
    }

    /// Decrease the balance of an account.
    fn burn_eth(&self, address: Address, amount: U256) -> ProviderResult<Account> {
        let mut account = self.basic_account(&address)?.unwrap_or_default();
        if amount > account.balance {
            warn!(
                balance = %account.balance,
                amount = %amount,
                "burning more than balance"
            );
        }
        account.balance = account.balance.saturating_sub(amount);
        self.tx_ref().put::<PlainAccountState>(address, account)?;
        trace!(%address, balance = %account.balance, "burning ETH");
        Ok(account)
    }

    fn insert_signet_header(
        &self,
        header: Zenith::BlockHeader,
        ru_height: u64,
    ) -> ProviderResult<()> {
        self.tx_ref().put::<ZenithHeaders>(ru_height, header.into())?;

        Ok(())
    }

    fn get_signet_header(&self, ru_height: u64) -> ProviderResult<Option<Zenith::BlockHeader>> {
        self.tx_ref().get::<ZenithHeaders>(ru_height).map(|h| h.map(Into::into)).map_err(Into::into)
    }

    /// Inserts the zenith block into the database, always modifying the following tables:
    /// * [`JournalHashes`]
    /// * [`CanonicalHeaders`](tables::CanonicalHeaders)
    /// * [`Headers`](tables::Headers)
    /// * [`HeaderTerminalDifficulties`](tables::HeaderTerminalDifficulties)
    /// * [`HeaderNumbers`](tables::HeaderNumbers)
    /// * [`BlockBodyIndices`](tables::BlockBodyIndices) (through
    ///   [`RuWriter::append_signet_block_body`])
    ///
    /// If there are transactions in the block, the following tables will be
    /// modified:
    /// * [`Transactions`](tables::Transactions) (through
    ///   [`RuWriter::append_signet_block_body`])
    /// * [`TransactionBlocks`](tables::TransactionBlocks) (through
    ///   [`RuWriter::append_signet_block_body`])
    ///
    /// If the provider has __not__ configured full sender pruning, this will
    /// modify [`TransactionSenders`](tables::TransactionSenders).
    ///
    /// If the provider has __not__ configured full transaction lookup pruning,
    /// this will modify [`TransactionHashNumbers`](tables::TransactionHashNumbers).
    ///
    /// Ommers and withdrawals are not inserted, as Signet does not use them.
    fn insert_signet_block(
        &self,
        header: Option<Zenith::BlockHeader>,
        block: &RecoveredBlock,
        journal_hash: B256,
        write_to: StorageLocation,
    ) -> ProviderResult<StoredBlockBodyIndices> {
        // Implementation largely copied from
        // `BlockWriter::insert_block`
        // in `reth/crates/storage/provider/src/providers/database/provider.rs`
        // duration metrics have been removed
        //
        // Last reviewed at tag v1.5.1
        let block_number = block.number();
        if let Some(header) = header {
            self.insert_signet_header(header, block_number)?;
        }

        // Put journal hash into the DB
        self.tx_ref().put::<crate::JournalHashes>(block_number, journal_hash)?;

        let block_hash = block.block.header.hash();
        let block_header = block.block.header.header();

        if write_to.database() {
            self.tx_ref().put::<tables::CanonicalHeaders>(block_number, block_hash)?;
            self.tx_ref().put::<tables::Headers>(block_number, block_header.clone())?;
            // NB: while this is meaningless for zenith blocks, it is necessary for
            // the RPC server to function properly. If the TTD for a block is not
            // set, the RPC server will return an error indicating that the block
            // is not found.
            self.tx_ref()
                .put::<tables::HeaderTerminalDifficulties>(block_number, U256::ZERO.into())?;
        }

        if write_to.static_files() {
            let sf = self.static_file_provider();
            let mut writer = sf.get_writer(block_number, StaticFileSegment::Headers)?;
            writer.append_header(block_header, U256::ZERO, &block_hash)?;
        }

        // Append the block number corresponding to a header on the DB
        self.tx_ref().put::<tables::HeaderNumbers>(block_hash, block_number)?;

        let mut next_tx_num = self
            .tx_ref()
            .cursor_read::<tables::TransactionBlocks>()?
            .last()?
            .map(|(n, _)| n + 1)
            .unwrap_or_default();
        let first_tx_num = next_tx_num;
        let tx_count = block.block.body.transactions.len() as u64;

        for (sender, transaction) in block.senders.iter().zip(block.block.body.transactions()) {
            let hash = *transaction.hash();
            debug_assert_ne!(hash, B256::ZERO, "transaction hash is zero");

            if self.prune_modes_ref().sender_recovery.as_ref().is_none_or(|m| !m.is_full()) {
                self.tx_ref().put::<tables::TransactionSenders>(next_tx_num, *sender)?;
            }

            if self.prune_modes_ref().transaction_lookup.is_none_or(|m| !m.is_full()) {
                self.tx_ref().put::<tables::TransactionHashNumbers>(hash, next_tx_num)?;
            }

            next_tx_num += 1;
        }

        self.append_signet_block_body((block_number, block), write_to)?;

        debug!(?block_number, "Inserted block");

        Ok(StoredBlockBodyIndices { first_tx_num, tx_count })
    }

    /// Appends the body of a signet block to the database.
    fn append_signet_block_body(
        &self,
        body: (BlockNumber, &RecoveredBlock),
        write_to: StorageLocation,
    ) -> ProviderResult<()> {
        // Implementation largely copied from
        // `DatabaseProvider::append_block_bodies`
        // in `reth/crates/storage/provider/src/providers/database/provider.rs`
        // duration metrics have been removed, and the implementation has been
        // modified to work with a single signet block.
        //
        // last reviewed at tag v1.5.1

        let sf = self.static_file_provider();

        // Initialize writer if we will be writing transactions to staticfiles
        let mut tx_static_writer = write_to
            .static_files()
            .then(|| sf.get_writer(body.0, StaticFileSegment::Transactions))
            .transpose()?;

        let mut block_indices_cursor = self.tx_ref().cursor_write::<tables::BlockBodyIndices>()?;
        let mut tx_block_cursor = self.tx_ref().cursor_write::<tables::TransactionBlocks>()?;

        // Initialize curosr if we will be writing transactions to database
        let mut tx_cursor = write_to
            .database()
            .then(|| self.tx_ref().cursor_write::<tables::Transactions>())
            .transpose()?;

        let block_number = body.0;
        let block = body.1;

        // Get id for the next tx_num or zero if there are no transactions.
        let mut next_tx_num = tx_block_cursor.last()?.map(|(n, _)| n + 1).unwrap_or_default();

        // Increment block on static file header if we're writing to static files.
        if let Some(writer) = tx_static_writer.as_mut() {
            writer.increment_block(block_number)?;
        }

        let tx_count = block.block.body.transactions.len();
        let block_indices =
            StoredBlockBodyIndices { first_tx_num: next_tx_num, tx_count: tx_count as u64 };

        // insert block meta
        block_indices_cursor.append(block_number, &block_indices)?;

        // write transaction block index
        if tx_count != 0 {
            tx_block_cursor.append(block_indices.last_tx_num(), &block_number)?;
        }

        // Write transactions
        for transaction in block.block.body.transactions() {
            if let Some(writer) = tx_static_writer.as_mut() {
                writer.append_transaction(next_tx_num, transaction)?;
            }

            if let Some(cursor) = tx_cursor.as_mut() {
                cursor.append(next_tx_num, transaction)?;
            }

            // Increment transaction id for each transaction
            next_tx_num += 1;
        }

        debug!(
            target: "signet_db_lifecycle",
            ?block_number,
            "Inserted block body"
        );

        // NB: Here we'd usually write ommers and withdrawals, via
        // `write_block_bodies` (which does not write txns, as you might
        // expect). Signet doesn't have ommers or withdrawals. Therefore we're
        // able to just return.
        Ok(())
    }

    fn get_signet_headers(
        &self,
        range: RangeInclusive<BlockNumber>,
    ) -> ProviderResult<Vec<(BlockNumber, Zenith::BlockHeader)>> {
        // Implementation largely copied from
        // `DatabaseProvider::get_or_take`
        // in `reth/crates/storage/provider/src/providers/database/provider.rs`
        // which after 1.1.3, has been removed and its functionality inlined.
        //
        // We have to customize the impl to unwrap the DbZenithHeader
        let mut items = Vec::new();

        trace!(target: "signet_db_lifecycle", "getting zenith headers");
        let mut cursor = self.tx_ref().cursor_read::<ZenithHeaders>()?;
        let mut walker = cursor.walk_range(range)?;

        while let Some((k, DbZenithHeader(e))) = walker.next().transpose()? {
            items.push((k, e))
        }

        Ok(items)
    }

    /// Take zenith headers from the DB.
    fn take_signet_headers_above(
        &self,
        target: BlockNumber,
        _remove_from: StorageLocation,
    ) -> ProviderResult<Vec<(BlockNumber, Zenith::BlockHeader)>> {
        // Implementation largely copied from
        // `DatabaseProvider::get_or_take`
        // in `reth/crates/storage/provider/src/providers/database/provider.rs`
        // which after 1.1.3, has been removed and the functionality inlined.

        // We have to customize the impl to unwrap the DB enters
        let mut items = Vec::new();
        trace!(target: "signet_db_lifecycle", "taking zenith headers");
        let mut cursor_write = self.tx_ref().cursor_write::<ZenithHeaders>()?;
        let mut walker = cursor_write.walk_range(target..)?;
        while let Some((k, DbZenithHeader(e))) = walker.next().transpose()? {
            walker.delete_current()?;
            items.push((k, e))
        }

        Ok(items)
    }

    /// Remove [`Zenith::BlockHeader`] objects above the specified height from the DB.
    fn remove_signet_headers_above(
        &self,
        target: BlockNumber,
        _remove_from: StorageLocation,
    ) -> ProviderResult<()> {
        self.remove::<ZenithHeaders>(target..)?;
        Ok(())
    }

    /// Get [`Passage::EnterToken`], [`Passage::Enter`] and
    /// [`Transactor::Transact`] events.
    ///
    /// [`Transactor::Transact`]: signet_zenith::Transactor::Transact
    fn get_signet_events(
        &self,
        range: RangeInclusive<BlockNumber>,
    ) -> ProviderResult<Vec<(BlockNumber, DbSignetEvent)>> {
        let mut cursor = self.tx_ref().cursor_read::<SignetEvents>()?;
        let walker = cursor.walk_range(range)?;
        walker.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Take [`Passage::EnterToken`]s from the DB.
    fn take_signet_events_above(
        &self,
        target: BlockNumber,
        remove_from: StorageLocation,
    ) -> ProviderResult<Vec<(BlockNumber, DbSignetEvent)>> {
        let range = target..=(1 + self.last_block_number()?);
        let items = self.get_signet_events(range)?;
        self.remove_signet_events_above(target, remove_from)?;
        Ok(items)
    }

    /// Remove [`Passage::EnterToken`], [`Passage::Enter`] and
    /// [`Transactor::Transact`] events above the specified height from the DB.
    ///
    /// [`Transactor::Transact`]: signet_zenith::Transactor::Transact
    fn remove_signet_events_above(
        &self,
        target: BlockNumber,
        _remove_from: StorageLocation,
    ) -> ProviderResult<()> {
        self.remove::<SignetEvents>(target..)?;
        Ok(())
    }

    /// Appends the signet-related contents of a host block to the DB:
    /// (RU block, state, enters, enter tokens, transactions)
    /// The contents MUST be appended in the following order:
    /// - The Signet Block (through `RuWriter::insert_signet_block`)
    /// - The state modified by the block (through `RuWriter::ru_write_state`)
    /// - The enters, if any (through `RuWriter::insert_enter`)
    /// - The enter tokens, if any (through `RuWriter::insert_enter_token`)
    /// - The force-included transactions, if any (through `RuWriter::insert_transact`)
    ///
    /// Several DB tables are affected throughout this process. For a detailed breakdown,
    /// see the documentation for each function.
    fn append_host_block(
        &self,
        host_height: u64,
        header: Option<Zenith::BlockHeader>,
        transacts: impl IntoIterator<Item = Transact>,
        enters: impl IntoIterator<Item = Passage::Enter>,
        enter_tokens: impl IntoIterator<Item = Passage::EnterToken>,
        block_result: &BlockResult,
        journal_hash: B256,
    ) -> ProviderResult<()> {
        // Implementation largely copied from
        // `BlockWriter::append_blocks_with_state`
        // in `reth/crates/storage/provider/src/providers/database/provider.rs`
        // duration metrics have been removed
        //
        // last reviewed at tag v1.5.1

        let BlockResult { sealed_block: block, execution_outcome, .. } = block_result;

        let ru_height = block.number();
        self.insert_signet_block(header, block, journal_hash, StorageLocation::Database)?;

        // Write the state and match the storage location that Reth uses.
        self.ru_write_state(execution_outcome, OriginalValuesKnown::No, StorageLocation::Database)?;

        // NB: At this point, reth writes hashed state and trie updates. Signet
        // skips this. We re-use these tables to write the enters, enter tokens,
        // and transact events.

        let mut index: u64 = 0;
        for enter in enters.into_iter() {
            self.insert_enter(ru_height, index, enter)?;
            debug!(ru_height, index, "inserted enter");
            index += 1;
        }

        for enter_token in enter_tokens.into_iter() {
            self.insert_enter_token(ru_height, index, enter_token)?;
            debug!(ru_height, index, "inserted enter token");
            index += 1;
        }

        for transact in transacts.into_iter() {
            self.insert_transact(ru_height, index, &transact)?;
            debug!(ru_height, index, "inserted transact");
            index += 1;
        }

        self.update_history_indices(ru_height..=ru_height)?;

        self.update_pipeline_stages(ru_height, false)?;

        debug!(target: "signet_db_lifecycle", host_height, ru_height, "Appended blocks");

        Ok(())
    }

    #[instrument(skip(self))]
    fn ru_take_blocks_and_execution_above(
        &self,
        target: BlockNumber,
        remove_from: StorageLocation,
    ) -> ProviderResult<RuChain> {
        // Implementation largely copied from
        // `BlockExecutionWriter::take_block_and_execution_above`
        // in `reth/crates/storage/provider/src/providers/database/provider.rs`
        //
        // last reviewed at tag v1.5.1

        let range = target..=self.last_block_number()?;

        // This block is copied from `unwind_trie_state_range`
        //
        // last reviewed at tag v1.5.1
        {
            let changed_accounts = self
                .tx_ref()
                .cursor_read::<tables::AccountChangeSets>()?
                .walk_range(range.clone())?
                .collect::<Result<Vec<_>, _>>()?;
            // There's no need to also unwind account hashes, since that is
            // only useful for filling intermediate tables that deal with state
            // root calculation, which we don't use.
            self.unwind_account_history_indices(changed_accounts.iter())?;

            let storage_range = BlockNumberAddress::range(range.clone());

            // Unwind storage history indices. Similarly, we don't need to
            // unwind storage hashes, since we don't use them.
            let changed_storages = self
                .tx_ref()
                .cursor_read::<tables::StorageChangeSets>()?
                .walk_range(storage_range)?
                .collect::<Result<Vec<_>, _>>()?;

            self.unwind_storage_history_indices(changed_storages.iter().copied())?;

            // We also skip calculating the reverted root here.
        }

        trace!("trie state unwound");

        let execution_state = self.take_state_above(target, remove_from)?;

        trace!("state taken");

        // get blocks
        let blocks = self.recovered_block_range(range.clone())?;

        trace!(count = blocks.len(), "blocks loaded");

        // remove block bodies it is needed for both get block range and get block execution results
        // that is why it is deleted afterwards.
        self.remove_blocks_above(target, remove_from)?;

        trace!("blocks removed");

        // This is a Signet-specific addition that removes the enters,
        // entertokens, zenith headers, and transact events.
        let ru_info =
            self.take_extraction_results_above(target, remove_from)?.into_iter().collect();

        trace!("extraction results taken");

        // Update pipeline stages
        self.update_pipeline_stages(target, true)?;

        let chain = Chain::new(blocks, execution_state, None);

        debug!("Succesfully reverted blocks and updated pipeline stages");

        Ok(RuChain { inner: chain, ru_info })
    }

    #[instrument(skip(self))]
    fn ru_remove_blocks_and_execution_above(
        &self,
        target: BlockNumber,
        remove_from: StorageLocation,
    ) -> ProviderResult<()> {
        // Implementation largely copied from
        // `BlockExecutionWriter::remove_block_and_execution_above`
        // in `reth/crates/storage/provider/src/providers/database/provider.rs`
        // duration metrics have been removed
        //
        // last reviewed at tag v1.5.1

        // This block is copied from `unwind_trie_state_range`
        //
        // last reviewed at tag v1.5.1
        {
            let range = target..=self.last_block_number()?;
            let changed_accounts = self
                .tx_ref()
                .cursor_read::<tables::AccountChangeSets>()?
                .walk_range(range.clone())?
                .collect::<Result<Vec<_>, _>>()?;

            self.unwind_account_history_indices(changed_accounts.iter())?;

            let storage_range = BlockNumberAddress::range(range.clone());

            // Unwind storage history indices.
            let changed_storages = self
                .tx_ref()
                .cursor_read::<tables::StorageChangeSets>()?
                .walk_range(storage_range)?
                .collect::<Result<Vec<_>, _>>()?;

            self.unwind_storage_history_indices(changed_storages.iter().copied())?;
        }

        self.remove_state_above(target, remove_from)?;
        self.remove_blocks_above(target, remove_from)?;

        // Signet specific:
        self.remove_extraction_results_above(target, remove_from)?;

        // Update pipeline stages
        self.update_pipeline_stages(target, true)?;

        Ok(())
    }

    fn ru_write_state(
        &self,
        execution_outcome: &signet_evm::ExecutionOutcome,
        is_value_known: OriginalValuesKnown,
        write_receipts_to: StorageLocation,
    ) -> ProviderResult<()> {
        // Implementation largely copied from
        // `StateWriter::write_state` for `DatabaseProvider`
        // in `reth/crates/storage/provider/src/providers/database/provider.rs`
        //
        // Last reviewed at tag v1.5.1
        let first_block = execution_outcome.first_block();
        let block_count = execution_outcome.len() as u64;
        let last_block = execution_outcome.last_block();
        let block_range = first_block..=last_block;

        let tip = self.last_block_number()?.max(last_block);

        let (plain_state, reverts) =
            execution_outcome.bundle().to_plain_state_and_reverts(is_value_known);

        self.write_state_reverts(reverts, first_block)?;
        self.write_state_changes(plain_state)?;

        // Fetch the first transaction number for each block in the range
        let block_indices: Vec<_> = self
            .block_body_indices_range(block_range)?
            .into_iter()
            .map(|b| b.first_tx_num)
            .collect();

        // Ensure all expected blocks are present.
        if block_indices.len() < block_count as usize {
            let missing_blocks = block_count - block_indices.len() as u64;
            return Err(ProviderError::BlockBodyIndicesNotFound(
                last_block.saturating_sub(missing_blocks - 1),
            ));
        }

        let has_receipts_pruning = self.prune_modes_ref().has_receipts_pruning();

        // Prepare receipts cursor if we are going to write receipts to the database
        //
        // We are writing to database if requested or if there's any kind of receipt pruning
        // configured
        let mut receipts_cursor = (write_receipts_to.database() || has_receipts_pruning)
            .then(|| self.tx_ref().cursor_write::<tables::Receipts<reth::primitives::Receipt>>())
            .transpose()?;

        // Prepare receipts static writer if we are going to write receipts to static files
        //
        // We are writing to static files if requested and if there's no receipt pruning configured
        let should_static = write_receipts_to.static_files() && !has_receipts_pruning;

        // SIGNET: This is a departure from Reth's implementation. Becuase their
        // impl is on `DatabaseProvider`, it has access to the static file
        // provider which is its own prop, and has access to its private field.
        // We are implementing this on `DatabaseProviderRW`, and are not able
        // to borrow from the inner, only to clone it. So we break up the
        // static file provider into a separate variable, and then use it to
        // create the static file writer.
        let sfp = should_static.then(|| self.0.static_file_provider());
        let mut receipts_static_writer = sfp
            .as_ref()
            .map(|sfp| sfp.get_writer(first_block, StaticFileSegment::Receipts))
            .transpose()?;

        let has_contract_log_filter = !self.prune_modes_ref().receipts_log_filter.is_empty();
        let contract_log_pruner =
            self.prune_modes_ref().receipts_log_filter.group_by_block(tip, None)?;

        // All receipts from the last 128 blocks are required for blockchain tree, even with
        // [`PruneSegment::ContractLogs`].
        let prunable_receipts =
            PruneMode::Distance(MINIMUM_PRUNING_DISTANCE).should_prune(first_block, tip);

        // Prepare set of addresses which logs should not be pruned.
        let mut allowed_addresses: HashSet<Address, _> = HashSet::new();
        for (_, addresses) in contract_log_pruner.range(..first_block) {
            allowed_addresses.extend(addresses.iter().copied());
        }

        for (idx, (receipts, first_tx_index)) in
            execution_outcome.receipts().iter().zip(block_indices).enumerate()
        {
            let block_number = first_block + idx as u64;

            // Increment block number for receipts static file writer
            if let Some(writer) = receipts_static_writer.as_mut() {
                writer.increment_block(block_number)?;
            }

            // Skip writing receipts if pruning configuration requires us to.
            if prunable_receipts
                && self
                    .prune_modes_ref()
                    .receipts
                    .is_some_and(|mode| mode.should_prune(block_number, tip))
            {
                continue;
            }

            // If there are new addresses to retain after this block number, track them
            if let Some(new_addresses) = contract_log_pruner.get(&block_number) {
                allowed_addresses.extend(new_addresses.iter().copied());
            }

            for (idx, receipt) in receipts.iter().map(DataCompat::clone_convert).enumerate() {
                let receipt_idx = first_tx_index + idx as u64;
                // Skip writing receipt if log filter is active and it does not have any logs to
                // retain
                if prunable_receipts
                    && has_contract_log_filter
                    && !receipt.logs().iter().any(|log| allowed_addresses.contains(&log.address))
                {
                    continue;
                }

                if let Some(writer) = &mut receipts_static_writer {
                    writer.append_receipt(receipt_idx, &receipt)?;
                }

                if let Some(cursor) = &mut receipts_cursor {
                    cursor.append(receipt_idx, &receipt)?;
                }
            }
        }

        Ok(())
    }
}

// Some code in this file has been copied and modified from reth
// <https://github.com/paradigmxyz/reth>
// The original license is included below:
//
// The MIT License (MIT)
//
// Copyright (c) 2022-2024 Reth Contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//.
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
