use crate::ser::{DeserError, KeySer, MAX_KEY_SIZE, ValSer};
use alloy::{
    consensus::{EthereumTxEnvelope, Signed, TxEip1559, TxEip2930, TxEip4844, TxEip7702, TxLegacy},
    eips::{
        eip2930::{AccessList, AccessListItem},
        eip7702::{Authorization, SignedAuthorization},
    },
    primitives::{Address, B256, FixedBytes, Signature, TxKind, U256},
};
use reth::{
    primitives::{Account, Bytecode, Header, Log, LogData, TransactionSigned, TxType},
    revm::bytecode::{JumpTable, LegacyAnalyzedBytecode, eip7702::Eip7702Bytecode},
};
use reth_db_api::{
    BlockNumberList,
    models::{
        AccountBeforeTx, ShardedKey, StoredBlockBodyIndices, storage_sharded_key::StorageShardedKey,
    },
};

impl<T: KeySer> KeySer for ShardedKey<T> {
    const SIZE: usize = T::SIZE + u64::SIZE;

    fn encode_key<'a: 'c, 'b: 'c, 'c>(&'a self, buf: &'b mut [u8; MAX_KEY_SIZE]) -> &'c [u8] {
        let mut scratch = [0u8; MAX_KEY_SIZE];

        T::encode_key(&self.key, &mut scratch);
        scratch[T::SIZE..Self::SIZE].copy_from_slice(&self.highest_block_number.to_be_bytes());
        *buf = scratch;

        &buf[0..Self::SIZE]
    }

    fn decode_key(data: &[u8]) -> Result<Self, DeserError> {
        if data.len() < Self::SIZE {
            return Err(DeserError::InsufficientData { needed: Self::SIZE, available: data.len() });
        }

        let key = T::decode_key(&data[0..T::SIZE])?;
        let highest_block_number = u64::decode_key(&data[T::SIZE..T::SIZE + 8])?;
        Ok(Self { key, highest_block_number })
    }
}

impl KeySer for StorageShardedKey {
    const SIZE: usize = Address::SIZE + B256::SIZE + u64::SIZE;

    fn encode_key<'a: 'c, 'b: 'c, 'c>(&'a self, buf: &'b mut [u8; MAX_KEY_SIZE]) -> &'c [u8] {
        buf[0..Address::SIZE].copy_from_slice(self.address.as_slice());
        buf[Address::SIZE..Address::SIZE + B256::SIZE]
            .copy_from_slice(self.sharded_key.key.as_slice());
        buf[Address::SIZE + B256::SIZE..Self::SIZE]
            .copy_from_slice(&self.sharded_key.highest_block_number.to_be_bytes());

        &buf[0..Self::SIZE]
    }

    fn decode_key(mut data: &[u8]) -> Result<Self, DeserError> {
        if data.len() < Self::SIZE {
            return Err(DeserError::InsufficientData { needed: Self::SIZE, available: data.len() });
        }

        let address = Address::from_slice(&data[0..Address::SIZE]);
        data = &data[Address::SIZE..];

        let storage_key = B256::from_slice(&data[0..B256::SIZE]);
        data = &data[B256::SIZE..];

        let highest_block_number = u64::from_be_bytes(data[0..8].try_into().unwrap());

        Ok(Self { address, sharded_key: ShardedKey { key: storage_key, highest_block_number } })
    }
}

macro_rules! by_props {
    (@size $($prop:ident),* $(,)?) => {
       {
            0 $(
                + $prop.encoded_size()
            )+
        }
    };
    (@encode $buf:ident; $($prop:ident),* $(,)?) => {
        {
            $(
                $prop.encode_value_to($buf);
            )+
        }
    };
    (@decode $data:ident; $($prop:ident),* $(,)?) => {
        {
            $(
                *$prop = ValSer::decode_value($data)?;
                $data = &$data[$prop.encoded_size()..];
            )*
        }
    };
}

impl ValSer for BlockNumberList {
    fn encoded_size(&self) -> usize {
        2 + self.serialized_size()
    }

    fn encode_value_to<B>(&self, mut buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        use std::io::Write;
        let mut writer: bytes::buf::Writer<&mut B> = bytes::BufMut::writer(&mut buf);

        debug_assert!(
            self.serialized_size() <= u16::MAX as usize,
            "BlockNumberList too large to encode"
        );

        writer.write_all(&(self.serialized_size() as u16).to_be_bytes()).unwrap();
        self.serialize_into(writer).unwrap();
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let size = u16::decode_value(&data[..2])? as usize;
        BlockNumberList::from_bytes(&data[2..2 + size])
            .map_err(|err| DeserError::String(format!("Failed to decode BlockNumberList {err}")))
    }
}

impl ValSer for Header {
    fn encoded_size(&self) -> usize {
        // NB: Destructure to ensure changes are compile errors and mistakes
        // are unused var warnings.
        let Header {
            parent_hash,
            ommers_hash,
            beneficiary,
            state_root,
            transactions_root,
            receipts_root,
            logs_bloom,
            difficulty,
            number,
            gas_limit,
            gas_used,
            timestamp,
            extra_data,
            mix_hash,
            nonce,
            base_fee_per_gas,
            withdrawals_root,
            blob_gas_used,
            excess_blob_gas,
            parent_beacon_block_root,
            requests_hash,
        } = self;

        by_props!(
            @size
            parent_hash,
            ommers_hash,
            beneficiary,
            state_root,
            transactions_root,
            receipts_root,
            logs_bloom,
            difficulty,
            number,
            gas_limit,
            gas_used,
            timestamp,
            extra_data,
            mix_hash,
            nonce,
            base_fee_per_gas,
            withdrawals_root,
            blob_gas_used,
            excess_blob_gas,
            parent_beacon_block_root,
            requests_hash,
        )
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        // NB: Destructure to ensure changes are compile errors and mistakes
        // are unused var warnings.
        let Header {
            parent_hash,
            ommers_hash,
            beneficiary,
            state_root,
            transactions_root,
            receipts_root,
            logs_bloom,
            difficulty,
            number,
            gas_limit,
            gas_used,
            timestamp,
            extra_data,
            mix_hash,
            nonce,
            base_fee_per_gas,
            withdrawals_root,
            blob_gas_used,
            excess_blob_gas,
            parent_beacon_block_root,
            requests_hash,
        } = self;

        by_props!(
            @encode buf;
            parent_hash,
            ommers_hash,
            beneficiary,
            state_root,
            transactions_root,
            receipts_root,
            logs_bloom,
            difficulty,
            number,
            gas_limit,
            gas_used,
            timestamp,
            extra_data,
            mix_hash,
            nonce,
            base_fee_per_gas,
            withdrawals_root,
            blob_gas_used,
            excess_blob_gas,
            parent_beacon_block_root,
            requests_hash,
        )
    }

    fn decode_value(mut data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        // NB: Destructure to ensure changes are compile errors and mistakes
        // are unused var warnings.
        let mut h = Header::default();
        let Header {
            parent_hash,
            ommers_hash,
            beneficiary,
            state_root,
            transactions_root,
            receipts_root,
            logs_bloom,
            difficulty,
            number,
            gas_limit,
            gas_used,
            timestamp,
            extra_data,
            mix_hash,
            nonce,
            base_fee_per_gas,
            withdrawals_root,
            blob_gas_used,
            excess_blob_gas,
            parent_beacon_block_root,
            requests_hash,
        } = &mut h;

        by_props!(
            @decode data;
            parent_hash,
            ommers_hash,
            beneficiary,
            state_root,
            transactions_root,
            receipts_root,
            logs_bloom,
            difficulty,
            number,
            gas_limit,
            gas_used,
            timestamp,
            extra_data,
            mix_hash,
            nonce,
            base_fee_per_gas,
            withdrawals_root,
            blob_gas_used,
            excess_blob_gas,
            parent_beacon_block_root,
            requests_hash,
        );
        Ok(h)
    }
}

impl ValSer for Account {
    fn encoded_size(&self) -> usize {
        // NB: Destructure to ensure changes are compile errors and mistakes
        // are unused var warnings.
        let Account { nonce, balance, bytecode_hash } = self;
        by_props!(
            @size
            nonce,
            balance,
            bytecode_hash,
        )
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        // NB: Destructure to ensure changes are compile errors and mistakes
        // are unused var warnings.
        let Account { nonce, balance, bytecode_hash } = self;
        by_props!(
            @encode buf;
            nonce,
            balance,
            bytecode_hash,
        )
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        // NB: Destructure to ensure changes are compile errors and mistakes
        // are unused var warnings.
        let mut account = Account::default();
        let Account { nonce, balance, bytecode_hash } = &mut account;

        let mut data = data;
        by_props!(
            @decode data;
            nonce,
            balance,
            bytecode_hash,
        );
        Ok(account)
    }
}

impl ValSer for LogData {
    fn encoded_size(&self) -> usize {
        let LogData { data, .. } = self;
        let topics = self.topics();
        2 + topics.iter().map(|t| t.encoded_size()).sum::<usize>() + data.encoded_size()
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        let LogData { data, .. } = self;
        let topics = self.topics();
        buf.put_u16(topics.len() as u16);
        for topic in topics {
            topic.encode_value_to(buf);
        }
        data.encode_value_to(buf);
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let mut data = data;
        let topics_len = u16::decode_value(&data[..2])? as usize;
        data = &data[2..];

        if topics_len > 4 {
            return Err(DeserError::String("LogData topics length exceeds maximum of 4".into()));
        }

        let mut topics = Vec::with_capacity(topics_len);
        for _ in 0..topics_len {
            let topic = B256::decode_value(data)?;
            data = &data[topic.encoded_size()..];
            topics.push(topic);
        }

        let log_data = alloy::primitives::Bytes::decode_value(data)?;

        Ok(LogData::new_unchecked(topics, log_data))
    }
}

impl ValSer for Log {
    fn encoded_size(&self) -> usize {
        let Log { address, data } = self;
        by_props!(
            @size
            address,
            data,
        )
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        let Log { address, data } = self;
        by_props!(
            @encode buf;
            address,
            data,
        )
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let mut log = Log::<LogData>::default();
        let Log { address, data: log_data } = &mut log;

        let mut data = data;
        by_props!(
            @decode data;
            address,
            log_data,
        );
        Ok(log)
    }
}

impl ValSer for TxType {
    fn encoded_size(&self) -> usize {
        1
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        buf.put_u8(*self as u8);
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let byte = u8::decode_value(data)?;
        TxType::try_from(byte)
            .map_err(|_| DeserError::String(format!("Invalid TxType value: {}", byte)))
    }
}

impl ValSer for StoredBlockBodyIndices {
    fn encoded_size(&self) -> usize {
        let StoredBlockBodyIndices { first_tx_num, tx_count } = self;
        by_props!(
            @size
            first_tx_num,
            tx_count,
        )
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        let StoredBlockBodyIndices { first_tx_num, tx_count } = self;
        by_props!(
            @encode buf;
            first_tx_num,
            tx_count,
        )
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let mut indices = StoredBlockBodyIndices::default();
        let StoredBlockBodyIndices { first_tx_num, tx_count } = &mut indices;

        let mut data = data;
        by_props!(
            @decode data;
            first_tx_num,
            tx_count,
        );
        Ok(indices)
    }
}

impl ValSer for Eip7702Bytecode {
    fn encoded_size(&self) -> usize {
        let Eip7702Bytecode { delegated_address, version, raw } = self;
        by_props!(
            @size
            delegated_address,
            version,
            raw,
        )
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        let Eip7702Bytecode { delegated_address, version, raw } = self;
        by_props!(
            @encode buf;
            delegated_address,
            version,
            raw,
        )
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let mut eip7702 = Eip7702Bytecode {
            delegated_address: Address::ZERO,
            version: 0,
            raw: alloy::primitives::Bytes::new(),
        };
        let Eip7702Bytecode { delegated_address, version, raw } = &mut eip7702;

        let mut data = data;
        by_props!(
            @decode data;
            delegated_address,
            version,
            raw,
        );
        Ok(eip7702)
    }
}

impl ValSer for JumpTable {
    fn encoded_size(&self) -> usize {
        2 + 2 + self.as_slice().len()
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        debug_assert!(self.len() <= u16::MAX as usize, "JumpTable bitlen too large to encode");
        debug_assert!(self.as_slice().len() <= u16::MAX as usize, "JumpTable too large to encode");
        buf.put_u16(self.len() as u16);
        buf.put_u16(self.as_slice().len() as u16);
        buf.put_slice(self.as_slice());
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let bit_len = u16::decode_value(&data[..2])? as usize;
        let slice_len = u16::decode_value(&data[2..4])? as usize;
        Ok(JumpTable::from_slice(&data[4..4 + slice_len], bit_len))
    }
}

impl ValSer for LegacyAnalyzedBytecode {
    fn encoded_size(&self) -> usize {
        let bytecode = self.bytecode();
        let original_len = self.original_len();
        let jump_table = self.jump_table();
        by_props!(
            @size
            bytecode,
            original_len,
            jump_table,
        )
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        let bytecode = self.bytecode();
        let original_len = self.original_len();
        let jump_table = self.jump_table();
        by_props!(
            @encode buf;
            bytecode,
            original_len,
            jump_table,
        )
    }

    fn decode_value(mut data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let mut bytecode = alloy::primitives::Bytes::new();
        let mut original_len = 0usize;
        let mut jump_table = JumpTable::default();

        let bc = &mut bytecode;
        let ol = &mut original_len;
        let jt = &mut jump_table;
        by_props!(
            @decode data;
            bc,
            ol,
            jt,
        );
        Ok(LegacyAnalyzedBytecode::new(bytecode, original_len, jump_table))
    }
}

impl ValSer for Bytecode {
    fn encoded_size(&self) -> usize {
        1 + match &self.0 {
            reth::revm::state::Bytecode::Eip7702(code) => code.encoded_size(),
            reth::revm::state::Bytecode::LegacyAnalyzed(code) => code.encoded_size(),
        }
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        match &self.0 {
            reth::revm::state::Bytecode::Eip7702(code) => {
                buf.put_u8(1);
                code.encode_value_to(buf);
            }
            reth::revm::state::Bytecode::LegacyAnalyzed(code) => {
                buf.put_u8(0);
                code.encode_value_to(buf);
            }
        }
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let ty = u8::decode_value(&data[..1])?;
        let data = &data[1..];
        match ty {
            0 => {
                let analyzed = LegacyAnalyzedBytecode::decode_value(data)?;
                Ok(Bytecode(reth::revm::state::Bytecode::LegacyAnalyzed(analyzed)))
            }
            1 => {
                let eip7702 = Eip7702Bytecode::decode_value(data)?;
                Ok(Bytecode(reth::revm::state::Bytecode::Eip7702(eip7702)))
            }
            _ => Err(DeserError::String(format!("Invalid Bytecode type value: {}. Max is 1.", ty))),
        }
    }
}

impl ValSer for AccountBeforeTx {
    fn encoded_size(&self) -> usize {
        let AccountBeforeTx { address, info } = self;
        by_props!(
            @size
            address,
            info,
        )
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        let AccountBeforeTx { address, info } = self;
        by_props!(
            @encode buf;
            address,
            info,
        )
    }

    fn decode_value(mut data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let mut abt = AccountBeforeTx::default();
        let AccountBeforeTx { address, info } = &mut abt;

        by_props!(
            @decode data;
            address,
            info,
        );
        Ok(abt)
    }
}

impl ValSer for Signature {
    fn encoded_size(&self) -> usize {
        65
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        FixedBytes(self.as_bytes()).encode_value_to(buf);
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let bytes = FixedBytes::<65>::decode_value(data)?;
        Self::from_raw_array(bytes.as_ref())
            .map_err(|e| DeserError::String(format!("Invalid signature bytes: {}", e)))
    }
}

impl ValSer for TxKind {
    fn encoded_size(&self) -> usize {
        1 + match self {
            TxKind::Create => 0,
            TxKind::Call(_) => 20,
        }
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        match self {
            TxKind::Create => {
                buf.put_u8(0);
            }
            TxKind::Call(address) => {
                buf.put_u8(1);
                address.encode_value_to(buf);
            }
        }
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let ty = u8::decode_value(&data[..1])?;
        let data = &data[1..];
        match ty {
            0 => Ok(TxKind::Create),
            1 => {
                let address = Address::decode_value(data)?;
                Ok(TxKind::Call(address))
            }
            _ => Err(DeserError::String(format!("Invalid TxKind type value: {}. Max is 1.", ty))),
        }
    }
}

impl ValSer for AccessListItem {
    fn encoded_size(&self) -> usize {
        let AccessListItem { address, storage_keys } = self;
        by_props!(
            @size
            address,
            storage_keys,
        )
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        let AccessListItem { address, storage_keys } = self;
        by_props!(
            @encode buf;
            address,
            storage_keys,
        )
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let mut item = AccessListItem::default();
        let AccessListItem { address, storage_keys } = &mut item;

        let mut data = data;
        by_props!(
            @decode data;
            address,
            storage_keys,
        );
        Ok(item)
    }
}

impl ValSer for AccessList {
    fn encoded_size(&self) -> usize {
        self.0.encoded_size()
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        self.0.encode_value_to(buf);
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        Vec::<AccessListItem>::decode_value(data).map(AccessList)
    }
}

impl ValSer for Authorization {
    fn encoded_size(&self) -> usize {
        let Authorization { chain_id, address, nonce } = self;
        by_props!(
            @size
            chain_id,
            address,
            nonce,
        )
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        let Authorization { chain_id, address, nonce } = self;
        by_props!(
            @encode buf;
            chain_id,
            address,
            nonce,
        )
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let mut auth = Authorization { chain_id: U256::ZERO, address: Address::ZERO, nonce: 0 };
        let Authorization { chain_id, address, nonce } = &mut auth;

        let mut data = data;
        by_props!(
            @decode data;
            chain_id,
            address,
            nonce,
        );
        Ok(auth)
    }
}

impl ValSer for SignedAuthorization {
    fn encoded_size(&self) -> usize {
        let auth = self.inner();
        let y_parity = self.y_parity();
        let r = self.r();
        let s = self.s();
        by_props!(
            @size
            auth,
            y_parity,
            r,
            s,
        )
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        let auth = self.inner();
        let y_parity = &self.y_parity();
        let r = &self.r();
        let s = &self.s();
        by_props!(
            @encode buf;
            auth,
            y_parity,
            r,
            s,
        )
    }

    fn decode_value(mut data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let mut auth = Authorization { chain_id: U256::ZERO, address: Address::ZERO, nonce: 0 };
        let mut y_parity = 0u8;
        let mut r = U256::ZERO;
        let mut s = U256::ZERO;

        let ap = &mut auth;
        let yp = &mut y_parity;
        let rr = &mut r;
        let ss = &mut s;

        by_props!(
            @decode data;
            ap,
            yp,
            rr,
            ss,
        );
        Ok(SignedAuthorization::new_unchecked(auth, y_parity, r, s))
    }
}

impl ValSer for TxLegacy {
    fn encoded_size(&self) -> usize {
        let TxLegacy { chain_id, nonce, gas_price, gas_limit, to, value, input } = self;
        by_props!(
            @size
            chain_id,
            nonce,
            gas_price,
            gas_limit,
            to,
            value,
            input,
        )
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        let TxLegacy { chain_id, nonce, gas_price, gas_limit, to, value, input } = self;
        by_props!(
            @encode buf;
            chain_id,
            nonce,
            gas_price,
            gas_limit,
            to,
            value,
            input,
        )
    }

    fn decode_value(mut data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let mut tx = TxLegacy::default();
        let TxLegacy { chain_id, nonce, gas_price, gas_limit, to, value, input } = &mut tx;

        by_props!(
            @decode data;
            chain_id,
            nonce,
            gas_price,
            gas_limit,
            to,
            value,
            input,
        );
        Ok(tx)
    }
}

impl ValSer for TxEip2930 {
    fn encoded_size(&self) -> usize {
        let TxEip2930 { chain_id, nonce, gas_price, gas_limit, to, value, input, access_list } =
            self;
        by_props!(
            @size
            chain_id,
            nonce,
            gas_price,
            gas_limit,
            to,
            value,
            input,
            access_list,
        )
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        let TxEip2930 { chain_id, nonce, gas_price, gas_limit, to, value, input, access_list } =
            self;
        by_props!(
            @encode buf;
            chain_id,
            nonce,
            gas_price,
            gas_limit,
            to,
            value,
            input,
            access_list,
        )
    }

    fn decode_value(mut data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let mut tx = TxEip2930::default();
        let TxEip2930 { chain_id, nonce, gas_price, gas_limit, to, value, input, access_list } =
            &mut tx;

        by_props!(
            @decode data;
            chain_id,
            nonce,
            gas_price,
            gas_limit,
            to,
            value,
            input,
            access_list,
        );
        Ok(tx)
    }
}

impl ValSer for TxEip1559 {
    fn encoded_size(&self) -> usize {
        let TxEip1559 {
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to,
            value,
            access_list,
            input,
        } = self;
        by_props!(
            @size
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to,
            value,
            access_list,
            input
        )
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        let TxEip1559 {
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to,
            value,
            access_list,
            input,
        } = self;
        by_props!(
            @encode buf;
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to,
            value,
            access_list,
            input
        )
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let mut tx = TxEip1559::default();
        let TxEip1559 {
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to,
            value,
            access_list,
            input,
        } = &mut tx;

        let mut data = data;
        by_props!(
            @decode data;
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to,
            value,
            access_list,
            input
        );
        Ok(tx)
    }
}

impl ValSer for TxEip4844 {
    fn encoded_size(&self) -> usize {
        let TxEip4844 {
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to,
            value,
            access_list,
            blob_versioned_hashes,
            max_fee_per_blob_gas,
            input,
        } = self;
        by_props!(
            @size
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to,
            value,
            access_list,
            blob_versioned_hashes,
            max_fee_per_blob_gas,
            input,
        )
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        let TxEip4844 {
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to,
            value,
            access_list,
            blob_versioned_hashes,
            max_fee_per_blob_gas,
            input,
        } = self;
        by_props!(
            @encode buf;
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to,
            value,
            access_list,
            blob_versioned_hashes,
            max_fee_per_blob_gas,
            input,
        )
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let mut tx = TxEip4844::default();
        let TxEip4844 {
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to,
            value,
            access_list,
            blob_versioned_hashes,
            max_fee_per_blob_gas,
            input,
        } = &mut tx;

        let mut data = data;
        by_props!(
            @decode data;
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to,
            value,
            access_list,
            blob_versioned_hashes,
            max_fee_per_blob_gas,
            input,
        );
        Ok(tx)
    }
}

impl ValSer for TxEip7702 {
    fn encoded_size(&self) -> usize {
        let TxEip7702 {
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to,
            value,
            access_list,
            authorization_list,
            input,
        } = self;
        by_props!(
            @size
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to,
            value,
            access_list,
            authorization_list,
            input,
        )
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        let TxEip7702 {
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to,
            value,
            access_list,
            authorization_list,
            input,
        } = self;
        by_props!(
            @encode buf;
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to,
            value,
            access_list,
            authorization_list,
            input,
        )
    }

    fn decode_value(mut data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let mut tx = TxEip7702::default();
        let TxEip7702 {
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to,
            value,
            access_list,
            authorization_list,
            input,
        } = &mut tx;
        by_props!(
            @decode data;
            chain_id,
            nonce,
            gas_limit,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            to,
            value,
            access_list,
            authorization_list,
            input,
        );
        Ok(tx)
    }
}

impl<T, Sig> ValSer for Signed<T, Sig>
where
    T: ValSer,
    Sig: ValSer,
{
    fn encoded_size(&self) -> usize {
        self.signature().encoded_size() + self.tx().encoded_size()
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        self.signature().encode_value_to(buf);
        self.tx().encode_value_to(buf);
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let mut data = data;

        let signature = Sig::decode_value(data)?;
        data = &data[signature.encoded_size()..];

        let tx = T::decode_value(data)?;

        Ok(Signed::new_unhashed(tx, signature))
    }
}

impl ValSer for TransactionSigned {
    fn encoded_size(&self) -> usize {
        self.tx_type().encoded_size()
            + match self {
                EthereumTxEnvelope::Legacy(signed) => signed.encoded_size(),
                EthereumTxEnvelope::Eip2930(signed) => signed.encoded_size(),
                EthereumTxEnvelope::Eip1559(signed) => signed.encoded_size(),
                EthereumTxEnvelope::Eip4844(signed) => signed.encoded_size(),
                EthereumTxEnvelope::Eip7702(signed) => signed.encoded_size(),
            }
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        self.tx_type().encode_value_to(buf);
        match self {
            EthereumTxEnvelope::Legacy(signed) => {
                signed.encode_value_to(buf);
            }
            EthereumTxEnvelope::Eip2930(signed) => {
                signed.encode_value_to(buf);
            }
            EthereumTxEnvelope::Eip1559(signed) => {
                signed.encode_value_to(buf);
            }
            EthereumTxEnvelope::Eip4844(signed) => {
                signed.encode_value_to(buf);
            }
            EthereumTxEnvelope::Eip7702(signed) => {
                signed.encode_value_to(buf);
            }
        }
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let ty = TxType::decode_value(data)?;
        let data = &data[ty.encoded_size()..];
        match ty {
            TxType::Legacy => ValSer::decode_value(data).map(EthereumTxEnvelope::Legacy),
            TxType::Eip2930 => ValSer::decode_value(data).map(EthereumTxEnvelope::Eip2930),
            TxType::Eip1559 => ValSer::decode_value(data).map(EthereumTxEnvelope::Eip1559),
            TxType::Eip4844 => ValSer::decode_value(data).map(EthereumTxEnvelope::Eip4844),
            TxType::Eip7702 => ValSer::decode_value(data).map(EthereumTxEnvelope::Eip7702),
        }
    }
}


