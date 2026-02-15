use ajj::{
    Router,
    pubsub::{Connect, ServerShutdown},
};
use axum::http::HeaderValue;
use interprocess::local_socket as ls;
use reqwest::Method;
use reth::{args::RpcServerArgs, rpc::builder::CorsDomainError, tasks::TaskExecutor};
use std::{future::IntoFuture, net::SocketAddr};
use tokio::task::JoinHandle;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tracing::error;

/// Guard to shutdown the RPC servers. When dropped, this will shutdown all
/// running servers
#[derive(Default)]
pub(crate) struct RpcServerGuard {
    http: Option<tokio::task::JoinHandle<()>>,
    ws: Option<tokio::task::JoinHandle<()>>,
    ipc: Option<ServerShutdown>,
}

impl core::fmt::Debug for RpcServerGuard {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RpcServerGuard")
            .field("http", &self.http.is_some())
            .field("ipc", &self.ipc.is_some())
            .field("ws", &self.ws.is_some())
            .finish()
    }
}

impl Drop for RpcServerGuard {
    fn drop(&mut self) {
        if let Some(http) = self.http.take() {
            http.abort();
        }
        if let Some(ws) = self.ws.take() {
            ws.abort();
        }
        // IPC is handled by its own drop guards.
    }
}

/// Configuration for the RPC server.
#[derive(Clone, Debug)]
pub(crate) struct ServeConfig {
    /// HTTP server addresses.
    pub http: Vec<SocketAddr>,
    /// CORS header to be used for HTTP (if any).
    pub http_cors: Option<String>,
    /// WS server addresses.
    pub ws: Vec<SocketAddr>,
    /// CORS header to be used for WS (if any).
    pub ws_cors: Option<String>,
    /// IPC name info.
    pub ipc: Option<String>,
}

impl From<RpcServerArgs> for ServeConfig {
    fn from(args: RpcServerArgs) -> Self {
        let http = if args.http {
            vec![SocketAddr::from((args.http_addr, args.http_port))]
        } else {
            vec![]
        };
        let ws =
            if args.ws { vec![SocketAddr::from((args.ws_addr, args.ws_port))] } else { vec![] };

        let http_cors = args.http_corsdomain;
        let ws_cors = args.ws_allowed_origins;

        let ipc = if !args.ipcdisable { Some(args.ipcpath) } else { None };

        Self { http, http_cors, ws, ws_cors, ipc }
    }
}

impl ServeConfig {
    /// Serve the router on the given addresses.
    async fn serve_http(
        &self,
        tasks: &TaskExecutor,
        router: Router<()>,
    ) -> eyre::Result<Option<JoinHandle<()>>> {
        if self.http.is_empty() {
            return Ok(None);
        }
        serve_axum(tasks, router, &self.http, self.http_cors.as_deref()).await.map(Some)
    }

    /// Serve the router on the given addresses.
    async fn serve_ws(
        &self,
        tasks: &TaskExecutor,
        router: Router<()>,
    ) -> eyre::Result<Option<JoinHandle<()>>> {
        if self.ws.is_empty() {
            return Ok(None);
        }
        serve_ws(tasks, router, &self.ws, self.ws_cors.as_deref()).await.map(Some)
    }

    /// Serve the router on the given ipc path.
    async fn serve_ipc(
        &self,
        tasks: &TaskExecutor,
        router: &Router<()>,
    ) -> eyre::Result<Option<ServerShutdown>> {
        let Some(endpoint) = &self.ipc else { return Ok(None) };
        let shutdown = serve_ipc(tasks, router, endpoint).await?;
        Ok(Some(shutdown))
    }

    /// Serve the router.
    pub(crate) async fn serve(
        &self,
        tasks: &TaskExecutor,
        router: Router<()>,
    ) -> eyre::Result<RpcServerGuard> {
        let (http, ws, ipc) = tokio::try_join!(
            self.serve_http(tasks, router.clone()),
            self.serve_ws(tasks, router.clone()),
            self.serve_ipc(tasks, &router),
        )?;
        Ok(RpcServerGuard { http, ws, ipc })
    }
}

fn make_cors(cors: Option<&str>) -> Result<CorsLayer, CorsDomainError> {
    let origins = match cors {
        None | Some("*") => AllowOrigin::any(),
        Some(cors) => {
            if cors.split(',').any(|o| o == "*") {
                return Err(CorsDomainError::WildCardNotAllowed { input: cors.to_string() });
            }
            cors.split(',')
                .map(|domain| {
                    domain
                        .parse::<HeaderValue>()
                        .map_err(|_| CorsDomainError::InvalidHeader { domain: domain.to_string() })
                })
                .collect::<Result<Vec<_>, _>>()?
                .into()
        }
    };

    Ok(CorsLayer::new()
        .allow_methods([Method::GET, Method::POST])
        .allow_origin(origins)
        .allow_headers(Any))
}

/// Serve the axum router on the specified addresses.
async fn serve(
    tasks: &TaskExecutor,
    addrs: &[SocketAddr],
    service: axum::Router,
) -> Result<JoinHandle<()>, eyre::Error> {
    let listener = tokio::net::TcpListener::bind(addrs).await?;

    let fut = async move {
        match axum::serve(listener, service).into_future().await {
            Ok(_) => (),
            Err(err) => error!(%err, "Error serving RPC via axum"),
        }
    };

    Ok(tasks.spawn(fut))
}

/// Serve the router on the given addresses using axum.
async fn serve_axum(
    tasks: &TaskExecutor,
    router: Router<()>,
    addrs: &[SocketAddr],
    cors: Option<&str>,
) -> eyre::Result<JoinHandle<()>> {
    let handle = tasks.handle().clone();
    let cors = make_cors(cors)?;

    let service = router.into_axum_with_handle("/", handle).layer(cors);

    serve(tasks, addrs, service).await
}

/// Serve the router on the given address using a Websocket.
async fn serve_ws(
    tasks: &TaskExecutor,
    router: Router<()>,
    addrs: &[SocketAddr],
    cors: Option<&str>,
) -> eyre::Result<JoinHandle<()>> {
    let handle = tasks.handle().clone();
    let cors = make_cors(cors)?;

    let service = router.into_axum_with_ws_and_handle("/rpc", "/", handle).layer(cors);

    serve(tasks, addrs, service).await
}

fn to_name(path: &std::ffi::OsStr) -> std::io::Result<ls::Name<'_>> {
    if cfg!(windows) && !path.as_encoded_bytes().starts_with(br"\\.\pipe\") {
        ls::ToNsName::to_ns_name::<ls::GenericNamespaced>(path)
    } else {
        ls::ToFsName::to_fs_name::<ls::GenericFilePath>(path)
    }
}

/// Serve the router on the given address using IPC.
async fn serve_ipc(
    tasks: &TaskExecutor,
    router: &Router<()>,
    endpoint: &str,
) -> eyre::Result<ServerShutdown> {
    let name = std::ffi::OsStr::new(endpoint);
    let name = to_name(name).expect("invalid name");
    ls::ListenerOptions::new()
        .name(name)
        .serve_with_handle(router.clone(), tasks.handle().clone())
        .await
        .map_err(Into::into)
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
