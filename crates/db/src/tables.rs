use alloy::{
    primitives::{bytes::BufMut, Address, BlockNumber, Bytes, B256, U256},
    rlp::Buf,
};
use reth_db::{
    table::{Compress, Decompress, DupSort, Table},
    tables, DatabaseError,
};
use signet_zenith::{
    Passage::{Enter, EnterToken},
    Transactor::Transact,
    Zenith::{self, BlockHeader},
};

const FLAG_TRANSACT: u8 = 0;
const FLAG_ENTER: u8 = 1;
const FLAG_ENTER_TOKEN: u8 = 2;

/// Table that maps heights numbers to zenith headers. We reuse a table from
/// reth's existing schema for this.
#[derive(Debug, Clone, Copy)]
pub struct ZenithHeaders {
    _private: (),
}

impl Table for ZenithHeaders {
    const NAME: &'static str = <tables::AccountsTrie as Table>::NAME;

    const DUPSORT: bool = <tables::AccountsTrie as Table>::DUPSORT;

    type Key = u64;

    type Value = DbZenithHeader;
}

/// Newtype for [`BlockHeader`] that implements [`Compress`] and [`Decompress`].
///
/// This is an implementation detail of the [`ZenithHeaders`] table, and should
/// not be used outside the DB module.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct DbZenithHeader(pub BlockHeader);

impl From<BlockHeader> for DbZenithHeader {
    fn from(header: BlockHeader) -> Self {
        Self(header)
    }
}

impl From<DbZenithHeader> for BlockHeader {
    fn from(header: DbZenithHeader) -> Self {
        header.0
    }
}

impl Compress for DbZenithHeader {
    type Compressed = <Vec<u8> as Compress>::Compressed;

    fn compress_to_buf<B: BufMut + AsMut<[u8]>>(&self, buf: &mut B) {
        let DbZenithHeader(Zenith::BlockHeader {
            rollupChainId,
            hostBlockNumber,
            gasLimit,
            rewardAddress,
            blockDataHash,
        }) = self;
        buf.put_slice(&rollupChainId.to_le_bytes::<32>());
        buf.put_slice(&hostBlockNumber.to::<u64>().to_le_bytes());
        buf.put_slice(&gasLimit.to::<u64>().to_le_bytes());
        buf.put_slice(rewardAddress.as_ref());
        buf.put_slice(blockDataHash.as_ref());
    }
}

impl Decompress for DbZenithHeader {
    fn decompress(value: &[u8]) -> Result<Self, DatabaseError> {
        if value.len() < 32 + 8 + 8 + 20 + 32 {
            tracing::error!(target: "signet", "decoding error");
            return Err(DatabaseError::Decode);
        }

        Ok(Self(Zenith::BlockHeader {
            rollupChainId: U256::from_le_slice(&value[0..32]),
            hostBlockNumber: U256::from(u64::from_le_bytes(value[32..40].try_into().unwrap())),
            gasLimit: U256::from(u64::from_le_bytes(value[40..48].try_into().unwrap())),
            rewardAddress: Address::from_slice(&value[48..68]),
            blockDataHash: B256::from_slice(&value[68..100]),
        }))
    }
}

/// Newtype for extracted Signet events that implements [`Compress`] and
/// [`Decompress`].
///
/// This is an implementation detail of the [`SignetEvents`] table, and should
/// not be used outside the DB module.
///
/// Each event is stored as a separate entry in the same table.
/// The first element of each event tuple is the event's order within all
/// events in the block.
///
/// The second element is the event itself.
///
/// We reuse a table from reth's existing schema for this table.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum DbSignetEvent {
    /// Each Transact event is stored as a separate entry in the same table.
    Transact(u64, Transact),
    /// Each Enter event is stored as a separate entry in the same table.
    Enter(u64, Enter),
    /// Each EnterToken event is stored as a separate entry in the same table.
    EnterToken(u64, EnterToken),
}

/// Table that maps block number and index number to signet events. We reuse a
/// table from reth's existing schema for this. The key is the rollup block
/// number, and the subkey is the index of the event within the block.
#[derive(Debug, Clone, Copy)]
pub struct SignetEvents {
    _private: (),
}

impl Table for SignetEvents {
    const NAME: &'static str = <tables::HashedStorages as Table>::NAME;

    const DUPSORT: bool = <tables::HashedStorages as Table>::DUPSORT;

    type Key = BlockNumber;

    type Value = DbSignetEvent;
}

impl DupSort for SignetEvents {
    type SubKey = u64;
}

/// Newtype for [`Transactor::Transact`] that implements [`Compress`] and
/// [`Decompress`].
///
/// This is an implementation detail of the [`SignetEvents`] table, and
/// should not be used outside the DB module.
///
/// The two fields are the index of the transact within the set of all
/// transacts within the block, and the transact itself. The index is used as a
/// subkey in the table. I.e. if this is the first transact in the block, the
/// index would be 0.
///
/// [`Transactor::Transact`]: signet_zenith::Transactor::Transact
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DbTransact(pub u64, pub Transact);

impl From<DbTransact> for Transact {
    fn from(transact: DbTransact) -> Self {
        transact.1
    }
}

// TODO: ENG-484 - Consider using CompactU256
// https://linear.app/initiates/issue/ENG-484/consider-using-compactu256-in-compress-impls
impl Compress for DbTransact {
    type Compressed = <Vec<u8> as Compress>::Compressed;

    fn compress_to_buf<B: BufMut + AsMut<[u8]>>(&self, buf: &mut B) {
        let Transact { rollupChainId, sender, to, data, value, gas, maxFeePerGas } = &self.1;

        buf.put_slice(&self.0.to_be_bytes());
        buf.put_slice(&rollupChainId.to_le_bytes::<32>());
        buf.put_slice(sender.as_ref());
        buf.put_slice(to.as_ref());
        buf.put_slice(&value.to_le_bytes::<32>());
        buf.put_slice(&gas.to_le_bytes::<32>());
        buf.put_slice(&maxFeePerGas.to_le_bytes::<32>());
        // variable element last
        buf.put_slice(data.as_ref());
    }
}

impl Decompress for DbTransact {
    fn decompress(value: &[u8]) -> Result<Self, DatabaseError> {
        if value.len() < 176 {
            return Err(DatabaseError::Decode);
        }

        let data = if value.len() >= 176 {
            Bytes::copy_from_slice(&value[176..])
        } else {
            Default::default()
        };

        Ok(Self(
            u64::from_be_bytes(value[0..8].try_into().unwrap()),
            Transact {
                rollupChainId: U256::from_le_slice(&value[8..40]),
                sender: Address::from_slice(&value[40..60]),
                to: Address::from_slice(&value[60..80]),
                data,
                value: U256::from_le_slice(&value[80..112]),
                gas: U256::from_le_slice(&value[112..144]),
                maxFeePerGas: U256::from_le_slice(&value[144..176]),
            },
        ))
    }
}

/// Newtype for [`Passage::Enter`] that implements [`Compress`] and
/// [`Decompress`].
///
/// This is an implementation detail of the [`SignetEvents`] table, and should
/// not be used outside the DB module.
///
/// The two fields are the index of the enter within the set of all
/// transacts within the block, and the enter itself. The index is used as a
/// subkey in the table. I.e. if this is the first enter in the block, the
/// index would be 0.
///
/// [`Passage::Enter`]: signet_zenith::Passage::Enter
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct DbEnter(pub u64, pub Enter);

impl From<DbEnter> for Enter {
    fn from(enter: DbEnter) -> Self {
        enter.1
    }
}

// TODO: ENG-484 - Consider using CompactU256
impl Compress for DbEnter {
    type Compressed = <Vec<u8> as Compress>::Compressed;

    fn compress_to_buf<B: BufMut + AsMut<[u8]>>(&self, buf: &mut B) {
        // BE here is important for the subkey
        buf.put_slice(&self.0.to_be_bytes());
        buf.put_slice(&self.1.rollupChainId.to_le_bytes::<32>());
        buf.put_slice(self.1.rollupRecipient.as_ref());
        buf.put_slice(&self.1.amount.to_le_bytes::<32>());
    }
}

impl Decompress for DbEnter {
    fn decompress(value: &[u8]) -> Result<Self, DatabaseError> {
        if value.len() < 8 + 32 + 20 + 32 {
            return Err(DatabaseError::Decode);
        }

        Ok(Self(
            u64::from_be_bytes(value[0..8].try_into().unwrap()),
            Enter {
                rollupChainId: U256::from_le_slice(&value[8..40]),
                rollupRecipient: Address::from_slice(&value[40..60]),
                amount: U256::from_le_slice(&value[60..92]),
            },
        ))
    }
}

/// Newtype for [`Passage::EnterToken`] that implements [`Compress`] and
/// [`Decompress`].
///
/// This is an implementation detail of the [`SignetEvents`] table, and should
/// not be used outside the DB module.
///
/// [`Passage::EnterToken`]: signet_zenith::Passage::EnterToken
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct DbEnterToken(pub u64, pub EnterToken);

impl From<DbEnterToken> for EnterToken {
    fn from(enter_token: DbEnterToken) -> Self {
        enter_token.1
    }
}

impl Compress for DbEnterToken {
    type Compressed = <Vec<u8> as Compress>::Compressed;

    fn compress_to_buf<B: BufMut + AsMut<[u8]>>(&self, buf: &mut B) {
        buf.put_slice(&self.0.to_be_bytes()); // 8 bytes // BE here is important for the subkey
        buf.put_slice(&self.1.rollupChainId.to_le_bytes::<32>()); // 32 bytes
        buf.put_slice(self.1.rollupRecipient.as_slice()); // 20 bytes
        buf.put_slice(&self.1.amount.to_le_bytes::<32>()); // 32 bytes
        buf.put_slice(self.1.token.as_slice()); // 20 bytes
    }
}

impl Decompress for DbEnterToken {
    fn decompress(value: &[u8]) -> Result<Self, DatabaseError> {
        if value.len() < 8 + 32 + 20 + 32 + 20 {
            return Err(DatabaseError::Decode);
        }

        Ok(Self(
            u64::from_be_bytes(value[0..8].try_into().unwrap()),
            EnterToken {
                rollupChainId: U256::from_le_slice(&value[8..40]),
                rollupRecipient: Address::from_slice(&value[40..60]),
                amount: U256::from_le_slice(&value[60..92]),
                token: Address::from_slice(&value[92..112]),
            },
        ))
    }
}

impl Compress for DbSignetEvent {
    type Compressed = <Vec<u8> as Compress>::Compressed;

    fn compress_to_buf<B: alloy::rlp::bytes::BufMut + AsMut<[u8]>>(&self, buf: &mut B) {
        match self {
            Self::Transact(idx, transact) => {
                buf.put_u8(FLAG_TRANSACT);
                DbTransact(*idx, transact.clone()).compress_to_buf(buf);
            }
            Self::Enter(idx, enter) => {
                buf.put_u8(FLAG_ENTER);
                DbEnter(*idx, *enter).compress_to_buf(buf);
            }
            Self::EnterToken(idx, enter_token) => {
                buf.put_u8(FLAG_ENTER_TOKEN);
                DbEnterToken(*idx, *enter_token).compress_to_buf(buf);
            }
        }
    }
}

impl Decompress for DbSignetEvent {
    fn decompress(value: &[u8]) -> Result<Self, DatabaseError> {
        let value = &mut &*value;

        if value.is_empty() {
            return Err(DatabaseError::Decode);
        }

        match value.get_u8() {
            FLAG_TRANSACT => {
                let transact = DbTransact::decompress(value)?;
                Ok(Self::Transact(transact.0, transact.1))
            }
            FLAG_ENTER => {
                let enter = DbEnter::decompress(value)?;
                Ok(Self::Enter(enter.0, enter.1))
            }
            FLAG_ENTER_TOKEN => {
                let enter_token = DbEnterToken::decompress(value)?;
                Ok(Self::EnterToken(enter_token.0, enter_token.1))
            }
            _ => Err(DatabaseError::Decode),
        }
    }
}

/// Table that maps rollup block heights to post-block journal hashes.
#[derive(Debug, Clone, Copy)]
pub struct JournalHashes {
    _private: (),
}

impl Table for JournalHashes {
    const NAME: &'static str = <tables::HashedAccounts as Table>::NAME;

    const DUPSORT: bool = <tables::HashedAccounts as Table>::DUPSORT;

    type Key = u64;

    type Value = B256;
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn db_event_transact_roundtrip() {
        let event = DbSignetEvent::Transact(
            32,
            Transact {
                rollupChainId: U256::from(1),
                sender: Address::repeat_byte(1),
                to: Address::repeat_byte(2),
                data: Bytes::from(vec![1, 2, 3]),
                value: U256::from(100),
                gas: U256::from(200),
                maxFeePerGas: U256::from(300),
            },
        );

        let buf = event.clone().compress();

        let decompressed = DbSignetEvent::decompress(buf.as_slice()).unwrap();
        assert_eq!(event, decompressed);
    }

    #[test]
    fn db_event_enter_roundtrip() {
        let event = DbSignetEvent::Enter(
            32,
            Enter {
                rollupChainId: U256::from(1),
                rollupRecipient: Address::repeat_byte(1),
                amount: U256::from(100),
            },
        );

        let buf = event.clone().compress();

        let decompressed = DbSignetEvent::decompress(buf.as_slice()).unwrap();
        assert_eq!(event, decompressed);
    }

    #[test]
    fn db_event_enter_token_roundtrip() {
        let event = DbSignetEvent::EnterToken(
            32,
            EnterToken {
                rollupChainId: U256::from(1),
                rollupRecipient: Address::repeat_byte(1),
                amount: U256::from(100),
                token: Address::repeat_byte(2),
            },
        );

        let buf = event.clone().compress();

        let decompressed = DbSignetEvent::decompress(buf.as_slice()).unwrap();
        assert_eq!(event, decompressed);
    }
}
