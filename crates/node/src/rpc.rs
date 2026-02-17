use crate::SignetNode;
use reth::{args::RpcServerArgs, primitives::EthPrimitives};
use reth_node_api::{FullNodeComponents, NodeTypes};
use signet_block_processor::AliasOracleFactory;
use signet_rpc::{RpcServerGuard, ServeConfig, StorageRpcConfig, StorageRpcCtx};
use signet_storage::HotKv;
use signet_tx_cache::TxCache;
use std::{net::SocketAddr, sync::Arc};
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
        let tx_cache =
            self.config.forward_url().map(|url| TxCache::new_with_client(url, self.client.clone()));

        let args = self.config.merge_rpc_configs(&self.host)?;
        let rpc_config = rpc_config_from_args(&args);

        let rpc_ctx = StorageRpcCtx::new(
            Arc::clone(&self.storage),
            self.constants.clone(),
            self.config.genesis().config.clone(),
            self.chain.clone(),
            tx_cache,
            rpc_config,
        );
        let router = signet_rpc::router::<H>().with_state(rpc_ctx);

        let serve_config = serve_config_from_args(args);
        serve_config.serve(router).await.map_err(Into::into)
    }
}

/// Extract [`StorageRpcConfig`] values from reth's host RPC settings.
///
/// Fields with no reth equivalent retain their defaults.
fn rpc_config_from_args(args: &RpcServerArgs) -> StorageRpcConfig {
    let gpo = &args.gas_price_oracle;
    StorageRpcConfig::builder()
        .rpc_gas_cap(args.rpc_gas_cap)
        .max_tracing_requests(args.rpc_max_tracing_requests)
        .gas_oracle_block_count(gpo.blocks as u64)
        .gas_oracle_percentile(gpo.percentile as f64)
        .ignore_price(Some(gpo.ignore_price as u128))
        .max_price(Some(gpo.max_price as u128))
        .build()
}

/// Convert reth [`RpcServerArgs`] into a reth-free [`ServeConfig`].
fn serve_config_from_args(args: RpcServerArgs) -> ServeConfig {
    let http =
        if args.http { vec![SocketAddr::from((args.http_addr, args.http_port))] } else { vec![] };
    let ws = if args.ws { vec![SocketAddr::from((args.ws_addr, args.ws_port))] } else { vec![] };
    let ipc = if !args.ipcdisable { Some(args.ipcpath) } else { None };

    ServeConfig { http, http_cors: args.http_corsdomain, ws, ws_cors: args.ws_allowed_origins, ipc }
}
