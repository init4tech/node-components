use crate::{
    SignetNode,
    serve::{RpcServerGuard, ServeConfig},
};
use reth::primitives::EthPrimitives;
use reth_node_api::{FullNodeComponents, NodeTypes};
use signet_block_processor::AliasOracleFactory;
use signet_rpc::{StorageRpcConfig, StorageRpcCtx};
use signet_storage::HotKv;
use signet_tx_cache::TxCache;
use std::sync::Arc;
use tracing::info;

impl<Host, H, AliasOracle> SignetNode<Host, H, AliasOracle>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as signet_storage::HotKvRead>::Error: trevm::revm::database::DBErrorMarker,
    AliasOracle: AliasOracleFactory,
{
    /// Start the RPC server.
    pub async fn start_rpc(&mut self) -> eyre::Result<()> {
        let guard = self.launch_rpc().await?;
        self.rpc_handle = Some(guard);
        info!("launched rpc server");
        Ok(())
    }

    async fn launch_rpc(&self) -> eyre::Result<RpcServerGuard> {
        let tasks = self.host.task_executor();
        let tx_cache =
            self.config.forward_url().map(|url| TxCache::new_with_client(url, self.client.clone()));
        let rpc_ctx = StorageRpcCtx::new(
            Arc::clone(&self.storage),
            self.constants.clone(),
            self.chain.clone(),
            tx_cache,
            StorageRpcConfig::default(),
        );
        let router = signet_rpc::router::<H>().with_state(rpc_ctx);
        let serve_config: ServeConfig = self.config.merge_rpc_configs(&self.host)?.into();
        serve_config.serve(tasks, router).await
    }
}
