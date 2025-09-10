#![doc = include_str!("../README.md")]
#![warn(
    missing_copy_implementations,
    missing_debug_implementations,
    missing_docs,
    unreachable_pub,
    clippy::missing_const_for_fn,
    rustdoc::all
)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![deny(unused_must_use, rust_2018_idioms)]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

mod blobs;
pub use blobs::{
    BlobCacher, BlobFetcher, BlobFetcherBuilder, BlobFetcherBuilderError, BlobFetcherConfig, Blobs,
    CacheHandle, FetchError, FetchResult,
};

mod coder;
pub use coder::{DecodeError, DecodeResult, SignetBlockDecoder};

mod error;
pub use error::{BlobberError, BlobberResult};

mod shim;
pub use shim::ExtractableChainShim;

pub(crate) mod utils;

#[cfg(test)]
mod test {
    use crate::utils::tests::BLOBSCAN_BLOB_RESPONSE;
    use foundry_blob_explorers::TransactionDetails;

    // Sanity check on dependency compatibility.
    #[test]
    fn test_deser_blob() {
        let _: TransactionDetails = serde_json::from_str(BLOBSCAN_BLOB_RESPONSE).unwrap();
    }
}
