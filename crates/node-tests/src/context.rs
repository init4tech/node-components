use crate::{
    HostBlockSpec, NotificationSpec, NotificationWithSidecars, RuBlockSpec,
    convert::ToRethPrimitive,
    types::{CtxProvider, Log, TestCounterInstance, TestErc20Instance, TestLogInstance},
    utils::create_test_provider_factory_with_chain_spec,
};
use alloy::{
    consensus::{BlockHeader, TxEnvelope, constants::ETH_TO_WEI},
    genesis::{Genesis, GenesisAccount},
    network::{Ethereum, EthereumWallet, TransactionBuilder as _},
    primitives::{Address, I256, Sign, U256, keccak256, map::HashSet},
    providers::{
        Provider as _, ProviderBuilder, SendableTx,
        fillers::{BlobGasFiller, SimpleNonceManager},
    },
    rpc::types::eth::{TransactionReceipt, TransactionRequest},
};
use reth::{
    primitives::Account,
    providers::{AccountReader, BlockNumReader, ProviderFactory},
    transaction_pool::{TransactionOrigin, TransactionPool, test_utils::MockTransaction},
};
use reth_db::{PlainAccountState, transaction::DbTxMut};
use reth_exex_test_utils::{Adapter, TestExExHandle, TmpDB as TmpDb};
use reth_node_api::FullNodeComponents;
use signet_db::DbProviderExt;
use signet_node::SignetNode;
use signet_node_config::test_utils::test_config;
use signet_node_types::{NodeStatus, SignetNodeTypes};
use signet_test_utils::contracts::counter::COUNTER_DEPLOY_CODE;
use signet_types::constants::{HostPermitted, RollupPermitted, SignetSystemConstants};
use signet_zenith::{HostOrders::OrdersInstance, RollupPassage::RollupPassageInstance};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use tokio::{sync::watch, task::JoinHandle};
use tracing::instrument;

/// Signet Node test context
///
/// This contains the following:
///
/// - Reth/ExEx context info
///     - The test exex handle, which is used to send notifications to the signet
///       instance.
///     - The components for the Signet Node instance
/// - A receiver for the node status (latest block processed)
/// - A DB provider factory
/// - An alloy provider connected to the Signet Node RPC,
///     - Configured with standard fillers
/// - A height, used to fill in block numbers for host block notifications
/// - A set of addresses for testing, each of which has a pre-existing balance
///   on the Signet Node chain.
pub struct SignetTestContext {
    /// The test exex handle
    pub handle: TestExExHandle,

    /// The components for the Signet Node instance
    pub components: Adapter,

    /// The Signet Node status receiver
    pub node_status: watch::Receiver<NodeStatus>,

    /// The provider factory for the Signet Node instance
    pub factory: ProviderFactory<SignetNodeTypes<TmpDb>>,

    /// An alloy provider connected to the Signet Node RPC.
    pub alloy_provider: CtxProvider,

    /// The system constants for the Signet Node instance.
    pub constants: SignetSystemConstants,

    /// The current host block height
    pub height: AtomicU64,

    /// The alias oracle used by the Signet Node instance.
    pub alias_oracle: Arc<Mutex<HashSet<Address>>>,

    /// Test addresses, copied from [`signet_types::test_utils::TEST_USERS`] for
    /// convenience
    pub addresses: [Address; 10],
}

impl core::fmt::Debug for SignetTestContext {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SignetTestContext").finish()
    }
}

impl SignetTestContext {
    /// Make a new test env
    #[instrument]
    pub async fn new() -> (Self, JoinHandle<eyre::Result<()>>) {
        let cfg = test_config();
        let (ctx, handle) = reth_exex_test_utils::test_exex_context().await.unwrap();
        let components = ctx.components.clone();

        // set up Signet Node db
        let constants = cfg.constants().unwrap();
        let chain_spec: Arc<_> = cfg.chain_spec().clone();

        let factory = create_test_provider_factory_with_chain_spec(chain_spec.clone());

        let alias_oracle: Arc<Mutex<HashSet<Address>>> = Arc::new(Mutex::new(HashSet::default()));

        // instantiate Signet Node, booting rpc
        let (node, mut node_status) = SignetNode::new(
            ctx,
            cfg.clone(),
            factory.clone(),
            Arc::clone(&alias_oracle),
            Default::default(),
        )
        .unwrap();

        // Spawn the node, and wait for it to indicate RPC readiness.
        let node = tokio::spawn(node.start());
        node_status.changed().await.unwrap();

        // set up some keys and addresses
        let keys = &signet_test_utils::users::TEST_SIGNERS;
        let addresses = *signet_test_utils::users::TEST_USERS;

        // register the signers on the alloy proider
        let mut wallet = EthereumWallet::new(keys[0].clone());
        for key in keys.iter().skip(1) {
            wallet.register_signer(key.clone());
        }

        let mint_amnt = U256::from(1_000) * U256::from(ETH_TO_WEI);
        factory
            .provider_rw()
            .unwrap()
            .update(|rw| {
                for address in addresses.into_iter() {
                    let mut account = rw.basic_account(&address)?.unwrap_or_default();
                    account.balance = account.balance.saturating_add(mint_amnt);
                    rw.tx_ref().put::<PlainAccountState>(address, account)?;
                }
                Ok(())
            })
            .unwrap();

        // after RPC booted, we can create the alloy provider
        let alloy_provider = ProviderBuilder::new_with_network()
            .disable_recommended_fillers()
            .filler(BlobGasFiller)
            .with_gas_estimation()
            .with_nonce_management(SimpleNonceManager::default())
            .with_chain_id(constants.ru_chain_id())
            .wallet(wallet)
            .connect(cfg.ipc_endpoint().unwrap())
            .await
            .unwrap();

        let this = Self {
            handle,
            components,
            node_status,
            factory,
            alloy_provider,
            constants,
            height: AtomicU64::new(cfg.constants().unwrap().host_deploy_height()),

            alias_oracle,
            addresses,
        };

        (this, node)
    }

    /// Set whether an address should be aliased. This will be propagated to
    /// the running node.
    pub fn set_should_alias(&self, address: Address, should_alias: bool) {
        let mut guard = self.alias_oracle.lock().expect("failed to lock alias oracle mutex");
        if should_alias {
            guard.insert(address);
        } else {
            guard.remove(&address);
        }
    }

    /// Clone the Signet system constants
    pub fn constants(&self) -> SignetSystemConstants {
        self.constants.clone()
    }

    /// Send a notification to the Signet Node instance
    pub async fn send_notification(&self, notification: NotificationWithSidecars) {
        let pool = self.components.pool();

        // Put these into the pool so that Signet Node can find them
        for (_, (sidecar, tx_signed)) in notification.sidecars {
            let tx_hash = *tx_signed.hash();
            let mut tx = MockTransaction::eip4844_with_sidecar(sidecar.into());
            tx.set_hash(tx_hash);
            pool.add_transaction(TransactionOrigin::Local, tx).await.unwrap();

            assert!(pool.get_blob(tx_hash).unwrap().is_some(), "Missing blob we just inserted");
        }

        self.handle.notifications_tx.send(notification.notification.to_reth()).await.unwrap();
    }

    /// Send a notification to the Signet Node instance and wait for it to be
    /// processed. Does not override block height with the current height.
    async fn process_notification(
        &self,
        notification: NotificationWithSidecars,
    ) -> Result<(), watch::error::RecvError> {
        let mut recv = self.node_status.clone();
        async move {
            let committed_chain = notification.notification.committed_chain();
            let reverted_chain = notification.notification.reverted_chain();

            // the expected height is either the last committed block, or the
            // first reverted block - 1, or 0 if neither exist
            let expected_height = committed_chain
                .and_then(|c| c.blocks.last().map(|b| b.number()))
                .or_else(|| {
                    reverted_chain
                        .and_then(|c| c.blocks.first().map(|b| b.number().saturating_sub(1)))
                })
                .unwrap_or(0)
                .saturating_sub(self.constants().host_deploy_height());

            recv.mark_unchanged();
            self.send_notification(notification).await;
            recv.changed().await?;

            // cheeky little check that the RPC is correct :)
            assert_eq!(self.alloy_provider.get_block_number().await.unwrap(), expected_height);

            Ok(())
        }
        .await
    }

    /// Send a single block to the signet node instance and wait for it to be
    /// processed. Overrides block height with the current height.
    pub async fn process_block(&self, block: HostBlockSpec) -> Result<(), watch::error::RecvError> {
        block.set_block_number(self.height.fetch_add(1, Ordering::SeqCst) + 1);

        let notification = NotificationWithSidecars::commit_single_block(block);

        self.process_notification(notification).await
    }

    /// Send multiple blocks to the signet node instance and wait for them to be
    /// processed. Overrides block heights with the correct height.
    pub async fn process_blocks(
        &self,
        blocks: Vec<HostBlockSpec>,
    ) -> Result<(), watch::error::RecvError> {
        let mut spec = NotificationSpec::default();
        for block in blocks.into_iter() {
            block.set_block_number(self.height.fetch_add(1, Ordering::SeqCst) + 1);
            spec = spec.commit(block);
        }

        self.process_notification(spec.to_exex_notification()).await
    }

    /// Revert a single block and wait for it to be processed. Overrides
    /// block height with the current height.
    pub async fn revert_block(&self, block: HostBlockSpec) -> Result<(), watch::error::RecvError> {
        block.set_block_number(self.height.fetch_sub(1, Ordering::SeqCst));

        let notification = NotificationWithSidecars::revert_single_block(block);

        self.process_notification(notification).await
    }

    /// Get the current host height.
    pub fn height(&self) -> u64 {
        self.height.load(Ordering::SeqCst)
    }

    /// Get the account for an address.
    pub fn account(&self, address: Address) -> Option<Account> {
        self.factory.provider().unwrap().basic_account(&address).unwrap()
    }

    /// Get the nonce off an addresss.
    pub fn nonce(&self, address: Address) -> u64 {
        self.account(address).unwrap_or_default().nonce
    }

    /// Get the balance of an address.
    pub fn balance_of(&self, address: Address) -> U256 {
        self.account(address).unwrap_or_default().balance
    }

    /// Track the balance of an address in a [`BalanceChecks`] guard.
    pub fn track_balance(
        &self,
        address: Address,
        label: Option<&'static str>,
    ) -> BalanceChecks<'_> {
        BalanceChecks::new(self, address, label)
    }

    /// Track the nonce of an address in a [`NonceChecks`] guard.
    pub fn track_nonce(&self, address: Address, label: Option<&'static str>) -> NonceChecks<'_> {
        NonceChecks::new(self, address, label)
    }

    /// Fill an alloy transaction
    pub async fn fill_alloy_tx(&self, tx: &TransactionRequest) -> eyre::Result<TxEnvelope> {
        let SendableTx::Envelope(tx) = self.alloy_provider.fill(tx.clone()).await? else {
            panic!("tx not completely filled")
        };
        Ok(tx)
    }

    /// Deploy the test Counter contract and return its address.
    pub async fn deploy_counter(&self, deployer: Address) -> TestCounterInstance {
        let contract_address = deployer.create(self.nonce(deployer));
        let tx = TransactionRequest::default()
            .from(deployer)
            .gas_limit(21_000_000)
            .with_deploy_code(COUNTER_DEPLOY_CODE);

        let (_tx, _receipt) = self.process_alloy_tx(&tx).await.unwrap();

        TestCounterInstance::new(contract_address, self.alloy_provider.clone())
    }

    /// Deploy the test Log contract and return its address.
    pub async fn deploy_log(&self, deployer: Address) -> TestLogInstance {
        let contract_address = deployer.create(self.nonce(deployer));
        let tx = TransactionRequest::default()
            .from(deployer)
            .gas_limit(21_000_000)
            .with_deploy_code(Log::BYTECODE.clone());

        let (_tx, _receipt) = self.process_alloy_tx(&tx).await.unwrap();

        TestLogInstance::new(contract_address, self.alloy_provider.clone())
    }

    /// Process an alloy transaction
    #[must_use = "Don't forget to make assertions :)"]
    pub async fn process_alloy_tx(
        &self,
        tx: &TransactionRequest,
    ) -> eyre::Result<(TxEnvelope, TransactionReceipt)> {
        let tx = self.fill_alloy_tx(tx).await?;

        let ru_block = RuBlockSpec::new(self.constants.clone()).alloy_tx(&tx);
        let host_block = HostBlockSpec::new(self.constants.clone()).submit_block(ru_block);

        self.process_block(host_block).await?;

        let tx_hash = tx.tx_hash();
        let receipt = self
            .alloy_provider
            .get_transaction_receipt(*tx_hash)
            .await?
            .ok_or_else(|| eyre::eyre!("no receipt"))?;

        tracing::trace!(
            ?tx,
            ?receipt,
            %tx_hash,
            "Processed alloy tx"
        );

        Ok((tx, receipt))
    }

    /// Verify all the allocations in a genesis block. This will only make
    /// assertions if run on the genesis block. If any other blocks have been
    /// processed it will do nothing.
    pub fn verify_allocs(&self, genesis: &Genesis) {
        if self.factory.provider().unwrap().last_block_number().unwrap() != 0 {
            return;
        }

        for (acct, alloc) in genesis.alloc.iter() {
            self.verify_alloc(*acct, alloc);
        }
    }

    #[track_caller]
    fn verify_alloc(&self, address: Address, alloc: &GenesisAccount) {
        let account = self.account(address).unwrap();
        assert_eq!(account.balance, alloc.balance);
        assert_eq!(account.nonce, alloc.nonce.unwrap_or_default());

        if let Some(ref code) = alloc.code {
            assert_eq!(account.bytecode_hash, Some(keccak256(code)));
        } else {
            assert_eq!(account.bytecode_hash, None);
        }

        if let Some(ref storage) = alloc.storage {
            for (key, value) in storage {
                assert_eq!(
                    self.factory.latest().unwrap().storage(address, *key).unwrap(),
                    Some((*value).into())
                );
            }
        }
    }

    /// Get the passage contract instance.
    pub fn ru_passage_contract(&self) -> RollupPassageInstance<CtxProvider, Ethereum> {
        RollupPassageInstance::new(self.constants.rollup().passage(), self.alloy_provider.clone())
    }

    /// Get the orders contract instance.
    pub fn ru_orders_contract(&self) -> OrdersInstance<CtxProvider, Ethereum> {
        OrdersInstance::new(self.constants.rollup().orders(), self.alloy_provider.clone())
    }

    /// Mint tokens for the user
    pub async fn mint_token(
        &self,
        token: HostPermitted,
        user: Address,
        amount: usize,
    ) -> eyre::Result<()> {
        let host_token = self.constants.host().tokens().address_for(token);
        let ru_token = self.constants.rollup().tokens().address_for(token.into());

        let block =
            HostBlockSpec::new(self.constants.clone()).enter_token(user, amount, host_token);
        self.process_block(block).await.unwrap();

        let ru_token = TestErc20Instance::new(ru_token, self.alloy_provider.clone());
        assert_eq!(ru_token.balanceOf(user).call().await.unwrap(), U256::from(amount));
        Ok(())
    }

    /// Get a token contract instance for a given rollup token.
    pub fn token_instance(&self, rollup: RollupPermitted) -> TestErc20Instance {
        let address = self.constants.rollup().tokens().address_for(rollup);
        TestErc20Instance::new(address, self.alloy_provider.clone())
    }
}

/// A guard that checks the nonce of an address
#[derive(Debug, Copy, Clone)]
pub struct NonceChecks<'a> {
    ctx: &'a SignetTestContext,
    address: Address,
    stored: u64,
    label: Option<&'static str>,
}

impl<'a> NonceChecks<'a> {
    /// Create a new nonce checks guard.
    pub fn new(ctx: &'a SignetTestContext, address: Address, label: Option<&'static str>) -> Self {
        let stored = ctx.nonce(address);
        Self { ctx, address, stored, label }
    }

    /// Format an error message with the address or label
    fn fmt_err_str(&self, message: String) -> String {
        let this = if let Some(label) = self.label {
            label.to_owned()
        } else {
            format!("{}", self.address)
        };

        format!(r#"\nAssertion error for "{this}".\n{message}\n"#)
    }

    /// Get the previous nonce of the address.
    pub const fn previous_nonce(&self) -> u64 {
        self.stored
    }

    /// Update the stored nonce, returning the old value.
    pub fn update_nonce(&mut self) -> u64 {
        let current = self.stored;
        self.stored = self.ctx.nonce(self.address);
        current
    }

    /// Assert that the nonce of the address has increased.
    #[track_caller]
    pub fn assert_increase_by(&mut self, amount: u64) {
        let old_nonce = self.update_nonce();
        let expected = old_nonce + amount;
        assert_eq!(
            self.stored,
            expected,
            "{}",
            self.fmt_err_str(format!(
                "expected nonce increase to be {}. Got nonce {}",
                expected, self.stored
            ))
        );
    }

    /// Assert that the nonce of the address has increased by 1.
    #[track_caller]
    pub fn assert_incremented(&mut self) {
        self.assert_increase_by(1);
    }
}

/// A guard that checks the balance of an address
#[derive(Debug, Copy, Clone)]
pub struct BalanceChecks<'a> {
    ctx: &'a SignetTestContext,
    address: Address,
    stored: U256,
    label: Option<&'static str>,
}

impl<'a> BalanceChecks<'a> {
    /// Instantiate a new balance checks guard.
    pub fn new(ctx: &'a SignetTestContext, address: Address, label: Option<&'static str>) -> Self {
        let stored = ctx.balance_of(address);
        Self { ctx, address, stored, label }
    }

    fn fmt_err_str(&self, message: String) -> String {
        let this = if let Some(label) = self.label {
            label.to_owned()
        } else {
            format!("{}", self.address)
        };

        format!(r#"\nAssertion error for "{this}".\n{message}\n"#)
    }

    /// Get the previous balance of the address.
    pub const fn previous_balance(&self) -> U256 {
        self.stored
    }

    /// Update the stored balance, returning the old value.
    pub fn update_balance(&mut self) -> U256 {
        let current = self.stored;
        self.stored = self.ctx.balance_of(self.address);
        current
    }

    /// Assert that the balance of the address has changed by a certain amount.
    /// Update the previous balance to the new balance.
    #[track_caller]
    pub fn assert_change(&mut self, change: I256) {
        let old_balance = self.update_balance();

        let (sign, abs) = change.into_sign_and_abs();

        let (actual_sign, actual) = if self.stored > old_balance {
            (Sign::Positive, self.stored - old_balance)
        } else {
            (Sign::Negative, old_balance - self.stored)
        };

        assert_eq!(
            (actual_sign, actual),
            (sign, abs),
            "{}",
            self.fmt_err_str(format!(
                "expected balance change of {}{}. Got balance change of {}{}",
                sign,
                abs,
                actual_sign,
                self.stored.abs_diff(old_balance)
            ))
        );
    }

    /// Assert that the balance has decreased.
    #[track_caller]
    pub fn assert_decrease(&mut self) {
        let old_balance = self.update_balance();
        assert!(
            self.stored < old_balance,
            "{}",
            self.fmt_err_str(format!(
                "expected balance decrease. Got balance increase of {}",
                self.stored.abs_diff(old_balance)
            ))
        );
    }

    /// Assert that the balance has decreased by at least a certain amount.
    #[track_caller]
    pub fn assert_decrease_at_least(&mut self, amount: U256) {
        let old_balance = self.update_balance();
        assert!(
            self.stored <= old_balance - amount,
            "{}",
            self.fmt_err_str(format!(
                "expected balance decrease of at least {}. Got balance increase of {}",
                amount,
                self.stored.abs_diff(old_balance)
            ))
        );
    }

    /// Assert that the balance has decreased by exactly a certain amount.
    #[track_caller]
    pub fn assert_decrease_exact(&mut self, amount: U256) {
        let change = I256::checked_from_sign_and_abs(Sign::Negative, amount).unwrap();
        self.assert_change(change)
    }

    /// Assert that the balance has increased.
    #[track_caller]
    pub fn assert_increase(&mut self) {
        let old_balance = self.update_balance();
        assert!(
            self.stored > old_balance,
            "{}",
            self.fmt_err_str(format!(
                "expected balance increase. Got balance decrease of {}",
                self.stored.abs_diff(old_balance)
            ))
        );
    }

    /// Assert that the balance has increased by at least a certain amount.
    #[track_caller]
    pub fn assert_increase_at_least(&mut self, amount: U256) {
        let old_balance = self.update_balance();
        assert!(
            self.stored >= old_balance + amount,
            "{}",
            self.fmt_err_str(format!(
                "expected balance increase of at least {}. Got balance decrease of {}",
                amount,
                self.stored.abs_diff(old_balance)
            ))
        );
    }

    /// Assert that the balance has increased by exactly a certain amount.
    #[track_caller]
    pub fn assert_increase_exact(&mut self, amount: U256) {
        let change = I256::checked_from_sign_and_abs(Sign::Positive, amount).unwrap();
        self.assert_change(change)
    }

    /// Assert that the balance has not changed.
    #[track_caller]
    pub fn assert_no_change(&mut self) {
        let old_balance = self.update_balance();
        assert_eq!(
            self.stored,
            old_balance,
            "{}",
            self.fmt_err_str("expected no balance change".to_owned())
        );
    }

    /// Assert that the balance is equal to a certain value.
    #[track_caller]
    pub fn assert_eq(&mut self, expected: U256) {
        let _old_balance = self.update_balance();
        assert_eq!(
            self.stored,
            expected,
            "{}",
            self.fmt_err_str(format!("expected balance of {expected}"))
        )
    }
}
