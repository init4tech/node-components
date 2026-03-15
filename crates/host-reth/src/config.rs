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
        .gas_oracle_block_count(u64::from(gpo.blocks))
        .gas_oracle_percentile(f64::from(gpo.percentile))
        .ignore_price(Some(u128::from(gpo.ignore_price)))
        .max_price(Some(u128::from(gpo.max_price)))
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

#[cfg(test)]
mod tests {
    use crate::config::{rpc_config_from_args, serve_config_from_args};
    use reth::args::RpcServerArgs;

    #[test]
    fn rpc_config_from_default_args() {
        let args = RpcServerArgs::default();
        let gpo = &args.gas_price_oracle;
        let config = rpc_config_from_args(&args);

        assert_eq!(config.rpc_gas_cap, args.rpc_gas_cap);
        assert_eq!(config.max_tracing_requests, args.rpc_max_tracing_requests);
        assert_eq!(config.gas_oracle_block_count, u64::from(gpo.blocks));
        assert_eq!(config.gas_oracle_percentile, f64::from(gpo.percentile));
        assert_eq!(config.ignore_price, Some(u128::from(gpo.ignore_price)));
        assert_eq!(config.max_price, Some(u128::from(gpo.max_price)));
    }

    #[test]
    fn serve_config_http_disabled_by_default() {
        let args = RpcServerArgs::default();
        let config = serve_config_from_args(&args);

        assert!(config.http.is_empty());
        assert!(config.ws.is_empty());
    }

    #[test]
    fn serve_config_http_enabled() {
        let args = RpcServerArgs { http: true, ..Default::default() };
        let config = serve_config_from_args(&args);

        assert_eq!(config.http.len(), 1);
        assert_eq!(config.http[0].port(), args.http_port);
    }

    #[test]
    fn serve_config_ws_enabled() {
        let args = RpcServerArgs { ws: true, ..Default::default() };
        let config = serve_config_from_args(&args);

        assert_eq!(config.ws.len(), 1);
        assert_eq!(config.ws[0].port(), args.ws_port);
    }

    #[test]
    fn serve_config_ipc_enabled_by_default() {
        let args = RpcServerArgs::default();
        let config = serve_config_from_args(&args);

        assert!(config.ipc.is_some());
    }
}
