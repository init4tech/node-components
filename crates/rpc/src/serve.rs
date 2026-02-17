//! RPC transport infrastructure (HTTP, WebSocket, IPC).
//!
//! Serves an [`ajj::Router`] over one or more transports. The
//! [`ServeConfig`] describes which addresses and endpoints to bind;
//! [`ServeConfig::serve`] starts all configured transports and returns
//! an [`RpcServerGuard`] that shuts them down on drop.

use ajj::{
    Router,
    pubsub::{Connect, ServerShutdown},
};
use axum::http::HeaderValue;
use interprocess::local_socket as ls;
use std::{future::IntoFuture, net::SocketAddr};
use tokio::{runtime::Handle, task::JoinHandle};
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tracing::error;

/// Errors that can occur when starting the RPC server.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// An I/O error (bind failure, IPC error, etc).
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Invalid CORS configuration.
    #[error(transparent)]
    Cors(#[from] CorsDomainError),
}

/// Error parsing a CORS domain configuration string.
#[derive(Debug, thiserror::Error)]
pub enum CorsDomainError {
    /// Wildcard `*` was mixed with explicit origins.
    #[error("wildcard origin `*` cannot be combined with other domains: {input}")]
    WildCardNotAllowed {
        /// The raw input string.
        input: String,
    },
    /// A domain could not be parsed as an HTTP header value.
    #[error("invalid CORS header value: {domain}")]
    InvalidHeader {
        /// The domain string that failed to parse.
        domain: String,
    },
}

/// Guard that shuts down the RPC servers on drop.
#[derive(Default)]
pub struct RpcServerGuard {
    http: Option<JoinHandle<()>>,
    ws: Option<JoinHandle<()>>,
    ipc: Option<ServerShutdown>,
}

impl core::fmt::Debug for RpcServerGuard {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RpcServerGuard")
            .field("http", &self.http.is_some())
            .field("ws", &self.ws.is_some())
            .field("ipc", &self.ipc.is_some())
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
        // IPC is handled by its own drop guard.
    }
}

/// Configuration for the RPC transport layer.
#[derive(Clone, Debug)]
pub struct ServeConfig {
    /// HTTP server bind addresses.
    pub http: Vec<SocketAddr>,
    /// CORS header to use for HTTP (if any).
    pub http_cors: Option<String>,
    /// WebSocket server bind addresses.
    pub ws: Vec<SocketAddr>,
    /// CORS header to use for WebSocket (if any).
    pub ws_cors: Option<String>,
    /// IPC endpoint path (e.g. `/tmp/signet.ipc`).
    pub ipc: Option<String>,
}

impl ServeConfig {
    /// Serve the router on all configured transports.
    ///
    /// Returns an [`RpcServerGuard`] that aborts the HTTP and WS
    /// servers on drop and signals the IPC server to shut down.
    pub async fn serve(&self, router: Router<()>) -> Result<RpcServerGuard, ServeError> {
        let handle = Handle::current();

        let (http, ws, ipc) = tokio::try_join!(
            self.serve_http(&handle, router.clone()),
            self.serve_ws(&handle, router.clone()),
            self.serve_ipc(&handle, &router),
        )?;

        Ok(RpcServerGuard { http, ws, ipc })
    }

    /// Start the HTTP transport (if configured).
    async fn serve_http(
        &self,
        handle: &Handle,
        router: Router<()>,
    ) -> Result<Option<JoinHandle<()>>, ServeError> {
        if self.http.is_empty() {
            return Ok(None);
        }
        serve_axum(handle, router, &self.http, self.http_cors.as_deref()).await.map(Some)
    }

    /// Start the WebSocket transport (if configured).
    async fn serve_ws(
        &self,
        handle: &Handle,
        router: Router<()>,
    ) -> Result<Option<JoinHandle<()>>, ServeError> {
        if self.ws.is_empty() {
            return Ok(None);
        }
        serve_ws_transport(handle, router, &self.ws, self.ws_cors.as_deref()).await.map(Some)
    }

    /// Start the IPC transport (if configured).
    async fn serve_ipc(
        &self,
        handle: &Handle,
        router: &Router<()>,
    ) -> Result<Option<ServerShutdown>, ServeError> {
        let Some(endpoint) = &self.ipc else { return Ok(None) };
        serve_ipc(handle, router, endpoint).await.map(Some)
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
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_origin(origins)
        .allow_headers(Any))
}

/// Bind a TCP listener and serve the axum service.
async fn bind_and_serve(
    addrs: &[SocketAddr],
    service: axum::Router,
) -> Result<JoinHandle<()>, ServeError> {
    let listener = tokio::net::TcpListener::bind(addrs).await?;

    Ok(tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, service).into_future().await {
            error!(%err, "error serving RPC via axum");
        }
    }))
}

/// Serve the router via HTTP with optional CORS.
async fn serve_axum(
    handle: &Handle,
    router: Router<()>,
    addrs: &[SocketAddr],
    cors: Option<&str>,
) -> Result<JoinHandle<()>, ServeError> {
    let cors = make_cors(cors)?;
    let service = router.into_axum_with_handle("/", handle.clone()).layer(cors);
    bind_and_serve(addrs, service).await
}

/// Serve the router via WebSocket with optional CORS.
async fn serve_ws_transport(
    handle: &Handle,
    router: Router<()>,
    addrs: &[SocketAddr],
    cors: Option<&str>,
) -> Result<JoinHandle<()>, ServeError> {
    let cors = make_cors(cors)?;
    let service = router.into_axum_with_ws_and_handle("/rpc", "/", handle.clone()).layer(cors);
    bind_and_serve(addrs, service).await
}

fn to_name(path: &std::ffi::OsStr) -> std::io::Result<ls::Name<'_>> {
    if cfg!(windows) && !path.as_encoded_bytes().starts_with(br"\\.\pipe\") {
        ls::ToNsName::to_ns_name::<ls::GenericNamespaced>(path)
    } else {
        ls::ToFsName::to_fs_name::<ls::GenericFilePath>(path)
    }
}

/// Serve the router via IPC.
async fn serve_ipc(
    handle: &Handle,
    router: &Router<()>,
    endpoint: &str,
) -> Result<ServerShutdown, ServeError> {
    let name = std::ffi::OsStr::new(endpoint);
    let name = to_name(name)?;
    ls::ListenerOptions::new()
        .name(name)
        .serve_with_handle(router.clone(), handle.clone())
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
