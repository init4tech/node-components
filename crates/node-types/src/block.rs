use alloy::eips::eip2718::{Decodable2718, Encodable2718};
use reth::primitives::TransactionSigned;
use signet_zenith::Coder;
use tracing::trace;

/// [signet_zenith::ZenithBlock] parameterized for use with reth.
pub type ZenithBlock = signet_zenith::ZenithBlock<Reth2718Coder>;

/// [Coder] implementation for reth's 2718 impl
#[derive(Debug, Clone, Copy)]
pub struct Reth2718Coder;

impl Coder for Reth2718Coder {
    type Tx = TransactionSigned;

    fn encode(t: &TransactionSigned) -> Vec<u8> {
        t.encoded_2718()
    }

    fn decode(buf: &mut &[u8]) -> Option<TransactionSigned> {
        TransactionSigned::decode_2718(buf)
            .inspect_err(|e| trace!(%e, "Discarding transaction due to failed decoding"))
            .ok()
            .filter(|tx| !tx.is_eip4844())
    }
}
