use crate::{Blobs, FetchResult};
use alloy::{
    eips::{eip4844::kzg_to_versioned_hash, eip7691::MAX_BLOBS_PER_BLOCK_ELECTRA},
    primitives::B256,
};
use reth::rpc::types::beacon::sidecar::BeaconBlobBundle;
use smallvec::SmallVec;

/// Extracts the blobs from the [`BeaconBlobBundle`], and returns the blobs that match the versioned hashes in the transaction.
/// This also dedups any duplicate blobs if a builder lands the same blob multiple times in a block.
pub(crate) fn extract_blobs_from_bundle(
    bundle: BeaconBlobBundle,
    versioned_hashes: &[B256],
) -> FetchResult<Blobs> {
    let mut blobs = vec![];
    // NB: There can be, at most, 9 blobs per block from Pectra forwards. We'll never need more space than this, unless blob capacity is increased again or made dynamic.
    let mut seen_versioned_hashes: SmallVec<[B256; MAX_BLOBS_PER_BLOCK_ELECTRA as usize]> =
        SmallVec::new();

    for item in bundle.data.iter() {
        let versioned_hash = kzg_to_versioned_hash(item.kzg_commitment.as_ref());

        if versioned_hashes.contains(&versioned_hash)
            && !seen_versioned_hashes.contains(&versioned_hash)
        {
            blobs.push(*item.blob);
            seen_versioned_hashes.push(versioned_hash);
        }
    }

    Ok(blobs.into())
}

#[cfg(test)]
pub(crate) mod tests {

    use super::*;
    use alloy::{
        consensus::{BlobTransactionSidecar, SidecarCoder, SimpleCoder, Transaction, TxEnvelope},
        eips::eip7594::BlobTransactionSidecarVariant,
    };
    use signet_types::primitives::TransactionSigned;
    use std::sync::Arc;

    pub(crate) const BLOBSCAN_BLOB_RESPONSE: &str =
        include_str!("../../../tests/artifacts/blob.json");
    /// Blob from Slot 2277733, corresponding to block 277722 on Pecorino host.
    pub(crate) const CL_BLOB_RESPONSE: &str = include_str!("../../../tests/artifacts/cl_blob.json");
    /// EIP4844 blob tx with hash 0x73d1c682fae85c761528a0a7ec22fac613b25ede87b80f0ac052107f3444324f,
    /// corresponding to blob sent to block 277722 on Pecorino host.
    pub(crate) const CL_BLOB_TX: &str = include_str!("../../../tests/artifacts/cl_blob_tx.json");
    /// Blob sidecar from Pylon, corresponding to block 277722 on Pecorino host.
    pub(crate) const PYLON_BLOB_RESPONSE: &str =
        include_str!("../../../tests/artifacts/pylon_blob.json");

    #[test]
    fn test_process_blob_extraction() {
        let bundle: BeaconBlobBundle = serde_json::from_str(CL_BLOB_RESPONSE).unwrap();
        let tx: TxEnvelope = serde_json::from_str::<TxEnvelope>(CL_BLOB_TX).unwrap();
        let tx: TransactionSigned = tx.into();

        let versioned_hashes = tx.blob_versioned_hashes().unwrap().to_owned();

        // Extract the blobs from the CL beacon blob bundle.
        let cl_blobs = extract_blobs_from_bundle(bundle, &versioned_hashes).unwrap();
        assert_eq!(cl_blobs.len(), 1);

        // Now, process the pylon blobs which come in a [`BlobTransactionSidecar`].
        // NB: this should be changes to `BlobTransactionSidecarVariant` in the
        // future. After https://github.com/alloy-rs/alloy/pull/2713
        // The json is definitely a `BlobTransactionSidecar`, so we can
        // deserialize it directly and it doesn't really matter much.
        let sidecar: BlobTransactionSidecar =
            serde_json::from_str::<BlobTransactionSidecar>(PYLON_BLOB_RESPONSE).unwrap();
        let pylon_blobs: Blobs = Arc::<BlobTransactionSidecarVariant>::new(sidecar.into()).into();

        // Make sure that both blob sources have the same blobs after being processed.
        assert_eq!(cl_blobs.len(), pylon_blobs.len());
        assert_eq!(cl_blobs.as_slice(), pylon_blobs.as_slice());

        // Make sure both can be decoded
        let cl_decoded = SimpleCoder::default().decode_all(cl_blobs.as_ref()).unwrap();
        let pylon_decoded = SimpleCoder::default().decode_all(pylon_blobs.as_ref()).unwrap();
        assert_eq!(cl_decoded.len(), pylon_decoded.len());
        assert_eq!(cl_decoded, pylon_decoded);
    }
}
