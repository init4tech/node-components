//! Type conversion traits and implementations for converting between Reth, Alloy, and Signet types.
//!
//! This module provides a set of conversion traits that enable seamless
//! interoperability between different type systems used in the Ethereum
//! ecosystem:
//!
//! - **Reth types**: Core primitives from the Reth Ethereum client
//! - **Alloy types**: Modern Ethereum types from the Alloy framework
//! - **Signet types**: Custom types specific to the Signet protocol
use alloy::consensus::TxReceipt;

/// Trait for types that can be converted into other types as they're already compatible.
/// Uswed for converting between alloy/reth/signet types.
pub trait DataCompat<Other: DataCompat<Self>>: Sized {
    /// Convert `self` into the target type.
    fn convert(self) -> Other;

    /// Convert `self` into the target type by cloning.
    fn clone_convert(&self) -> Other
    where
        Self: Clone,
    {
        self.clone().convert()
    }
}

impl<T, U> DataCompat<Vec<U>> for Vec<T>
where
    U: DataCompat<T>,
    T: DataCompat<U>,
{
    fn convert(self) -> Vec<U> {
        self.into_iter().map(|item| item.convert()).collect()
    }
}

impl DataCompat<reth::providers::ExecutionOutcome> for signet_evm::ExecutionOutcome {
    fn convert(self) -> reth::providers::ExecutionOutcome {
        let (bundle, receipts, first_block) = self.into_parts();

        reth::providers::ExecutionOutcome {
            bundle,
            receipts: receipts.convert(),
            first_block,
            requests: Default::default(), // Requests are not present in Signet's ExecutionOutcome
        }
    }
}

impl DataCompat<signet_evm::ExecutionOutcome> for reth::providers::ExecutionOutcome {
    fn convert(self) -> signet_evm::ExecutionOutcome {
        signet_evm::ExecutionOutcome::new(
            self.bundle,
            self.receipts.into_iter().map(DataCompat::convert).collect(),
            self.first_block,
        )
    }
}

impl DataCompat<reth::primitives::Receipt> for alloy::consensus::ReceiptEnvelope {
    fn convert(self) -> reth::primitives::Receipt {
        reth::primitives::Receipt {
            tx_type: self.tx_type(),
            success: self.is_success(),
            cumulative_gas_used: self.cumulative_gas_used(),
            logs: self.logs().to_owned(),
        }
    }
}

impl DataCompat<alloy::consensus::ReceiptEnvelope> for reth::primitives::Receipt {
    fn convert(self) -> alloy::consensus::ReceiptEnvelope {
        let receipt = alloy::consensus::Receipt {
            status: self.status().into(),
            cumulative_gas_used: self.cumulative_gas_used,
            logs: self.logs.to_owned(),
        };

        match self.tx_type {
            reth::primitives::TxType::Legacy => {
                alloy::consensus::ReceiptEnvelope::Legacy(receipt.into())
            }
            reth::primitives::TxType::Eip2930 => {
                alloy::consensus::ReceiptEnvelope::Eip2930(receipt.into())
            }
            reth::primitives::TxType::Eip1559 => {
                alloy::consensus::ReceiptEnvelope::Eip1559(receipt.into())
            }
            reth::primitives::TxType::Eip4844 => {
                alloy::consensus::ReceiptEnvelope::Eip4844(receipt.into())
            }
            reth::primitives::TxType::Eip7702 => {
                alloy::consensus::ReceiptEnvelope::Eip7702(receipt.into())
            }
        }
    }
}

impl DataCompat<signet_types::primitives::SealedHeader> for reth::primitives::SealedHeader {
    fn convert(self) -> signet_types::primitives::SealedHeader {
        signet_types::primitives::SealedHeader::new(self.into_header())
    }
}

impl DataCompat<reth::primitives::SealedHeader> for signet_types::primitives::SealedHeader {
    fn convert(self) -> reth::primitives::SealedHeader {
        reth::primitives::SealedHeader::new_unhashed(self.header().to_owned())
    }
}
