//! `web3` namespace RPC handlers.

use crate::config::StorageRpcCtx;
use alloy::primitives::{B256, Bytes, keccak256};
use signet_hot::{HotKv, model::HotKvRead};
use trevm::revm::database::DBErrorMarker;

/// Instantiate the `web3` API router.
pub(crate) fn web3<H>() -> ajj::Router<StorageRpcCtx<H>>
where
    H: HotKv + Send + Sync + 'static,
    <H::RoTx as HotKvRead>::Error: DBErrorMarker,
{
    ajj::Router::new().route("clientVersion", client_version).route("sha3", sha3)
}

/// `web3_clientVersion` — returns the signet client version string.
pub(crate) async fn client_version() -> Result<String, ()> {
    Ok(format!("signet/v{}/{}", env!("CARGO_PKG_VERSION"), std::env::consts::OS,))
}

/// `web3_sha3` — returns the keccak256 hash of the given data.
pub(crate) async fn sha3((data,): (Bytes,)) -> Result<B256, ()> {
    Ok(keccak256(&data))
}

#[cfg(test)]
mod tests {
    use super::{client_version, sha3};
    use alloy::primitives::{Bytes, keccak256};

    #[tokio::test]
    async fn client_version_format() {
        let version = client_version().await.unwrap();
        assert!(version.starts_with("signet/v"), "got: {version}");
        assert!(version.contains('/'), "expected platform suffix, got: {version}");
    }

    #[tokio::test]
    async fn sha3_empty_input() {
        let result = sha3((Bytes::new(),)).await.unwrap();
        assert_eq!(result, keccak256(b""));
    }

    #[tokio::test]
    async fn sha3_nonempty_input() {
        let input = Bytes::from_static(b"hello");
        let result = sha3((input.clone(),)).await.unwrap();
        assert_eq!(result, keccak256(&input));
    }
}
