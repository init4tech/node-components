use crate::SignetNode;
use signet_block_processor::AliasOracleFactory;
use signet_node_types::HostNotifier;
use signet_rpc::{RpcServerGuard, StorageRpcCtx};
use signet_storage::HotKv;
use signet_tx_cache::TxCache;
use std::sync::Arc;
use tracing::info;

impl<N, H, AliasOracle> SignetNode<N, H, AliasOracle>
where
    N: HostNotifier,
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as signet_storage::HotKvRead>::Error: trevm::revm::database::DBErrorMarker,
    AliasOracle: AliasOracleFactory,
{
    /// Start the RPC server.
    pub(crate) async fn start_rpc(&mut self) -> eyre::Result<()> {
        let guard = self.launch_rpc().await?;
        self.rpc_handle = Some(guard);
        info!("launched rpc server");
        Ok(())
    }

    async fn launch_rpc(&self) -> eyre::Result<RpcServerGuard> {
        let tx_cache =
            self.config.forward_url().map(|url| TxCache::new_with_client(url, self.client.clone()));

        let rpc_ctx = StorageRpcCtx::new(
            Arc::clone(&self.storage),
            self.constants.clone(),
            self.config.genesis().config.clone(),
            self.chain.clone(),
            tx_cache,
            self.rpc_config,
        );
        let router = signet_rpc::router::<H>().with_state(rpc_ctx);

        self.serve_config.clone().serve(router).await.map_err(Into::into)
    }
}
