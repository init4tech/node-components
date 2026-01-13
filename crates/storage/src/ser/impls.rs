use crate::ser::{DeserError, KeySer, MAX_KEY_SIZE, ValSer};
use bytes::BufMut;

macro_rules! delegate_val_to_key {
    ($ty:ty) => {
        impl ValSer for $ty {
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

// NB: 52 is for AccountStorageKey which is (20 + 32)
ser_alloy_fixed!(16, 20, 32, 52);

impl KeySer for alloy::primitives::Address {
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

impl ValSer for bytes::Bytes {
    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: BufMut + AsMut<[u8]>,
    {
        buf.put_slice(self);
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        Ok(bytes::Bytes::copy_from_slice(data))
    }
}

impl ValSer for alloy::primitives::Bytes {
    fn encode_value_to<B>(&self, buf: &mut B)
    where
        B: BufMut + AsMut<[u8]>,
    {
        buf.put_slice(self);
    }

    fn decode_value(data: &[u8]) -> Result<Self, DeserError>
    where
        Self: Sized,
    {
        Ok(alloy::primitives::Bytes::copy_from_slice(data))
    }
}
