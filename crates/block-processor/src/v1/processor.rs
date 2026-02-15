use crate::{AliasOracle, AliasOracleFactory, Chain, metrics};
use alloy::{
    consensus::BlockHeader,
    primitives::{Address, map::HashSet},
};
use core::fmt;
use eyre::ContextCompat;
use init4_bin_base::utils::calc::SlotCalculator;
use reth::{
    primitives::EthPrimitives,
    providers::{
        BlockNumReader, BlockReader, ExecutionOutcome, HeaderProvider, ProviderFactory,
        StateProviderFactory,
    },
    revm::{database::StateProviderDatabase, db::StateBuilder},
};
use reth_chainspec::ChainSpec;
use reth_node_api::{FullNodeComponents, NodeTypes};
use signet_blobber::{CacheHandle, ExtractableChainShim};
use signet_constants::SignetSystemConstants;
use signet_db::{DataCompat, DbProviderExt, RuChain, RuRevmState, RuWriter};
use signet_evm::{BlockResult, EvmNeedsCfg, SignetDriver};
use signet_extract::{Extractor, Extracts};
use signet_node_types::{NodeTypesDbTrait, SignetNodeTypes};
use std::{collections::VecDeque, sync::Arc};
use tracing::{Instrument, error, info, info_span, instrument};
use trevm::revm::primitives::hardfork::SpecId;

/// A block processor that listens to host chain commits and processes
/// Signet blocks accordingly.
pub struct SignetBlockProcessor<Db, Alias = Box<dyn StateProviderFactory>>
where
    Db: NodeTypesDbTrait,
{
    /// Signet System Constants
    constants: SignetSystemConstants,

    /// The chain specification, used to determine active hardforks.
    chain_spec: Arc<ChainSpec>,

    /// A [`ProviderFactory`] instance to allow RU database access.
    ru_provider: ProviderFactory<SignetNodeTypes<Db>>,

    /// A [`ProviderFactory`] instance to allow Host database access.
    alias_oracle: Alias,

    /// The slot calculator.
    slot_calculator: SlotCalculator,

    /// A handle to the blob cacher.
    blob_cacher: CacheHandle,
}

impl<Db> fmt::Debug for SignetBlockProcessor<Db>
where
    Db: NodeTypesDbTrait,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignetBlockProcessor").finish()
    }
}

impl<Db, Alias> SignetBlockProcessor<Db, Alias>
where
    Db: NodeTypesDbTrait,
    Alias: AliasOracleFactory,
{
    /// Create a new [`SignetBlockProcessor`].
    pub const fn new(
        constants: SignetSystemConstants,
        chain_spec: Arc<ChainSpec>,
        ru_provider: ProviderFactory<SignetNodeTypes<Db>>,
        alias_oracle: Alias,
        slot_calculator: SlotCalculator,
        blob_cacher: CacheHandle,
    ) -> Self {
        Self { constants, chain_spec, ru_provider, alias_oracle, slot_calculator, blob_cacher }
    }

    /// Get the active spec id at the given timestamp.
    fn spec_id(&self, timestamp: u64) -> SpecId {
        crate::revm_spec(&self.chain_spec, timestamp)
    }

    /// Make a [`StateProviderDatabase`] from the read-write provider, suitable
    /// for use with Trevm.
    fn state_provider_database(&self, height: u64) -> eyre::Result<RuRevmState> {
        // Get the state provider for the block number
        let sp = self.ru_provider.history_by_block_number(height)?;

        // Wrap in Revm comatibility layer
        let spd = StateProviderDatabase::new(sp);
        let builder = StateBuilder::new_with_database(spd);

        Ok(builder.with_bundle_update().build())
    }

    /// Make a new Trevm instance, building on the given height.
    fn trevm(&self, parent_height: u64, spec_id: SpecId) -> eyre::Result<EvmNeedsCfg<RuRevmState>> {
        let db = self.state_provider_database(parent_height)?;

        let mut trevm = signet_evm::signet_evm(db, self.constants.clone());

        trevm.set_spec_id(spec_id);

        Ok(trevm)
    }

    /// Check if the given address should be aliased.
    fn should_alias(&self, address: Address) -> eyre::Result<bool> {
        self.alias_oracle.create()?.should_alias(address)
    }

    /// Called when the host chain has committed a block or set of blocks.
    #[instrument(skip_all, fields(count = chain.len(), first = chain.first().number(), tip = chain.tip().number()))]
    pub async fn on_host_commit<Host>(&self, chain: &Chain<Host>) -> eyre::Result<Option<RuChain>>
    where
        Host: FullNodeComponents,
        Host::Types: NodeTypes<Primitives = EthPrimitives>,
    {
        let highest = chain.tip().number();
        if highest < self.constants.host_deploy_height() {
            return Ok(None);
        }

        // this should never happen but we want to handle it anyway
        if chain.is_empty() {
            return Ok(None);
        }

        let start_time = std::time::Instant::now();

        let extractor = Extractor::new(self.constants.clone());
        let shim = ExtractableChainShim::new(chain);
        let outputs = extractor.extract_signet(&shim);

        metrics::record_extraction_time(&start_time);

        // TODO: ENG-481 Inherit prune modes from Reth configuration.
        // https://linear.app/initiates/issue/ENG-481/inherit-prune-modes-from-reth-node

        // The extractor will filter out blocks at or before the deployment
        // height, so we don't need compute the start from the notification.
        let mut start = None;
        let mut current = 0;
        let last_ru_height = self.ru_provider.last_block_number()?;

        let mut net_outcome = ExecutionOutcome::default();

        // There might be a case where we can get a notification that starts
        // "lower" than our last processed block,
        // but contains new information beyond one point. In this case, we
        // should simply skip the block.
        for block_extracts in outputs.skip_while(|extract| extract.ru_height <= last_ru_height) {
            // If we haven't set the start yet, set it to the first block.
            if start.is_none() {
                let new_ru_height = block_extracts.ru_height;

                // If the above condition passes, we should always be
                // committing without skipping a range of blocks.
                if new_ru_height != last_ru_height + 1 {
                    error!(
                        %new_ru_height,
                        %last_ru_height,
                        "missing range of DB blocks"
                    );
                    eyre::bail!("missing range of DB blocks");
                }
                start = Some(new_ru_height);
            }

            metrics::record_extracts(&block_extracts);

            let start_time = std::time::Instant::now();
            current = block_extracts.ru_height;
            let spec_id = self.spec_id(block_extracts.host_block.timestamp());

            let span = info_span!(
                "signet::handle_zenith_outputs::block_processing",
                start = start.unwrap(),
                ru_height = block_extracts.ru_height,
                host_height = block_extracts.host_block.number(),
                has_ru_block = block_extracts.submitted.is_some(),
                height_before_notification = last_ru_height,
            );

            let block_result =
                self.run_evm(&block_extracts, spec_id).instrument(span.clone()).await?;
            metrics::record_block_result(&block_result, &start_time);

            let _ = span.enter();
            self.commit_evm_results(&block_extracts, &block_result)?;

            net_outcome.extend(block_result.execution_outcome.convert());
        }
        info!("committed blocks");

        // If we didn't process any blocks, we don't need to return anything.
        // In practice, this should never happen, as we should always have at
        // least one block to process.
        if start.is_none() {
            return Ok(None);
        }
        let start = start.expect("checked by early return");

        // Return the range of blocks we processed
        let provider = self.ru_provider.provider_rw()?;

        let ru_info = provider.get_extraction_results(start..=current)?;

        let inner = Chain::<Host>::new(
            provider.recovered_block_range(start..=current)?,
            net_outcome,
            Default::default(),
        );

        Ok(Some(RuChain { inner, ru_info }))
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
    async fn run_evm(
        &self,
        block_extracts: &Extracts<'_, ExtractableChainShim<'_>>,
        spec_id: SpecId,
    ) -> eyre::Result<BlockResult> {
        let ru_height = block_extracts.ru_height;
        let host_height = block_extracts.host_block.number();
        let timestamp = block_extracts.host_block.timestamp();

        let parent_header = self
            .ru_provider
            .sealed_header(block_extracts.ru_height.saturating_sub(1))?
            .wrap_err("parent ru block not present in DB")
            .inspect_err(|e| error!(%e))?;

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
            parent_header.convert(),
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

    /// Commit the outputs of a zenith block to the database.
    #[instrument(skip_all)]
    fn commit_evm_results(
        &self,
        extracts: &Extracts<'_, ExtractableChainShim<'_>>,
        block_result: &BlockResult,
    ) -> eyre::Result<()> {
        self.ru_provider.provider_rw()?.update(|writer| {
            writer.append_host_block(
                extracts.ru_header(),
                extracts.transacts().cloned(),
                extracts.enters(),
                extracts.enter_tokens(),
                block_result,
            )?;
            Ok(())
        })?;
        Ok(())
    }
}
