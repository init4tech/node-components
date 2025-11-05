use crate::SignetNode;
use reth::{primitives::EthPrimitives, rpc::builder::config::RethRpcServerConfig};
use reth_node_api::{FullNodeComponents, NodeTypes};
use signet_block_processor::AliasOracleFactory;
use signet_node_types::NodeTypesDbTrait;
use signet_rpc::{RpcCtx, RpcServerGuard, ServeConfig};
use signet_tx_cache::client::TxCache;
use tracing::info;

impl<Host, Db, AliasOracle> SignetNode<Host, Db, AliasOracle>
where
    Host: FullNodeComponents,
    Host::Types: NodeTypes<Primitives = EthPrimitives>,
    Db: NodeTypesDbTrait,
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
        let forwarder =
            self.config.forward_url().map(|url| TxCache::new_with_client(url, self.client.clone()));
        let eth_config = self.host.config.rpc.eth_config();
        let router = signet_rpc::router().with_state(RpcCtx::new(
            self.host.components.clone(),
            self.constants.clone(),
            self.bp.clone(),
            eth_config,
            forwarder,
            tasks.clone(),
        )?);
        let serve_config: ServeConfig = self.config.merge_rpc_configs(&self.host)?.into();
        serve_config.serve(tasks, router).await
    }
}
