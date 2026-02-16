//! Property-based fuzz tests for transaction and block types.
//!
//! Uses proptest to verify structural invariants such as RLP encoding
//! roundtrips, transaction type preservation, and block construction
//! consistency.
//!
//! Control iteration count with `PROPTEST_CASES` env var (default: 64).

use alloy::{
    consensus::{Transaction as _, TxType},
    eips::eip2718::{Decodable2718, Encodable2718},
    primitives::Sealable,
};
use proptest::prelude::*;
use reth::primitives::{Block, Header, Transaction, TransactionSigned};
use signet_node_tests::fuzz::{
    arb_block, arb_block_body, arb_header, arb_transaction, arb_tx_eip1559, arb_tx_eip2930,
    arb_tx_eip4844, arb_tx_legacy, proptest_config,
};
use signet_node_tests::utils::make_wallet;

// ── Helpers ─────────────────────────────────────────────────────────

/// Sign a fuzzed transaction so it can be encoded as a [`TransactionSigned`].
fn sign_fuzzed(tx: Transaction) -> TransactionSigned {
    let wallet = make_wallet(1);
    signet_node_tests::utils::sign_tx_with_key_pair(&wallet, tx)
}

// ── Transaction type strategies ─────────────────────────────────────

proptest! {
    #![proptest_config(proptest_config())]

    // -- Legacy transactions --

    #[test]
    fn legacy_tx_roundtrip(tx in arb_tx_legacy()) {
        let signed = sign_fuzzed(tx);
        let encoded = signed.encoded_2718();
        let decoded = TransactionSigned::decode_2718(&mut encoded.as_slice())
            .expect("decode_2718 should succeed for a valid legacy tx");
        prop_assert_eq!(signed, decoded);
    }

    #[test]
    fn legacy_tx_type(tx in arb_tx_legacy()) {
        prop_assert_eq!(tx.tx_type(), TxType::Legacy);
    }

    // -- EIP-1559 transactions --

    #[test]
    fn eip1559_tx_roundtrip(tx in arb_tx_eip1559()) {
        let signed = sign_fuzzed(tx);
        let encoded = signed.encoded_2718();
        let decoded = TransactionSigned::decode_2718(&mut encoded.as_slice())
            .expect("decode_2718 should succeed for a valid eip1559 tx");
        prop_assert_eq!(signed, decoded);
    }

    #[test]
    fn eip1559_tx_type(tx in arb_tx_eip1559()) {
        prop_assert_eq!(tx.tx_type(), TxType::Eip1559);
    }

    // -- EIP-2930 transactions --

    #[test]
    fn eip2930_tx_roundtrip(tx in arb_tx_eip2930()) {
        let signed = sign_fuzzed(tx);
        let encoded = signed.encoded_2718();
        let decoded = TransactionSigned::decode_2718(&mut encoded.as_slice())
            .expect("decode_2718 should succeed for a valid eip2930 tx");
        prop_assert_eq!(signed, decoded);
    }

    #[test]
    fn eip2930_tx_type(tx in arb_tx_eip2930()) {
        prop_assert_eq!(tx.tx_type(), TxType::Eip2930);
    }

    // -- EIP-4844 transactions --

    #[test]
    fn eip4844_tx_roundtrip(tx in arb_tx_eip4844()) {
        let signed = sign_fuzzed(tx);
        let encoded = signed.encoded_2718();
        let decoded = TransactionSigned::decode_2718(&mut encoded.as_slice())
            .expect("decode_2718 should succeed for a valid eip4844 tx");
        prop_assert_eq!(signed, decoded);
    }

    #[test]
    fn eip4844_tx_type(tx in arb_tx_eip4844()) {
        prop_assert_eq!(tx.tx_type(), TxType::Eip4844);
    }

    // -- Mixed transaction roundtrips --

    #[test]
    fn any_tx_roundtrip(tx in arb_transaction()) {
        let signed = sign_fuzzed(tx);
        let encoded = signed.encoded_2718();
        let decoded = TransactionSigned::decode_2718(&mut encoded.as_slice())
            .expect("decode_2718 should succeed for any valid signed tx");
        prop_assert_eq!(signed, decoded);
    }

    #[test]
    fn any_tx_type_preserved(tx in arb_transaction()) {
        let original_type = tx.tx_type();
        let signed = sign_fuzzed(tx);
        let encoded = signed.encoded_2718();
        let decoded = TransactionSigned::decode_2718(&mut encoded.as_slice()).unwrap();
        prop_assert_eq!(decoded.tx().tx_type(), original_type);
    }

    #[test]
    fn signed_tx_length_is_nonzero(tx in arb_transaction()) {
        let signed = sign_fuzzed(tx);
        let encoded = signed.encoded_2718();
        prop_assert!(encoded.len() > 0);
    }

    // ── Block header tests ──────────────────────────────────────────

    #[test]
    fn header_seal_roundtrip(header in arb_header()) {
        // Sealing and splitting should preserve the header.
        let (sealed_header, hash) = header.clone().seal_slow().into_parts();
        prop_assert_eq!(&sealed_header, &header);
        // Hash should be deterministic.
        let (_, hash2) = header.seal_slow().into_parts();
        prop_assert_eq!(hash, hash2);
    }

    #[test]
    fn header_fields_preserved(header in arb_header()) {
        // Verify a few fields survive a clone round-trip (sanity check).
        let cloned = header.clone();
        prop_assert_eq!(header.number, cloned.number);
        prop_assert_eq!(header.gas_limit, cloned.gas_limit);
        prop_assert_eq!(header.timestamp, cloned.timestamp);
        prop_assert_eq!(header.parent_hash, cloned.parent_hash);
        prop_assert_eq!(header.beneficiary, cloned.beneficiary);
    }

    // ── Block body tests ────────────────────────────────────────────

    #[test]
    fn block_body_tx_count(body in arb_block_body()) {
        // Body should contain between 0 and 7 transactions.
        prop_assert!(body.transactions.len() < 8);
    }

    // ── Complete block tests ────────────────────────────────────────

    #[test]
    fn block_construction(block in arb_block()) {
        // A block's header and body should be accessible.
        let Block { header, body } = block;
        prop_assert!(header.gas_limit > 0);
        prop_assert!(body.transactions.len() < 8);
    }

    #[test]
    fn block_tx_roundtrips(block in arb_block()) {
        // Every transaction in a fuzzed block should survive encode/decode.
        for tx in &block.body.transactions {
            let signed = sign_fuzzed(tx.clone());
            let encoded = signed.encoded_2718();
            let decoded = TransactionSigned::decode_2718(&mut encoded.as_slice())
                .expect("tx in block should roundtrip");
            prop_assert_eq!(signed, decoded);
        }
    }

    // ── Zenith coder compatibility ──────────────────────────────────

    #[test]
    fn zenith_coder_roundtrip_non4844(tx in prop_oneof![arb_tx_legacy(), arb_tx_eip1559(), arb_tx_eip2930()]) {
        // The Signet zenith coder filters out EIP-4844, so non-4844 txs
        // should survive a coder encode → decode roundtrip.
        use signet_node_types::Reth2718Coder;
        use signet_zenith::Coder;

        let signed = sign_fuzzed(tx);
        let encoded = Reth2718Coder::encode(&signed);
        let decoded = Reth2718Coder::decode(&mut encoded.as_slice())
            .expect("non-4844 tx should survive zenith coder roundtrip");
        prop_assert_eq!(signed, decoded);
    }

    #[test]
    fn zenith_coder_rejects_eip4844(tx in arb_tx_eip4844()) {
        // The zenith coder explicitly rejects EIP-4844 transactions.
        use signet_node_types::Reth2718Coder;
        use signet_zenith::Coder;

        let signed = sign_fuzzed(tx);
        let encoded = Reth2718Coder::encode(&signed);
        let decoded = Reth2718Coder::decode(&mut encoded.as_slice());
        prop_assert!(decoded.is_none(), "eip4844 tx should be rejected by zenith coder");
    }

    // ── RLP encoding roundtrips ─────────────────────────────────────

    #[test]
    fn header_rlp_roundtrip(header in arb_header()) {
        use alloy_rlp::{Decodable, Encodable};

        let mut buf = Vec::new();
        header.encode(&mut buf);
        let decoded = Header::decode(&mut buf.as_slice())
            .expect("RLP decode should succeed for encoded header");
        prop_assert_eq!(header, decoded);
    }
}
