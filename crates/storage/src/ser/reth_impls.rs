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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{
        Address, B256, Bloom, Bytes as AlloBytes, Signature, TxKind, U256, keccak256,
    };
    use alloy::{
        consensus::{TxEip1559, TxEip2930, TxEip4844, TxEip7702, TxLegacy},
        eips::{
            eip2930::{AccessList, AccessListItem},
            eip7702::{Authorization, SignedAuthorization},
        },
    };
    use reth::primitives::{Account, Header, Log, LogData, TxType};
    use reth::revm::bytecode::JumpTable;
    use reth_db_api::{BlockNumberList, models::StoredBlockBodyIndices};

    /// Generic roundtrip test for any ValSer type
    #[track_caller]
    fn test_roundtrip<T>(original: &T)
    where
        T: ValSer + PartialEq + std::fmt::Debug,
    {
        // Encode
        let mut buf = bytes::BytesMut::new();
        original.encode_value_to(&mut buf);
        let encoded = buf.freeze();

        // Assert that the encoded size matches
        assert_eq!(
            original.encoded_size(),
            encoded.len(),
            "Encoded size mismatch: expected {}, got {}",
            original.encoded_size(),
            encoded.len()
        );

        // Decode
        let decoded = T::decode_value(&encoded).expect("Failed to decode value");

        // Assert equality
        assert_eq!(*original, decoded, "Roundtrip failed");
    }

    #[test]
    fn test_blocknumberlist_roundtrips() {
        // Empty list
        test_roundtrip(&BlockNumberList::empty());

        // Single item
        let mut single = BlockNumberList::empty();
        single.push(42u64).unwrap();
        test_roundtrip(&single);

        // Multiple items
        let mut multiple = BlockNumberList::empty();
        for i in [0, 1, 255, 256, 65535, 65536, u64::MAX] {
            multiple.push(i).unwrap();
        }
        test_roundtrip(&multiple);
    }

    #[test]
    fn test_account_roundtrips() {
        // Default account
        test_roundtrip(&Account::default());

        // Account with values
        let account = Account {
            nonce: 42,
            balance: U256::from(123456789u64),
            bytecode_hash: Some(keccak256(b"hello world")),
        };
        test_roundtrip(&account);

        // Account with max values
        let max_account = Account {
            nonce: u64::MAX,
            balance: U256::MAX,
            bytecode_hash: Some(B256::from([0xFF; 32])),
        };
        test_roundtrip(&max_account);
    }

    #[test]
    fn test_header_roundtrips() {
        // Default header
        test_roundtrip(&Header::default());

        // Header with some values
        let header = Header {
            number: 12345,
            gas_limit: 8000000,
            timestamp: 1234567890,
            difficulty: U256::from(1000000u64),
            ..Default::default()
        };
        test_roundtrip(&header);
    }

    #[test]
    fn test_logdata_roundtrips() {
        // Empty log data
        test_roundtrip(&LogData::new_unchecked(vec![], AlloBytes::new()));

        // Log data with one topic
        test_roundtrip(&LogData::new_unchecked(
            vec![B256::from([1; 32])],
            AlloBytes::from_static(b"hello"),
        ));

        // Log data with multiple topics
        test_roundtrip(&LogData::new_unchecked(
            vec![
                B256::from([1; 32]),
                B256::from([2; 32]),
                B256::from([3; 32]),
                B256::from([4; 32]),
            ],
            AlloBytes::from_static(b"world"),
        ));
    }

    #[test]
    fn test_log_roundtrips() {
        let log_data = LogData::new_unchecked(
            vec![B256::from([1; 32]), B256::from([2; 32])],
            AlloBytes::from_static(b"test log data"),
        );
        let log = Log { address: Address::from([0x42; 20]), data: log_data };
        test_roundtrip(&log);
    }

    #[test]
    fn test_txtype_roundtrips() {
        test_roundtrip(&TxType::Legacy);
        test_roundtrip(&TxType::Eip2930);
        test_roundtrip(&TxType::Eip1559);
        test_roundtrip(&TxType::Eip4844);
        test_roundtrip(&TxType::Eip7702);
    }

    #[test]
    fn test_stored_block_body_indices_roundtrips() {
        test_roundtrip(&StoredBlockBodyIndices { first_tx_num: 0, tx_count: 0 });

        test_roundtrip(&StoredBlockBodyIndices { first_tx_num: 12345, tx_count: 67890 });

        test_roundtrip(&StoredBlockBodyIndices { first_tx_num: u64::MAX, tx_count: u64::MAX });
    }

    #[test]
    fn test_signature_roundtrips() {
        test_roundtrip(&Signature::test_signature());

        // Zero signature
        let zero_sig = Signature::new(U256::ZERO, U256::ZERO, false);
        test_roundtrip(&zero_sig);

        // Max signature
        let max_sig = Signature::new(U256::MAX, U256::MAX, true);
        test_roundtrip(&max_sig);
    }

    #[test]
    fn test_txkind_roundtrips() {
        test_roundtrip(&TxKind::Create);
        test_roundtrip(&TxKind::Call(Address::ZERO));
        test_roundtrip(&TxKind::Call(Address::from([0xFF; 20])));
    }

    #[test]
    fn test_accesslist_roundtrips() {
        // Empty access list
        test_roundtrip(&AccessList::default());

        // Access list with one item
        let item = AccessListItem {
            address: Address::from([0x12; 20]),
            storage_keys: vec![B256::from([0x34; 32])],
        };
        test_roundtrip(&AccessList(vec![item]));

        // Access list with multiple items and keys
        let items = vec![
            AccessListItem {
                address: Address::repeat_byte(11),
                storage_keys: vec![B256::from([0x22; 32]), B256::from([0x33; 32])],
            },
            AccessListItem { address: Address::from([0x44; 20]), storage_keys: vec![] },
            AccessListItem {
                address: Address::from([0x55; 20]),
                storage_keys: vec![B256::from([0x66; 32])],
            },
        ];
        test_roundtrip(&AccessList(items));
    }

    #[test]
    fn test_authorization_roundtrips() {
        test_roundtrip(&Authorization {
            chain_id: U256::from(1u64),
            address: Address::repeat_byte(11),
            nonce: 0,
        });

        test_roundtrip(&Authorization {
            chain_id: U256::MAX,
            address: Address::from([0xFF; 20]),
            nonce: u64::MAX,
        });
    }

    #[test]
    fn test_signed_authorization_roundtrips() {
        let auth = Authorization {
            chain_id: U256::from(1u64),
            address: Address::repeat_byte(11),
            nonce: 42,
        };
        let signed_auth =
            SignedAuthorization::new_unchecked(auth, 1, U256::from(12345u64), U256::from(67890u64));
        test_roundtrip(&signed_auth);
    }

    #[test]
    fn test_tx_legacy_roundtrips() {
        test_roundtrip(&TxLegacy::default());

        let tx = TxLegacy {
            chain_id: Some(1),
            nonce: 42,
            gas_price: 20_000_000_000,
            gas_limit: 21000u64,
            to: TxKind::Call(Address::repeat_byte(11)),
            value: U256::from(1000000000000000000u64), // 1 ETH in wei
            input: AlloBytes::from_static(b"hello world"),
        };
        test_roundtrip(&tx);
    }

    #[test]
    fn test_tx_eip2930_roundtrips() {
        test_roundtrip(&TxEip2930::default());

        let access_list = AccessList(vec![AccessListItem {
            address: Address::from([0x22; 20]),
            storage_keys: vec![B256::from([0x33; 32])],
        }]);

        let tx = TxEip2930 {
            chain_id: 1,
            nonce: 42,
            gas_price: 20_000_000_000,
            gas_limit: 21000u64,
            to: TxKind::Call(Address::repeat_byte(11)),
            value: U256::from(1000000000000000000u64),
            input: AlloBytes::from_static(b"eip2930 tx"),
            access_list,
        };
        test_roundtrip(&tx);
    }

    #[test]
    fn test_tx_eip1559_roundtrips() {
        test_roundtrip(&TxEip1559::default());

        let tx = TxEip1559 {
            chain_id: 1,
            nonce: 42,
            gas_limit: 21000u64,
            max_fee_per_gas: 30_000_000_000,
            max_priority_fee_per_gas: 2_000_000_000,
            to: TxKind::Call(Address::repeat_byte(11)),
            value: U256::from(1000000000000000000u64),
            input: AlloBytes::from_static(b"eip1559 tx"),
            access_list: AccessList::default(),
        };
        test_roundtrip(&tx);
    }

    #[test]
    fn test_tx_eip4844_roundtrips() {
        test_roundtrip(&TxEip4844::default());

        let tx = TxEip4844 {
            chain_id: 1,
            nonce: 42,
            gas_limit: 21000u64,
            max_fee_per_gas: 30_000_000_000,
            max_priority_fee_per_gas: 2_000_000_000,
            to: Address::repeat_byte(11),
            value: U256::from(1000000000000000000u64),
            input: AlloBytes::from_static(b"eip4844 tx"),
            access_list: AccessList::default(),
            blob_versioned_hashes: vec![B256::from([0x44; 32])],
            max_fee_per_blob_gas: 1_000_000,
        };
        test_roundtrip(&tx);
    }

    #[test]
    fn test_tx_eip7702_roundtrips() {
        test_roundtrip(&TxEip7702::default());

        let auth = SignedAuthorization::new_unchecked(
            Authorization {
                chain_id: U256::from(1u64),
                address: Address::from([0x77; 20]),
                nonce: 0,
            },
            1,
            U256::from(12345u64),
            U256::from(67890u64),
        );

        let tx = TxEip7702 {
            chain_id: 1,
            nonce: 42,
            gas_limit: 21000u64,
            max_fee_per_gas: 30_000_000_000,
            max_priority_fee_per_gas: 2_000_000_000,
            to: Address::repeat_byte(11),
            value: U256::from(1000000000000000000u64),
            input: AlloBytes::from_static(b"eip7702 tx"),
            access_list: AccessList::default(),
            authorization_list: vec![auth],
        };
        test_roundtrip(&tx);
    }

    #[test]
    fn test_jump_table_roundtrips() {
        // Empty jump table
        test_roundtrip(&JumpTable::default());

        // Jump table with some jumps
        let jump_table = JumpTable::from_slice(&[0b10101010, 0b01010101], 16);
        test_roundtrip(&jump_table);
    }

    #[test]
    fn test_complex_combinations() {
        // Test a complex Header with all fields populated
        let header = Header {
            number: 12345,
            gas_limit: 8000000,
            timestamp: 1234567890,
            difficulty: U256::from(1000000u64),
            parent_hash: keccak256(b"parent"),
            ommers_hash: keccak256(b"ommers"),
            beneficiary: Address::from([0xBE; 20]),
            state_root: keccak256(b"state"),
            transactions_root: keccak256(b"txs"),
            receipts_root: keccak256(b"receipts"),
            logs_bloom: Bloom::default(),
            gas_used: 7999999,
            mix_hash: keccak256(b"mix"),
            nonce: [0x42; 8].into(),
            extra_data: AlloBytes::from_static(b"extra data"),
            base_fee_per_gas: Some(1000000000),
            withdrawals_root: Some(keccak256(b"withdrawals_root")),
            blob_gas_used: Some(500000),
            excess_blob_gas: Some(10000),
            parent_beacon_block_root: Some(keccak256(b"parent_beacon_block_root")),
            requests_hash: Some(keccak256(b"requests_hash")),
        };
        test_roundtrip(&header);

        // Test a complex EIP-1559 transaction
        let access_list = AccessList(vec![
            AccessListItem {
                address: Address::repeat_byte(11),
                storage_keys: vec![B256::from([0x22; 32]), B256::from([0x33; 32])],
            },
            AccessListItem { address: Address::from([0x44; 20]), storage_keys: vec![] },
        ]);

        let complex_tx = TxEip1559 {
            chain_id: 1,
            nonce: 123456,
            gas_limit: 500000u64,
            max_fee_per_gas: 50_000_000_000,
            max_priority_fee_per_gas: 3_000_000_000,
            to: TxKind::Create,
            value: U256::ZERO,
            input: AlloBytes::copy_from_slice(&[0xFF; 1000]), // Large input
            access_list,
        };
        test_roundtrip(&complex_tx);
    }

    #[test]
    fn test_edge_cases() {
        // Very large access list
        let large_storage_keys: Vec<B256> =
            (0..1000).map(|i| B256::from(U256::from(i).to_be_bytes::<32>())).collect();
        let large_access_list = AccessList(vec![AccessListItem {
            address: Address::from([0xAA; 20]),
            storage_keys: large_storage_keys,
        }]);
        test_roundtrip(&large_access_list);

        // Transaction with maximum values
        let max_tx = TxEip1559 {
            chain_id: u64::MAX,
            nonce: u64::MAX,
            gas_limit: u64::MAX,
            max_fee_per_gas: u128::MAX,
            max_priority_fee_per_gas: u128::MAX,
            to: TxKind::Call(Address::repeat_byte(0xFF)),
            value: U256::MAX,
            input: AlloBytes::copy_from_slice(&[0xFF; 10000]), // Very large input
            access_list: AccessList::default(),
        };
        test_roundtrip(&max_tx);

        // BlockNumberList with many numbers
        let mut large_list = BlockNumberList::empty();
        for i in 0..10000u64 {
            large_list.push(i).unwrap();
        }
        test_roundtrip(&large_list);
    }
}
