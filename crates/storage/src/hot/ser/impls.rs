use crate::hot::ser::{DeserError, KeySer, MAX_KEY_SIZE, ValSer};
use alloy::primitives::{Address, B256, Bloom};
use bytes::BufMut;
use reth::primitives::StorageEntry;
use reth_db::models::BlockNumberAddress;

macro_rules! delegate_val_to_key {
    ($ty:ty) => {
        impl ValSer for $ty {
            const FIXED_SIZE: Option<usize> = Some(<Self as KeySer>::SIZE);

            fn encoded_size(&self) -> usize {
                <Self as KeySer>::SIZE
            }

            fn encode_value_to<B>(&self, buf: &mut B)
            where
                B: BufMut + AsMut<[u8]>,
            {
                let mut key_buf = [0u8; MAX_KEY_SIZE];
                let key_bytes = KeySer::encode_key(self, &mut key_buf);
                buf.put_slice(key_bytes);
            }

            fn decode_value(data: &[u8]) -> Result<Self, DeserError>
            where
                Self: Sized,
            {
                KeySer::decode_key(&data)
            }
        }
    };
}

macro_rules! ser_alloy_fixed {
    ($size:expr) => {
        impl KeySer for alloy::primitives::FixedBytes<$size> {
            const SIZE: usize = $size;

            fn encode_key<'a: 'c, 'b: 'c, 'c>(&'a self, _buf: &'b mut [u8; MAX_KEY_SIZE]) -> &'c [u8] {
                self.as_ref()
            }

            fn decode_key(data: &[u8]) -> Result<Self, DeserError>
            where
                Self: Sized,
            {
                if data.len() < $size {
                    return Err(DeserError::InsufficientData {
                        needed: $size,
                        available: data.len(),
                    });
                }
                let mut this = Self::default();
                this.as_mut_slice().copy_from_slice(&data[..$size]);
                Ok(this)
            }
        }

        delegate_val_to_key!(alloy::primitives::FixedBytes<$size>);
    };

    ($($size:expr),* $(,)?) => {
        $(
            ser_alloy_fixed!($size);
        )+
    };
}

macro_rules! ser_be_num {
    ($ty:ty, $size:expr) => {
        impl KeySer for $ty {
            const SIZE: usize = $size;

            fn encode_key<'a: 'c, 'b: 'c, 'c>(&'a self, buf: &'b mut [u8; MAX_KEY_SIZE]) -> &'c [u8] {
                let be_bytes: [u8; $size] = self.to_be_bytes();
                buf[..$size].copy_from_slice(&be_bytes);
                &buf[..$size]
            }

            fn decode_key(data: &[u8]) -> Result<Self, DeserError>
            where
                Self: Sized,
            {
                if data.len() < $size {
                    return Err(DeserError::InsufficientData {
                        needed: $size,
                        available: data.len(),
                    });
                }
                let bytes: [u8; $size] = data[..$size].try_into().map_err(DeserError::from)?;
                Ok(<$ty>::from_be_bytes(bytes))
            }
        }

        delegate_val_to_key!($ty);
    };
    ($($ty:ty, $size:expr);* $(;)?) => {
        $(
            ser_be_num!($ty, $size);
        )+
    };
}

ser_be_num!(
    u8, 1;
    i8, 1;
    u16, 2;
    u32, 4;
    u64, 8;
    u128, 16;
    i16, 2;
    i32, 4;
    i64, 8;
    i128, 16;
    usize, std::mem::size_of::<usize>();
    isize, std::mem::size_of::<isize>();
    alloy::primitives::U160, 20;
    alloy::primitives::U256, 32;
);

// NB: 52 is for AccountStorageKey which is (20 + 32).
// 65 is for Signature, which is (1 + 32 + 32).
ser_alloy_fixed!(8, 16, 20, 32, 52, 65, 256);
delegate_val_to_key!(alloy::primitives::Address);

impl KeySer for Address {
    const SIZE: usize = 20;

    fn encode_key<'a: 'c, 'b: 'c, 'c>(&'a self, _buf: &'b mut [u8; MAX_KEY_SIZE]) -> &'c [u8] {
        self.as_ref()
    }

    fn decode_key(data: &[u8]) -> Result<Self, DeserError> {
        if data.len() < Self::SIZE {
            return Err(DeserError::InsufficientData { needed: Self::SIZE, available: data.len() });
        }
        let mut addr = Self::default();
        addr.copy_from_slice(&data[..Self::SIZE]);
        Ok(addr)
    }
}

impl ValSer for Bloom {
    const FIXED_SIZE: Option<usize> = Some(256);

    fn encoded_size(&self) -> usize {
        self.as_slice().len()
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: BufMut + AsMut<[u8]>,
    {
        buf.put_slice(self.as_ref());
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        if data.len() < 256 {
            return Err(DeserError::InsufficientData { needed: 256, available: data.len() });
        }
        let mut bloom = Self::default();
        bloom.as_mut_slice().copy_from_slice(&data[..256]);
        Ok(bloom)
    }
}

impl ValSer for bytes::Bytes {
    fn encoded_size(&self) -> usize {
        2 + self.len()
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: BufMut + AsMut<[u8]>,
    {
        buf.put_u16(self.len() as u16);
        buf.put_slice(self);
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        if data.len() < 2 {
            return Err(DeserError::InsufficientData { needed: 2, available: data.len() });
        }
        let len = u16::from_be_bytes(data[0..2].try_into().unwrap()) as usize;
        Ok(bytes::Bytes::copy_from_slice(&data[2..2 + len]))
    }
}

impl ValSer for alloy::primitives::Bytes {
    fn encoded_size(&self) -> usize {
        self.0.encoded_size()
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: BufMut + AsMut<[u8]>,
    {
        self.0.encode_value_to(buf);
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        bytes::Bytes::decode_value(data).map(alloy::primitives::Bytes)
    }
}

impl<T> ValSer for Option<T>
where
    T: ValSer,
{
    fn encoded_size(&self) -> usize {
        1 + match self {
            Some(inner) => inner.encoded_size(),
            None => 0,
        }
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        // Simple presence flag
        if let Some(inner) = self {
            buf.put_u8(1);
            inner.encode_value_to(buf);
        } else {
            buf.put_u8(0);
        }
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let flag = data
            .first()
            .ok_or(DeserError::InsufficientData { needed: 1, available: data.len() })?;
        match flag {
            0 => Ok(None),
            1 => Ok(Some(T::decode_value(&data[1..])?)),
            _ => Err(DeserError::String(format!("Invalid Option flag: {}", flag))),
        }
    }
}

impl<T> ValSer for Vec<T>
where
    T: ValSer,
{
    fn encoded_size(&self) -> usize {
        // 2 bytes for length prefix
        2 + self.iter().map(|item| item.encoded_size()).sum::<usize>()
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        buf.put_u16(self.len() as u16);
        self.iter().for_each(|item| item.encode_value_to(buf));
    }

    fn decode_value(mut data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        if data.len() < 2 {
            return Err(DeserError::InsufficientData { needed: 2, available: data.len() });
        }

        let items = u16::from_be_bytes(data[0..2].try_into().unwrap()) as usize;
        data = &data[2..];

        // Preallocate the vector
        let mut vec = Vec::with_capacity(items);

        vec.spare_capacity_mut().iter_mut().try_for_each(|slot| {
            // Decode the item and advance the data slice
            let item = slot.write(T::decode_value(data)?);
            // Advance data slice by the size of the decoded item
            data = &data[item.encoded_size()..];
            Ok::<_, DeserError>(())
        })?;

        // SAFETY:
        // If we did not shortcut return, we have initialized all `items`
        // elements.
        unsafe {
            vec.set_len(items);
        }
        Ok(vec)
    }
}

impl KeySer for BlockNumberAddress {
    const SIZE: usize = u64::SIZE + Address::SIZE;

    fn encode_key<'a: 'c, 'b: 'c, 'c>(&'a self, buf: &'b mut [u8; MAX_KEY_SIZE]) -> &'c [u8] {
        buf[0..8].copy_from_slice(&self.0.0.to_be_bytes());
        buf[8..28].copy_from_slice(self.0.1.as_ref());
        &buf[..Self::SIZE]
    }

    fn decode_key(data: &[u8]) -> Result<Self, DeserError> {
        if data.len() < Self::SIZE {
            return Err(DeserError::InsufficientData { needed: Self::SIZE, available: data.len() });
        }
        let number = u64::from_be_bytes(data[0..8].try_into().unwrap());
        let address = Address::from_slice(&data[8..28]);
        Ok(BlockNumberAddress((number, address)))
    }
}

impl ValSer for StorageEntry {
    const FIXED_SIZE: Option<usize> = Some(64);

    fn encoded_size(&self) -> usize {
        self.key.encoded_size() + self.value.encoded_size()
    }

    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: bytes::BufMut + AsMut<[u8]>,
    {
        self.key.encode_value_to(buf);
        self.value.encode_value_to(buf);
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        let key: B256 = ValSer::decode_value(data)?;
        let value = ValSer::decode_value(&data[key.encoded_size()..])?;
        Ok(StorageEntry { key, value })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{Address, Bloom, Bytes as AlloBytes, FixedBytes, U160, U256};
    use bytes::Bytes;

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
    fn test_integer_roundtrips() {
        // Test boundary values for all integer types
        test_roundtrip(&0u8);
        test_roundtrip(&255u8);
        test_roundtrip(&127i8);
        test_roundtrip(&-128i8);

        test_roundtrip(&0u16);
        test_roundtrip(&65535u16);
        test_roundtrip(&32767i16);
        test_roundtrip(&-32768i16);

        test_roundtrip(&0u32);
        test_roundtrip(&4294967295u32);
        test_roundtrip(&2147483647i32);
        test_roundtrip(&-2147483648i32);

        test_roundtrip(&0u64);
        test_roundtrip(&18446744073709551615u64);
        test_roundtrip(&9223372036854775807i64);
        test_roundtrip(&-9223372036854775808i64);

        test_roundtrip(&0u128);
        test_roundtrip(&340282366920938463463374607431768211455u128);
        test_roundtrip(&170141183460469231731687303715884105727i128);
        test_roundtrip(&-170141183460469231731687303715884105728i128);

        test_roundtrip(&0usize);
        test_roundtrip(&usize::MAX);
        test_roundtrip(&0isize);
        test_roundtrip(&isize::MAX);
        test_roundtrip(&isize::MIN);
    }

    #[test]
    fn test_u256_roundtrips() {
        test_roundtrip(&U256::ZERO);
        test_roundtrip(&U256::from(1u64));
        test_roundtrip(&U256::from(255u64));
        test_roundtrip(&U256::from(65535u64));
        test_roundtrip(&U256::from(u64::MAX));
        test_roundtrip(&U256::MAX);
    }

    #[test]
    fn test_u160_roundtrips() {
        test_roundtrip(&U160::ZERO);
        test_roundtrip(&U160::from(1u64));
        test_roundtrip(&U160::from(u64::MAX));
        // Create a maxed U160 (20 bytes = 160 bits)
        let max_u160 = U160::from_be_bytes([0xFF; 20]);
        test_roundtrip(&max_u160);
    }

    #[test]
    fn test_address_roundtrips() {
        test_roundtrip(&Address::ZERO);
        // Create a test address with known pattern
        let test_addr = Address::from([
            0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB,
            0xCD, 0xEF, 0x01, 0x23, 0x45, 0x67,
        ]);
        test_roundtrip(&test_addr);
    }

    #[test]
    fn test_fixedbytes_roundtrips() {
        // Test various FixedBytes sizes
        test_roundtrip(&FixedBytes::<8>::ZERO);
        test_roundtrip(&FixedBytes::<16>::ZERO);
        test_roundtrip(&FixedBytes::<20>::ZERO);
        test_roundtrip(&FixedBytes::<32>::ZERO);
        test_roundtrip(&FixedBytes::<52>::ZERO);
        test_roundtrip(&FixedBytes::<65>::ZERO);
        test_roundtrip(&FixedBytes::<256>::ZERO);

        // Test with non-zero patterns
        let pattern_32 = FixedBytes::<32>::from([
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
            0x0F, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C,
            0x1D, 0x1E, 0x1F, 0x20,
        ]);
        test_roundtrip(&pattern_32);
    }

    #[test]
    fn test_bloom_roundtrips() {
        test_roundtrip(&Bloom::ZERO);
        // Create a bloom with some bits set
        let mut bloom_data = [0u8; 256];
        bloom_data[0] = 0xFF;
        bloom_data[127] = 0xAA;
        bloom_data[255] = 0x55;
        let bloom = Bloom::from(bloom_data);
        test_roundtrip(&bloom);
    }

    #[test]
    fn test_bytes_roundtrips() {
        // Test bytes::Bytes
        test_roundtrip(&Bytes::new());
        test_roundtrip(&Bytes::from_static(b"hello world"));
        test_roundtrip(&Bytes::from(vec![0x00, 0x01, 0x02, 0x03, 0xFF, 0xFE, 0xFD]));

        // Test alloy::primitives::Bytes
        test_roundtrip(&AlloBytes::new());
        test_roundtrip(&AlloBytes::from_static(b"hello alloy"));
        test_roundtrip(&AlloBytes::copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]));
    }

    #[test]
    fn test_option_roundtrips() {
        // None variants
        test_roundtrip(&None::<u32>);
        test_roundtrip(&None::<Address>);
        test_roundtrip(&None::<AlloBytes>);

        // Some variants
        test_roundtrip(&Some(42u32));
        test_roundtrip(&Some(u64::MAX));
        test_roundtrip(&Some(Address::ZERO));
        test_roundtrip(&Some(U256::from(12345u64)));
        test_roundtrip(&Some(AlloBytes::from_static(b"test")));

        // Nested options
        test_roundtrip(&Some(Some(123u32)));
        test_roundtrip(&Some(None::<u32>));
        test_roundtrip(&None::<Option<u32>>);
    }

    #[test]
    fn test_vec_roundtrips() {
        // Empty vectors
        test_roundtrip(&Vec::<u32>::new());
        test_roundtrip(&Vec::<Address>::new());

        // Single element vectors
        test_roundtrip(&vec![42u32]);
        test_roundtrip(&vec![Address::ZERO]);

        // Multiple element vectors
        test_roundtrip(&vec![1u32, 2, 3, 4, 5]);
        test_roundtrip(&vec![U256::ZERO, U256::from(1u64), U256::MAX]);

        // Vector of bytes
        test_roundtrip(&vec![
            AlloBytes::from_static(b"first"),
            AlloBytes::from_static(b"second"),
            AlloBytes::new(),
            AlloBytes::from_static(b"last"),
        ]);

        // Nested vectors
        test_roundtrip(&vec![vec![1u32, 2, 3], vec![], vec![4, 5]]);

        // Vector of options
        test_roundtrip(&vec![Some(1u32), None, Some(2u32), Some(3u32), None]);
    }

    #[test]
    fn test_complex_combinations() {
        // Option of Vec
        test_roundtrip(&Some(vec![1u32, 2, 3, 4]));
        test_roundtrip(&None::<Vec<u32>>);

        // Vec of Options
        test_roundtrip(&vec![Some(Address::ZERO), None, Some(Address::ZERO)]);

        // Option of Option
        test_roundtrip(&Some(Some(42u32)));
        test_roundtrip(&Some(None::<u32>));

        // Complex nested structure
        let complex = vec![
            Some(vec![AlloBytes::from_static(b"hello")]),
            None,
            Some(vec![
                AlloBytes::from_static(b"world"),
                AlloBytes::new(),
                AlloBytes::from_static(b"!"),
            ]),
        ];
        test_roundtrip(&complex);
    }

    #[test]
    fn test_edge_cases() {
        // Maximum values that should still work
        test_roundtrip(&vec![0u8; 65535]); // Max length for Vec

        // Large FixedBytes
        let large_fixed = FixedBytes::<256>::from([0xFF; 256]);
        test_roundtrip(&large_fixed);

        // Very large U256
        let large_u256 = U256::from_str_radix(
            "115792089237316195423570985008687907853269984665640564039457584007913129639935",
            10,
        )
        .unwrap();
        test_roundtrip(&large_u256);
    }
}
