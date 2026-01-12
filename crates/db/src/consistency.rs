use alloy::primitives::BlockNumber;
use reth::{
    api::NodePrimitives,
    primitives::EthPrimitives,
    providers::{
        BlockBodyIndicesProvider, ProviderFactory, ProviderResult, StageCheckpointReader,
        StaticFileProviderFactory, StaticFileSegment, StaticFileWriter,
    },
};
use reth_db::{cursor::DbCursorRO, table::Table, tables, transaction::DbTx};
use reth_stages_types::StageId;
use signet_node_types::{NodeTypesDbTrait, SignetNodeTypes};
use tracing::{debug, info, info_span, instrument, warn};

/// Extension trait that provides consistency checking for the RU database
/// provider. Consistency checks are MANDATORY on node startup to ensure that
/// the static file segments and database are in sync.
///
/// in general, this should not be implemented outside this crate.
pub trait ProviderConsistencyExt {
    /// Check the consistency of the static file segments and return the last
    /// known-good block number.
    fn ru_check_consistency(&self) -> ProviderResult<Option<BlockNumber>>;
}

impl<Db> ProviderConsistencyExt for ProviderFactory<SignetNodeTypes<Db>>
where
    Db: NodeTypesDbTrait,
{
    /// Check the consistency of the static file segments and return the last
    /// known good block number.
    #[instrument(skip(self), fields(read_only = self.static_file_provider().is_read_only()))]
    fn ru_check_consistency(&self) -> ProviderResult<Option<BlockNumber>> {
        // Based on `StaticFileProvider::check_consistency` in
        // `reth/crates/storage/provider/src/providers/static_file/manager.rs`
        // with modifications for RU-specific logic.
        //
        // Comments are largely reproduced from the original source for context.
        //
        // Last updated @ reth@1.9.1
        let prune_modes = self.provider_rw()?.prune_modes_ref().clone();
        let sfp = self.static_file_provider();

        debug!("Checking static file consistency.");

        let last_good_height: Option<BlockNumber> = None;

        let update_last_good_height = |new_height: BlockNumber| {
            last_good_height.map(|current| current.max(new_height)).or(Some(new_height))
        };

        for segment in StaticFileSegment::iter() {
            let initial_highest_block = sfp.get_highest_static_file_block(segment);

            if prune_modes.has_receipts_pruning() && segment.is_receipts() {
                // Pruned nodes (including full node) do not store receipts as static files.
                continue;
            }

            let span = info_span!(
                "checking_segment",
                ?segment,
                initial_highest_block,
                highest_block = tracing::field::Empty,
                highest_tx = tracing::field::Empty
            );
            let _guard = span.enter();

            //  File consistency is broken if:
            //
            // * appending data was interrupted before a config commit, then
            //   data file will be truncated according to the config.
            //
            // * pruning data was interrupted before a config commit, then we
            //   have deleted data that we are expected to still have. We need
            //   to check the Database and unwind everything accordingly.
            if sfp.is_read_only() {
                sfp.check_segment_consistency(segment)?;
            } else {
                // Fetching the writer will attempt to heal any file level
                // inconsistency.
                sfp.latest_writer(segment)?;
            }

            // Only applies to block-based static files. (Headers)
            //
            // The updated `highest_block` may have decreased if we healed from a pruning
            // interruption.
            let mut highest_block = sfp.get_highest_static_file_block(segment);
            span.record("highest_block", highest_block);

            if initial_highest_block != highest_block {
                update_last_good_height(highest_block.unwrap_or_default());
            }

            // Only applies to transaction-based static files. (Receipts & Transactions)
            //
            // Make sure the last transaction matches the last block from its indices, since a heal
            // from a pruning interruption might have decreased the number of transactions without
            // being able to update the last block of the static file segment.
            let highest_tx = sfp.get_highest_static_file_tx(segment);
            if let Some(highest_tx) = highest_tx {
                span.record("highest_tx", highest_tx);
                let mut last_block = highest_block.unwrap_or_default();
                loop {
                    if let Some(indices) = self.block_body_indices(last_block)? {
                        if indices.last_tx_num() <= highest_tx {
                            break;
                        }
                    } else {
                        // If the block body indices can not be found, then it means that static
                        // files is ahead of database, and the `ensure_invariants` check will fix
                        // it by comparing with stage checkpoints.
                        break;
                    }
                    if last_block == 0 {
                        break;
                    }
                    last_block -= 1;

                    highest_block = Some(last_block);
                    update_last_good_height(last_block);
                }
            }

            if let Some(unwind) = match segment {
                StaticFileSegment::Headers => {
                    ensure_invariants::<
                        _,
                        tables::Headers<<EthPrimitives as NodePrimitives>::BlockHeader>,
                    >(self, segment, highest_block, highest_block)?
                }
                StaticFileSegment::Transactions => {
                    ensure_invariants::<
                        _,
                        tables::Transactions<<EthPrimitives as NodePrimitives>::SignedTx>,
                    >(self, segment, highest_tx, highest_block)?
                }
                StaticFileSegment::Receipts => {
                    ensure_invariants::<
                        _,
                        tables::Receipts<<EthPrimitives as NodePrimitives>::Receipt>,
                    >(self, segment, highest_tx, highest_block)?
                }
            } {
                update_last_good_height(unwind);
            }
        }

        Ok(last_good_height)
    }
}

/// Check invariants for each corresponding table and static file segment:
///
/// 1. The corresponding database table should overlap or have continuity in
///    their keys ([`TxNumber`] or [`BlockNumber`]).
/// 2. Its highest block should match the stage checkpoint block number if it's
///    equal or higher than the corresponding database table last entry.
///    * If the checkpoint block is higher, then request a pipeline unwind to
///      the static file block. This is expressed by returning [`Some`] with the
///      requested pipeline unwind target.
///    * If the checkpoint block is lower, then heal by removing rows from the
///      static file. In this case, the rows will be removed and [`None`] will
///      be returned.
/// 3. If the database tables overlap with static files and have contiguous
///    keys, or the checkpoint block matches the highest static files block,
///    then [`None`] will be returned.
///
/// [`TxNumber`]: alloy::primitives::TxNumber
#[instrument(skip(this), fields(table = T::NAME))]
fn ensure_invariants<Db, T: Table<Key = u64>>(
    this: &ProviderFactory<SignetNodeTypes<Db>>,
    segment: StaticFileSegment,
    highest_static_file_entry: Option<u64>,
    highest_static_file_block: Option<BlockNumber>,
) -> ProviderResult<Option<BlockNumber>>
where
    Db: NodeTypesDbTrait,
{
    let provider = this.provider_rw()?;
    let sfp = this.static_file_provider();

    let mut db_cursor = provider.tx_ref().cursor_read::<T>()?;

    if let Some((db_first_entry, _)) = db_cursor.first()? {
        if let (Some(highest_entry), Some(highest_block)) =
            (highest_static_file_entry, highest_static_file_block)
        {
            // If there is a gap between the entry found in static file and
            // database, then we have most likely lost static file data and
            // need to unwind so we can load it again
            if !(db_first_entry <= highest_entry || highest_entry + 1 == db_first_entry) {
                info!(unwind_target = highest_block, "Setting unwind target.");
                return Ok(Some(highest_block));
            }
        }

        if let Some((db_last_entry, _)) = db_cursor.last()?
            && highest_static_file_entry.is_none_or(|highest_entry| db_last_entry > highest_entry)
        {
            return Ok(None);
        }
    }

    let highest_static_file_entry = highest_static_file_entry.unwrap_or_default();
    let highest_static_file_block = highest_static_file_block.unwrap_or_default();

    // If static file entry is ahead of the database entries, then ensure the
    // checkpoint block number matches.
    let checkpoint_block_number = provider
        .get_stage_checkpoint(match segment {
            StaticFileSegment::Headers => StageId::Headers,
            StaticFileSegment::Transactions => StageId::Bodies,
            StaticFileSegment::Receipts => StageId::Execution,
        })?
        .unwrap_or_default()
        .block_number;

    // If the checkpoint is ahead, then we lost static file data. May be data corruption.
    if checkpoint_block_number > highest_static_file_block {
        info!(
            checkpoint_block_number,
            unwind_target = highest_static_file_block,
            "Setting unwind target."
        );
        return Ok(Some(highest_static_file_block));
    }

    // If the checkpoint is behind, then we failed to do a database commit
    // **but committed** to static files on executing a stage, or the reverse
    // on unwinding a stage.
    //
    // All we need to do is to prune the extra static file rows.
    if checkpoint_block_number < highest_static_file_block {
        info!(
            from = highest_static_file_block,
            to = checkpoint_block_number,
            "Unwinding static file segment."
        );

        let mut writer = sfp.latest_writer(segment)?;
        if segment.is_headers() {
            // TODO(joshie): is_block_meta
            writer.prune_headers(highest_static_file_block - checkpoint_block_number)?;
        } else if let Some(block) = provider.block_body_indices(checkpoint_block_number)? {
            // todo joshie: is querying block_body_indices a potential issue
            // once bbi is moved to sf as well
            let number = highest_static_file_entry - block.last_tx_num();
            if segment.is_receipts() {
                writer.prune_receipts(number, checkpoint_block_number)?;
            } else {
                writer.prune_transactions(number, checkpoint_block_number)?;
            }
        }
        writer.commit()?;
    }

    Ok(None)
}

// Some code in this file is adapted from reth. It is used under the terms of
// the MIT License.
//
// The MIT License (MIT)
//
// Copyright (c) 2022-2025 Reth Contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
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
