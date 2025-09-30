use crate::SignetNodeConfig;
use reth::args::RpcServerArgs;
use reth_exex::ExExContext;
use reth_node_api::FullNodeComponents;

impl SignetNodeConfig {
    /// Inherits the IP host address from the Reth RPC server configuration,
    /// and change the configured port for the RPC server. If the host server
    /// is configured to use IPC, Signet Node will use the endpoint specified by the
    /// environment variable `IPC_ENDPOINT`.
    fn modify_args(&self, sc: &RpcServerArgs) -> eyre::Result<RpcServerArgs> {
        let mut args = sc.clone();

        args.http_port = self.http_port();
        args.ws_port = self.ws_port();
        args.ipcpath = self.ipc_endpoint().map(ToOwned::to_owned).unwrap_or_default();

        Ok(args)
    }

    /// Merges Signet Node configurations over the transport and rpc server
    /// configurations, and returns the modified configs.
    pub fn merge_rpc_configs<Node>(&self, exex: &ExExContext<Node>) -> eyre::Result<RpcServerArgs>
    where
        Node: FullNodeComponents,
    {
        self.modify_args(&exex.config.rpc)
    }
}
