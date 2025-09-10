use crate::{Blobs, DecodeError, DecodeResult};
use alloy::{
    consensus::SidecarCoder,
    primitives::{B256, Bytes, keccak256},
};
use signet_zenith::{Zenith, ZenithBlock};

/// A trait for decoding blocks from blob data.
pub trait SignetBlockDecoder {
    /// Decodes a block from the given blob bytes.
    fn decode_block(
        &mut self,
        blobs: Blobs,
        header: Zenith::BlockHeader,
        data_hash: B256,
    ) -> DecodeResult<ZenithBlock>;

    /// Decodes a block from the given blob bytes, or returns an empty block.
    fn decode_block_or_default(
        &mut self,
        blobs: Blobs,
        header: Zenith::BlockHeader,
        data_hash: B256,
    ) -> ZenithBlock {
        self.decode_block(blobs, header, data_hash)
            .unwrap_or_else(|_| ZenithBlock::from_header_and_data(header, Bytes::new()))
    }
}

impl<T> SignetBlockDecoder for T
where
    T: SidecarCoder,
{
    fn decode_block(
        &mut self,
        blobs: Blobs,
        header: Zenith::BlockHeader,
        data_hash: B256,
    ) -> DecodeResult<ZenithBlock> {
        let block_data = self
            .decode_all(blobs.as_ref())
            .ok_or(DecodeError::BlobDecodeError)?
            .into_iter()
            .find(|data| keccak256(data) == data_hash)
            .map(Into::<Bytes>::into)
            .ok_or(DecodeError::BlockDataNotFound(data_hash))?;
        Ok(ZenithBlock::from_header_and_data(header, block_data))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{Blobs, utils::tests::PYLON_BLOB_RESPONSE};
    use alloy::{
        consensus::{BlobTransactionSidecar, SimpleCoder},
        primitives::{Address, B256, U256, b256},
    };
    use signet_zenith::Zenith;

    #[test]
    fn it_decodes() {
        let sidecar: BlobTransactionSidecar =
            serde_json::from_str::<BlobTransactionSidecar>(PYLON_BLOB_RESPONSE).unwrap();
        let blobs = Blobs::from(sidecar);

        let block = SimpleCoder::default()
            .decode_block(
                blobs,
                Zenith::BlockHeader {
                    rollupChainId: U256::ZERO,
                    hostBlockNumber: U256::ZERO,
                    gasLimit: U256::ZERO,
                    rewardAddress: Address::ZERO,
                    blockDataHash: B256::ZERO,
                },
                b256!("0xfd93968f4e7d4d4451f211980f2fec4f0c32e67fae63a70ca90024b54a70e9ee"),
            )
            .unwrap();

        assert_eq!(block.transactions().len(), 1);
    }

    #[test]
    fn it_decodes_defaultly() {
        let sidecar: BlobTransactionSidecar =
            serde_json::from_str::<BlobTransactionSidecar>(PYLON_BLOB_RESPONSE).unwrap();
        let blobs = Blobs::from(sidecar);

        let block = SimpleCoder::default().decode_block_or_default(
            blobs,
            Zenith::BlockHeader {
                rollupChainId: U256::ZERO,
                hostBlockNumber: U256::ZERO,
                gasLimit: U256::ZERO,
                rewardAddress: Address::ZERO,
                blockDataHash: B256::ZERO,
            },
            B256::ZERO,
        );

        assert_eq!(block.transactions().len(), 0);
    }
}
