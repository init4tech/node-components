use crate::{RuRevmState, SignetCtx};
use alloy::{
    consensus::{BlockHeader, Header},
    eips::BlockId,
};
use reth::{
    providers::{ProviderFactory, ProviderResult},
    rpc::server_types::eth::{EthApiError, EthConfig},
    tasks::{TaskExecutor, TaskSpawner},
};
use reth_node_api::FullNodeComponents;
use signet_evm::EvmNeedsTx;
use signet_node_types::Pnt;
use signet_tx_cache::client::TxCache;
use signet_types::constants::SignetSystemConstants;
use std::sync::Arc;

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
    pub fn new<Tasks>(
        host: Host,
        constants: SignetSystemConstants,
        factory: ProviderFactory<Signet>,
        eth_config: EthConfig,
        tx_cache: Option<TxCache>,
        spawner: Tasks,
    ) -> ProviderResult<Self>
    where
        Tasks: TaskSpawner + Clone + 'static,
    {
        RpcCtxInner::new(host, constants, factory, eth_config, tx_cache, spawner)
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

/// Inner context for [`RpcCtx`].
#[derive(Debug)]
pub struct RpcCtxInner<Host, Signet>
where
    Host: FullNodeComponents,
    Signet: Pnt,
{
    host: Host,
    signet: SignetCtx<Signet>,
}

impl<Host, Signet> RpcCtxInner<Host, Signet>
where
    Host: FullNodeComponents,
    Signet: Pnt,
{
    /// Create a new `RpcCtxInner`.
    pub fn new<Tasks>(
        host: Host,
        constants: SignetSystemConstants,
        factory: ProviderFactory<Signet>,
        eth_config: EthConfig,
        tx_cache: Option<TxCache>,
        spawner: Tasks,
    ) -> ProviderResult<Self>
    where
        Tasks: TaskSpawner + Clone + 'static,
    {
        SignetCtx::new(constants, factory, eth_config, tx_cache, spawner)
            .map(|signet| Self { host, signet })
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

    /// Create a trevm instance.
    pub fn trevm(
        &self,
        block_id: BlockId,
        block: &Header,
    ) -> Result<EvmNeedsTx<RuRevmState>, EthApiError> {
        // decrement if the id is pending, so that the state is on the latest block
        let height = block.number() - block_id.is_pending() as u64;
        let spec_id = self.signet.evm_spec_id(block);

        let db = self.signet.state_provider_database(height)?;

        let mut trevm = signet_evm::signet_evm(db, self.signet.constants().clone())
            .fill_cfg(&self.signet)
            .fill_block(block);

        trevm.set_spec_id(spec_id);

        Ok(trevm)
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
