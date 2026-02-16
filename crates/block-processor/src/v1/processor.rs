use crate::{AliasOracle, AliasOracleFactory, metrics};
use alloy::{
    consensus::BlockHeader,
    primitives::{Address, Sealable, map::HashSet},
};
use core::fmt;
use eyre::ContextCompat;
use init4_bin_base::utils::calc::SlotCalculator;
use reth::{providers::StateProviderFactory, revm::db::StateBuilder};
use reth_chainspec::ChainSpec;
use signet_blobber::{CacheHandle, ExtractableChainShim};
use signet_constants::SignetSystemConstants;
use signet_evm::{BlockResult, EvmNeedsCfg, SignetDriver};
use signet_extract::Extracts;
use signet_hot::{
    db::HotDbRead,
    model::{HotKv, HotKvRead, RevmRead},
};
use signet_storage_types::{DbSignetEvent, DbZenithHeader, ExecutedBlock, ExecutedBlockBuilder};
use std::{collections::VecDeque, sync::Arc};
use tracing::{error, instrument};
use trevm::revm::{
    database::{DBErrorMarker, State},
    primitives::hardfork::SpecId,
};

/// The revm state type backed by hot storage.
type HotRevmState<H> = State<RevmRead<<H as HotKv>::RoTx>>;

/// A block processor that extracts and processes Signet blocks from host
/// chain commits.
///
/// The processor is a stateless executor: it reads state from hot storage,
/// runs the EVM, and returns an [`ExecutedBlock`]. The caller (node) handles
/// extraction, persistence, and orchestrates the per-block loop.
pub struct SignetBlockProcessor<H, Alias = Box<dyn StateProviderFactory>>
where
    H: HotKv,
{
    /// Signet System Constants.
    constants: SignetSystemConstants,

    /// The chain specification, used to determine active hardforks.
    chain_spec: Arc<ChainSpec>,

    /// Hot storage handle for rollup state reads.
    hot: H,

    /// An oracle for determining whether addresses should be aliased.
    /// Reads HOST (L1) state, not rollup state.
    alias_oracle: Alias,

    /// The slot calculator.
    slot_calculator: SlotCalculator,

    /// A handle to the blob cacher.
    blob_cacher: CacheHandle,
}

impl<H> fmt::Debug for SignetBlockProcessor<H>
where
    H: HotKv,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignetBlockProcessor").finish()
    }
}

impl<H, Alias> SignetBlockProcessor<H, Alias>
where
    H: HotKv,
    H::RoTx: 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
    Alias: AliasOracleFactory,
{
    /// Create a new [`SignetBlockProcessor`].
    pub const fn new(
        constants: SignetSystemConstants,
        chain_spec: Arc<ChainSpec>,
        hot: H,
        alias_oracle: Alias,
        slot_calculator: SlotCalculator,
        blob_cacher: CacheHandle,
    ) -> Self {
        Self { constants, chain_spec, hot, alias_oracle, slot_calculator, blob_cacher }
    }

    /// Get the active spec id at the given timestamp.
    fn spec_id(&self, timestamp: u64) -> SpecId {
        crate::revm_spec(&self.chain_spec, timestamp)
    }

    /// Build a revm [`State`] backed by hot storage at the given parent
    /// height.
    fn revm_state(&self, parent_height: u64) -> eyre::Result<HotRevmState<H>> {
        let reader = self.hot.reader()?;
        let db = RevmRead::at_height(reader, parent_height);
        Ok(StateBuilder::new_with_database(db).with_bundle_update().build())
    }

    /// Make a new Trevm instance, building on the given height.
    fn trevm(
        &self,
        parent_height: u64,
        spec_id: SpecId,
    ) -> eyre::Result<EvmNeedsCfg<HotRevmState<H>>> {
        let db = self.revm_state(parent_height)?;
        let mut trevm = signet_evm::signet_evm(db, self.constants.clone());
        trevm.set_spec_id(spec_id);
        Ok(trevm)
    }

    /// Check if the given address should be aliased.
    fn should_alias(&self, address: Address) -> eyre::Result<bool> {
        self.alias_oracle.create()?.should_alias(address)
    }

    /// Process a single extracted block, returning an [`ExecutedBlock`].
    ///
    /// The caller is responsible for driving extraction (via [`Extractor`])
    /// and persisting the result to storage between calls.
    ///
    /// [`Extractor`]: signet_extract::Extractor
    #[instrument(skip_all, fields(
        ru_height = block_extracts.ru_height,
        host_height = block_extracts.host_block.number(),
        has_ru_block = block_extracts.submitted.is_some(),
    ))]
    pub async fn process_block(
        &self,
        block_extracts: &Extracts<'_, ExtractableChainShim<'_>>,
    ) -> eyre::Result<ExecutedBlock> {
        let start_time = std::time::Instant::now();
        let spec_id = self.spec_id(block_extracts.host_block.timestamp());

        metrics::record_extracts(block_extracts);

        let block_result = self.run_evm(block_extracts, spec_id).await?;
        metrics::record_block_result(&block_result, &start_time);

        self.build_executed_block(block_extracts, block_result)
    }

    /// ==========================
    /// ==========================
    /// ██████  ██    ██ ███    ██
    /// ██   ██ ██    ██ ████   ██
    /// ██████  ██    ██ ██ ██  ██
    /// ██   ██ ██    ██ ██  ██ ██
    /// ██   ██  ██████  ██   ████
    ///
    ///
    /// ███████ ██    ██ ███    ███
    /// ██      ██    ██ ████  ████
    /// █████   ██    ██ ██ ████ ██
    /// ██       ██  ██  ██  ██  ██
    /// ███████   ████   ██      ██
    /// ===========================
    /// ===========================
    ///
    /// Run the EVM for a single block extraction.
    #[instrument(skip_all)]
    async fn run_evm(
        &self,
        block_extracts: &Extracts<'_, ExtractableChainShim<'_>>,
        spec_id: SpecId,
    ) -> eyre::Result<BlockResult> {
        let ru_height = block_extracts.ru_height;
        let host_height = block_extracts.host_block.number();
        let timestamp = block_extracts.host_block.timestamp();

        let parent_header = self
            .hot
            .reader()?
            .get_header(ru_height.saturating_sub(1))?
            .wrap_err("parent ru block not present in DB")
            .inspect_err(|e| error!(%e))?;
        let parent_header = signet_types::primitives::SealedHeader::new(parent_header.into_inner());

        let txns = match &block_extracts.submitted {
            Some(submitted) => {
                // NB: Pre-merge blocks do not have predictable slot times.
                let slot = self
                    .slot_calculator
                    .slot_ending_at(timestamp)
                    .expect("expect submitted events only occur post-merge");
                self.blob_cacher
                    .signet_block(block_extracts.host_block.number(), slot, submitted)
                    .await?
                    .into_parts()
                    .1
                    .into_iter()
                    .filter(|tx| !tx.is_eip4844()) // redundant, but let's be sure
                    .map(|tx| tx.into())
                    .collect::<VecDeque<_>>()
            }
            None => VecDeque::new(),
        };

        // Determine which addresses need to be aliased.
        let mut to_alias: HashSet<Address> = Default::default();
        for transact in block_extracts.transacts() {
            let addr = transact.host_sender();
            if !to_alias.contains(&addr) && self.should_alias(addr)? {
                to_alias.insert(addr);
            }
        }

        let mut driver = SignetDriver::new(
            block_extracts,
            to_alias,
            txns,
            parent_header,
            self.constants.clone(),
        );

        let trevm = self.trevm(driver.parent().number(), spec_id)?.fill_cfg(&driver);

        let trevm = match trevm.drive_block(&mut driver) {
            Ok(t) => t,
            Err(e) => return Err(e.into_error().into()),
        };

        let (sealed_block, receipts) = driver.finish();
        let bundle = trevm.finish();

        Ok(BlockResult {
            sealed_block,
            execution_outcome: signet_evm::ExecutionOutcome::new(bundle, vec![receipts], ru_height),
            host_height,
        })
    }

    /// Build an [`ExecutedBlock`] from processor outputs.
    #[instrument(skip_all)]
    fn build_executed_block(
        &self,
        extracts: &Extracts<'_, ExtractableChainShim<'_>>,
        block_result: BlockResult,
    ) -> eyre::Result<ExecutedBlock> {
        let BlockResult { sealed_block, execution_outcome, .. } = block_result;

        // Header from the sealed block. Re-use the known hash to avoid
        // recomputing it.
        let hash = sealed_block.block.header.hash();
        let header = sealed_block.block.header.header().clone().seal_unchecked(hash);

        // Bundle and receipts from execution outcome.
        let (bundle, receipt_vecs, _) = execution_outcome.into_parts();

        // Flatten receipts (one block → one inner vec) and convert to
        // storage Receipt type.
        let receipts = receipt_vecs
            .into_iter()
            .flatten()
            .map(|envelope| {
                let tx_type = envelope.tx_type();
                signet_storage_types::Receipt { tx_type, inner: envelope.into_receipt() }
            })
            .collect();

        // Transactions: zip txs + senders → Vec<RecoveredTx>.
        let transactions = sealed_block
            .block
            .body
            .transactions
            .into_iter()
            .zip(sealed_block.senders)
            .map(|(tx, sender)| signet_storage_types::Recovered::new_unchecked(tx, sender))
            .collect();

        // Signet events with a single incrementing index across all types.
        let signet_events: Vec<_> = extracts
            .enters()
            .map(|e| DbSignetEvent::Enter(0, e))
            .chain(extracts.enter_tokens().map(|e| DbSignetEvent::EnterToken(0, e)))
            .chain(extracts.transacts().map(|t| DbSignetEvent::Transact(0, t.clone())))
            .enumerate()
            .map(|(i, e)| match e {
                DbSignetEvent::Enter(_, v) => DbSignetEvent::Enter(i as u64, v),
                DbSignetEvent::EnterToken(_, v) => DbSignetEvent::EnterToken(i as u64, v),
                DbSignetEvent::Transact(_, v) => DbSignetEvent::Transact(i as u64, v),
            })
            .collect();

        // Zenith header from extracts.
        let zenith_header = extracts.ru_header().map(DbZenithHeader::from);

        ExecutedBlockBuilder::new()
            .header(header)
            .bundle(bundle)
            .transactions(transactions)
            .receipts(receipts)
            .signet_events(signet_events)
            .zenith_header(zenith_header)
            .build()
            .map_err(|e| eyre::eyre!("failed to build ExecutedBlock: {e}"))
    }
}
