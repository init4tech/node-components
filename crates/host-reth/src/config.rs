use reth::args::RpcServerArgs;
use signet_rpc::{ServeConfig, StorageRpcConfig};
use std::net::SocketAddr;

/// Extract [`StorageRpcConfig`] values from reth's host RPC settings.
///
/// Fields with no reth equivalent retain their defaults.
pub fn rpc_config_from_args(args: &RpcServerArgs) -> StorageRpcConfig {
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
pub fn serve_config_from_args(args: &RpcServerArgs) -> ServeConfig {
    let http =
        if args.http { vec![SocketAddr::from((args.http_addr, args.http_port))] } else { vec![] };
    let ws = if args.ws { vec![SocketAddr::from((args.ws_addr, args.ws_port))] } else { vec![] };
    let ipc = if !args.ipcdisable { Some(args.ipcpath.clone()) } else { None };

    ServeConfig {
        http,
        http_cors: args.http_corsdomain.clone(),
        ws,
        ws_cors: args.ws_allowed_origins.clone(),
        ipc,
    }
}
