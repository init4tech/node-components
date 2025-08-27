use crate::{RuRevmState, SignetCtx};
use alloy::{consensus::Header, eips::BlockId};
use reth::{
    providers::{ProviderResult, providers::BlockchainProvider},
    rpc::server_types::eth::{EthApiError, EthConfig},
    rpc::types::BlockNumberOrTag,
    tasks::{TaskExecutor, TaskSpawner},
};
use reth_node_api::FullNodeComponents;
use signet_evm::EvmNeedsTx;
use signet_node_types::Pnt;
use signet_tx_cache::client::TxCache;
use signet_types::constants::SignetSystemConstants;
use std::sync::Arc;
use tokio::sync::{AcquireError, OwnedSemaphorePermit, Semaphore};
use trevm::{helpers::Ctx, revm::Inspector};

/// State location when instantiating an EVM instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i8)]
pub enum LoadState {
    /// Load the state before the block's transactions (i.e. at the start of
    /// the block).
    Before = -1,
    /// Load the state after the block's transactions (i.e. at the end of the
    /// block).
    After = 0,
}

impl LoadState {
    /// Adjust the height based on the state location.
    pub const fn adjust_height(&self, height: u64) -> u64 {
        match self {
            LoadState::Before => height.saturating_sub(1),
            LoadState::After => height,
        }
    }

    /// Returns `true` if the state location is before the block.
    pub const fn is_before_block(&self) -> bool {
        matches!(self, Self::Before)
    }

    /// Returns `true` if the state location is after the block.
    pub const fn is_after_block(&self) -> bool {
        matches!(self, Self::After)
    }
}

impl From<BlockId> for LoadState {
    fn from(value: BlockId) -> Self {
        match value {
            BlockId::Number(no) => no.into(),
            _ => LoadState::After,
        }
    }
}

impl From<BlockNumberOrTag> for LoadState {
    fn from(value: BlockNumberOrTag) -> Self {
        match value {
            BlockNumberOrTag::Pending => LoadState::Before,
            _ => LoadState::After,
        }
    }
}

impl From<LoadState> for bool {
    fn from(value: LoadState) -> Self {
        matches!(value, LoadState::Before)
    }
}

/// RPC context. Contains all necessary host and signet components for serving
/// RPC requests.
#[derive(Debug)]
pub struct RpcCtx<Host, Signet>
where
    Host: FullNodeComponents,
    Signet: Pnt,
{
    inner: Arc<RpcCtxInner<Host, Signet>>,
}

impl<Host, Signet> RpcCtx<Host, Signet>
where
    Host: FullNodeComponents,
    Signet: Pnt,
{
    /// Create a new `RpcCtx`.
    ///
    /// ## WARNING
    ///
    /// The [`BlockchainProvider`] passed in MUST be receiving updates from the
    /// node wrt canonical chain changes. Some task MUST be calling relevant
    /// [`CanonChainTracker`] methods on a clone of this [`BlockchainProvider`],
    ///
    /// If this is not correctly set up, [`BlockId`] resolution for `latest`,
    /// `safe,` finalized, etc will not work correctly.
    ///
    /// [`CanonChainTracker`]: reth::providers::CanonChainTracker
    pub fn new<Tasks>(
        host: Host,
        constants: SignetSystemConstants,
        provider: BlockchainProvider<Signet>,
        eth_config: EthConfig,
        tx_cache: Option<TxCache>,
        spawner: Tasks,
    ) -> ProviderResult<Self>
    where
        Tasks: TaskSpawner + Clone + 'static,
    {
        RpcCtxInner::new(host, constants, provider, eth_config, tx_cache, spawner)
            .map(|inner| Self { inner: Arc::new(inner) })
    }
}

impl<Host, Signet> Clone for RpcCtx<Host, Signet>
where
    Host: FullNodeComponents,
    Signet: Pnt,
{
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

impl<Host, Signet> core::ops::Deref for RpcCtx<Host, Signet>
where
    Host: FullNodeComponents,
    Signet: Pnt,
{
    type Target = RpcCtxInner<Host, Signet>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// Shared context between all RPC handlers.
#[derive(Debug)]
struct SharedContext {
    tracing_semaphores: Arc<Semaphore>,
}

/// Inner context for [`RpcCtx`].
#[derive(Debug)]
pub struct RpcCtxInner<Host, Signet>
where
    Host: FullNodeComponents,
    Signet: Pnt,
{
    host: Host,
    signet: SignetCtx<Signet>,

    shared: SharedContext,
}

impl<Host, Signet> RpcCtxInner<Host, Signet>
where
    Host: FullNodeComponents,
    Signet: Pnt,
{
    /// Create a new `RpcCtxInner`.
    ///
    /// ## WARNING
    ///
    /// The [`BlockchainProvider`] passed in MUST be receiving updates from the
    /// node wrt canonical chain changes. Some task MUST be calling relevant
    /// [`CanonChainTracker`] methods on a clone of this [`BlockchainProvider`],
    ///
    /// If this is not correctly set up, [`BlockId`] resolution for `latest`,
    /// `safe,` finalized, etc will not work correctly.
    ///
    /// [`CanonChainTracker`]: reth::providers::CanonChainTracker
    pub fn new<Tasks>(
        host: Host,
        constants: SignetSystemConstants,
        provider: BlockchainProvider<Signet>,
        eth_config: EthConfig,
        tx_cache: Option<TxCache>,
        spawner: Tasks,
    ) -> ProviderResult<Self>
    where
        Tasks: TaskSpawner + Clone + 'static,
    {
        SignetCtx::new(constants, provider, eth_config, tx_cache, spawner).map(|signet| Self {
            host,
            signet,
            shared: SharedContext {
                tracing_semaphores: Semaphore::new(eth_config.max_tracing_requests).into(),
            },
        })
    }

    /// Acquire a permit for tracing.
    pub async fn acquire_tracing_permit(&self) -> Result<OwnedSemaphorePermit, AcquireError> {
        self.shared.tracing_semaphores.clone().acquire_owned().await
    }

    pub const fn host(&self) -> &Host {
        &self.host
    }

    pub const fn signet(&self) -> &SignetCtx<Signet> {
        &self.signet
    }

    pub fn task_executor(&self) -> &TaskExecutor {
        self.host.task_executor()
    }

    /// Instantiate a trevm instance with a custom inspector.
    ///
    /// The `header` argument is used to fill the block context of the EVM. If
    /// the `block_id` is `Pending` the EVM state will be the block BEFORE the
    /// `header`. I.e. if the block number of the `header` is `n`, the state
    /// will be after block `n-1`, (effectively the state at the start of block
    /// `n`).
    ///
    /// if the `block_id` is `Pending` the state will be based on the
    /// and `block` arguments
    pub fn trevm_with_inspector<I: Inspector<Ctx<RuRevmState>>>(
        &self,
        state: LoadState,
        header: &Header,
        inspector: I,
    ) -> Result<EvmNeedsTx<RuRevmState, I>, EthApiError> {
        let load_height = state.adjust_height(header.number);
        let spec_id = self.signet.evm_spec_id(header);

        let db = self.signet.state_provider_database(load_height)?;

        let mut trevm =
            signet_evm::signet_evm_with_inspector(db, inspector, self.signet.constants().clone())
                .fill_cfg(&self.signet)
                .fill_block(header);

        trevm.set_spec_id(spec_id);

        Ok(trevm)
    }

    /// Create a trevm instance.
    pub fn trevm(
        &self,
        state: LoadState,
        header: &Header,
    ) -> Result<EvmNeedsTx<RuRevmState>, EthApiError> {
        self.trevm_with_inspector(state, header, trevm::revm::inspector::NoOpInspector)
    }
}

// Some code in this file has been copied and modified from reth
// <https://github.com/paradigmxyz/reth>
// The original license is included below:
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
